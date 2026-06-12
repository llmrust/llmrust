# Examples

Each example is a standalone binary demonstrating a specific feature. Run with:

```bash
cargo run --example <name>
```

Some examples require API keys. Check the source for `std::env::var` calls to see which environment variables are needed.

## Index

| Example | What it demonstrates | Requires API key? |
|---------|---------------------|-------------------|
| `demo` | Register all 7 providers, call each (chat), then a stream demo | Yes (any one provider) |
| `chat` | Interactive REPL or single-prompt CLI with streaming and multi-turn support | Yes (OPENAI_API_KEY) |
| `multiturn` | 3-turn conversation with system prompt and manual history management | Yes (OPENAI_API_KEY) |
| `multimodal` | Send an image + text message (GPT-4o vision) | Yes (OPENAI_API_KEY) |
| `tool_calling` | Full tool-call loop: define tool, receive call, simulate execution, return result | Yes (OPENAI_API_KEY) |
| `retry_e2e` | End-to-end retry with `RetryProvider` and `LmrsClient::with_retry()` | Yes (any one provider) |
| `router` | Multi-deployment routing with Ordered and RoundRobin strategies | Yes (any one provider) |
| `proxy_server` | Start the HTTP proxy server (dual-protocol OpenAI + Anthropic) | Yes (need `--features proxy`) |
| `e2e_openai_compat` | 6-test suite against any OpenAI-compatible endpoint | Yes (E2E_API_KEY, E2E_BASE_URL, E2E_MODEL) |
| `e2e_anthropic_compat` | 4-test suite against any Anthropic-compatible endpoint | Yes (E2E_API_KEY, E2E_BASE_URL, E2E_MODEL) |

## Quick run

### Chat demo (needs one API key)

```bash
export OPENAI_API_KEY="sk-..."
cargo run --example demo
```

### Interactive chat

```bash
export OPENAI_API_KEY="sk-..."
cargo run --example chat -- --multi --stream
```

### Tool calling

```bash
export OPENAI_API_KEY="sk-..."
cargo run --example tool_calling
```

### Proxy server

```bash
export LLMRUST_OPENAI_KEY="sk-..."
# Optional auth
export LLMRUST_PROXY_KEY="some-shared-secret"
cargo run --example proxy_server --features proxy
```

### End-to-end test suites

```bash
# OpenAI-compatible endpoint
E2E_API_KEY=sk-... E2E_BASE_URL=https://api.openai.com/v1 E2E_MODEL=gpt-4o-mini \
  cargo run --example e2e_openai_compat

# Anthropic endpoint
E2E_API_KEY=sk-ant-... E2E_BASE_URL=https://api.anthropic.com/v1 E2E_MODEL=claude-3-5-sonnet-20241022 \
  cargo run --example e2e_anthropic_compat
```

## Adding a new example

1. Create `examples/<your_example>.rs`
2. Add a `[[example]]` entry to `Cargo.toml`
3. Update this `README.md` index table
4. Keep the example self-contained — it should run with `cargo run --example <name>` without extra setup beyond environment variables
