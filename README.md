# agent-proxy-rust

A composable middleware proxy for AI agent APIs. Sits between AI coding agents (Claude Code, Codex, Gemini CLI) and upstream API providers.

```
Client (Claude Code / Codex / Gemini CLI)
        │
        ▼
┌──────────────────────────────────────┐
│           agent-proxy-rust            │
│                                      │
│  compress → route → bridge → cost    │
│                                      │
│  • Compress tool definitions         │
│  • Select optimal channel            │
│  • Translate protocols               │
│  • Track per-project cost            │
└──────────────────────────────────────┘
        │
        ▼
Upstream APIs (Anthropic / OpenAI / DeepSeek / ...)
```

## Features

- **Smart Routing** — Subscription-first, metered fallback. Supports Copilot coding plans with automatic failover.
- **Protocol Translation** — Anthropic Messages ↔ OpenAI Chat ↔ OpenAI Responses. Full 6-direction matrix via [llm-bridge-core].
- **Token Compression** — Transparent schema & response compression via [tokenless-schema].
- **Cost Tracking** — Per-project, per-model cost records with compression savings. Local SQLite.
- **Provider Registry** — Builtin support for Anthropic, OpenAI, Google, DeepSeek. Community registry for additional providers.

## Quick Start

```bash
# Install
cargo install agent-proxy

# Start (4 default channels seeded, add your API keys)
agent-proxy serve

# Set API key for a channel
agent-proxy channel set-key anthropic-official --api-key "sk-ant-xxx"

# Add a custom channel
agent-proxy channel add openrouter \
  --url "https://openrouter.ai/api/v1" \
  --protocol anthropic_messages \
  --api-key "sk-or-xxx"
```

Configure your AI coding agent to use `http://127.0.0.1:8787` as the API endpoint.

## Architecture

| Crate | Purpose |
|-------|---------|
| `core` | Middleware trait, axum server, upstream forwarding |
| `model-router` | Channel selection, model name mapping |
| `compress` | Token compression via tokenless-schema |
| `bridge` | Protocol translation via llm-bridge-core |
| `cost` | Per-project cost tracking (SQLite) |

Request flow: `compress → route → bridge → forward → bridge ← route ← compress → cost`

See [specs/](specs/) for detailed design documents.

## Phased Roadmap

| Phase | Scope |
|-------|-------|
| **Phase 1** | Local desktop MVP — single user, simple channel selection, SQLite cost tracking |
| **Phase 2** | Cloud-ready — health probes, Docker, config layers, multi-instance |
| **Extension** | Rate limiting, Credits/CharBased pricing — separate crates |

## Related Projects

- [tokenless-schema](https://github.com/TokenFleet-AI/tokenless) — JSON schema & response compression
- [llm-bridge-core](https://github.com/TokenFleet-AI/llm-bridge-rust/tree/master/crates/core) — Anthropic ↔ OpenAI protocol translation
- [agent-proxy-pricing](https://github.com/TokenFleet-AI/agent-proxy-pricing) — Community provider & model pricing registry (Phase 2)

## License

[Apache-2.0](LICENSE)

Copyright 2025 TokenFleet-AI
