//! Minimal interactive / single-shot chat CLI for llmrust.
//!
//! Reads credentials and model from environment variables (so secrets are
//! never baked into the binary or shell history):
//!
//! ```bash
//! export OPENAI_API_KEY="sk-..."
//! export OPENAI_BASE_URL="https://api.openai.com/v1"   # or any OpenAI-compatible endpoint
//! export OPENAI_MODEL="gpt-4o-mini"
//! ```
//!
//! Usage:
//!
//! ```bash
//! # Single-shot non-streaming (prints reply, exits)
//! cargo run --example chat -- "Hello in one sentence."
//!
//! # Single-shot streaming (token-by-token)
//! cargo run --example chat -- --stream "Tell me a haiku about Rust."
//!
//! # Interactive multi-turn REPL
//! cargo run --example chat -- --multi
//!
//! # Interactive multi-turn with streaming per reply
//! cargo run --example chat -- --multi --stream
//! ```
//!
//! You can also pre-set a system prompt:
//!
//! ```bash
//! export OPENAI_SYSTEM="You are a helpful assistant who answers in Chinese."
//! ```

use futures::StreamExt;
use llmrust::{ChatRequest, LmrsClient, Message};
use std::io::{self, BufRead, Write};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse CLI args
    let args: Vec<String> = std::env::args().skip(1).collect();
    let stream = args.iter().any(|a| a == "--stream");
    let multi = args.iter().any(|a| a == "--multi");
    let prompt_args: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();

    // Read config from env
    let api_key =
        std::env::var("OPENAI_API_KEY").map_err(|_| "OPENAI_API_KEY env var is required")?;
    if api_key.trim().is_empty() {
        return Err("OPENAI_API_KEY is empty".into());
    }
    let base_url = std::env::var("OPENAI_BASE_URL")
        .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
    let model_name = std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());
    let system_prompt = std::env::var("OPENAI_SYSTEM").ok();

    // Set up the client
    let llm = LmrsClient::new();
    llm.set_openai_compatible(&api_key, &base_url).await;
    let model_string = format!("openai/{model_name}");

    eprintln!("-> connected to {base_url}");
    eprintln!("-> model: {model_name}");
    eprintln!(
        "-> mode: {}",
        match (multi, stream) {
            (true, true) => "interactive + streaming",
            (true, false) => "interactive + non-streaming",
            (false, true) => "single-shot + streaming",
            (false, false) => "single-shot + non-streaming",
        }
    );
    eprintln!();

    if multi {
        run_repl(&llm, &model_string, system_prompt.as_deref(), stream).await
    } else {
        let prompt = if prompt_args.is_empty() {
            eprintln!("Usage: chat [--stream] [--multi] \"<prompt>\"");
            return Ok(());
        } else {
            prompt_args
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        };
        run_single(
            &llm,
            &model_string,
            &prompt,
            system_prompt.as_deref(),
            stream,
        )
        .await
    }
}

async fn run_single(
    llm: &LmrsClient,
    model: &str,
    prompt: &str,
    system: Option<&str>,
    stream: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("[user] {prompt}");
    if stream {
        let mut req = ChatRequest::new("", prompt);
        if let Some(s) = system {
            req = req.with_system(s);
        }
        let mut s = llm.stream_with(model, req).await?;
        eprintln!("[assistant]");
        let mut full = String::new();
        while let Some(chunk) = s.next().await {
            let chunk = chunk?;
            print!("{}", chunk.delta);
            std::io::stdout().flush().ok();
            full.push_str(&chunk.delta);
        }
        println!();
        eprintln!();
        eprintln!("[done] {} chars", full.chars().count());
    } else {
        let mut req = ChatRequest::new("", prompt);
        if let Some(s) = system {
            req = req.with_system(s);
        }
        let resp = llm.chat_with(model, req).await?;
        println!("{}", resp.content);
        if let Some(u) = &resp.usage {
            eprintln!(
                "\n[tokens] prompt={} completion={} total={}",
                u.prompt_tokens, u.completion_tokens, u.total_tokens
            );
        }
    }
    Ok(())
}

async fn run_repl(
    llm: &LmrsClient,
    model: &str,
    system: Option<&str>,
    stream: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let stdin = io::stdin();
    let mut history: Vec<Message> = Vec::new();
    if let Some(s) = system {
        history.push(Message::system(s));
    }

    eprintln!("Type your message and press Enter. Empty line or Ctrl+D to quit.");
    eprintln!();

    loop {
        eprint!("[you] ");
        io::stderr().flush().ok();

        let mut line = String::new();
        let n = stdin.lock().read_line(&mut line)?;
        if n == 0 {
            eprintln!();
            break;
        }
        let prompt = line.trim();
        if prompt.is_empty() {
            eprintln!();
            break;
        }

        history.push(Message::user(prompt));

        eprintln!();
        eprint!("[assistant] ");
        io::stderr().flush().ok();

        if stream {
            let req = ChatRequest {
                model: String::new(),
                messages: history.clone(),
                temperature: None,
                max_tokens: None,
                stream: true,
                top_p: None,
                tools: None,
                tool_choice: None,
                ..Default::default()
            };
            let mut s = match llm.stream_with(model, req).await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("\n[error] {e}");
                    history.pop();
                    continue;
                }
            };
            let mut full = String::new();
            while let Some(chunk) = s.next().await {
                match chunk {
                    Ok(c) => {
                        print!("{}", c.delta);
                        std::io::stdout().flush().ok();
                        full.push_str(&c.delta);
                    }
                    Err(e) => {
                        eprintln!("\n[stream error] {e}");
                        break;
                    }
                }
            }
            println!();
            history.push(Message::assistant(&full));
        } else {
            let req = ChatRequest {
                model: String::new(),
                messages: history.clone(),
                temperature: None,
                max_tokens: None,
                stream: false,
                top_p: None,
                tools: None,
                tool_choice: None,
                ..Default::default()
            };
            match llm.chat_with(model, req).await {
                Ok(resp) => {
                    println!("{}", resp.content);
                    history.push(Message::assistant(&resp.content));
                }
                Err(e) => {
                    eprintln!("\n[error] {e}");
                    history.pop();
                }
            }
        }
        eprintln!();
    }

    Ok(())
}
