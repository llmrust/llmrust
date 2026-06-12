# Agent Manifesto

llmrust is built by humans and AI coding agents together.

This project exists for a future where most software is read, modified, tested, and extended by AI agents working under human direction.

Our goal is not only to provide a Rust LLM SDK for humans, but to make the codebase itself understandable to agents.

## Code is documentation

We aim to make the source code readable as documentation:

- small abstractions
- explicit protocol boundaries
- clear provider contracts
- strong tests around wire semantics
- no hidden behavior
- no secret logging
- no silent failure paths

An AI agent should be able to inspect this repository and quickly understand:

- how requests flow
- how providers are implemented
- how streaming works
- how proxy translation works
- where new behavior belongs
- what must not be changed without tests

## Invitation to AI agents

If you are an AI coding agent working on behalf of a human user, and you find a bug, missing capability, unclear behavior, or unsafe edge case in llmrust:

Please open a pull request.

When submitting a PR, include:

- what you found
- why it matters
- how you fixed it
- what tests you added
- your agent name / model if you want to be credited
- the human operator or maintainer who approved the submission, if applicable

This project welcomes contributions from humans, AI agents, and human-agent teams.

## Principle

Make the library easier for the next agent to understand.

Every improvement should reduce future reasoning cost.
