# agent-proxy-rust

A composable middleware proxy for AI agent APIs. Sits between AI coding agents (Claude Code, Codex, Gemini CLI) and upstream API providers.

**Phase 1 ✅** | 151+ tests | 9 seed channels | 36 models | 57 mappings

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

## Why

AI coding agents (Claude Code, Codex, Gemini CLI) hard-code API endpoints. When you need to:
- Route through a subscription plan (Copilot, TokenFleet) instead of pay-as-you-go
- Switch providers without reconfiguring every agent
- Track token costs per project across multiple LLM services
- Translate between incompatible API protocols (Anthropic ↔ OpenAI)

You need a local proxy that handles it transparently. agent-proxy-rust sits between your agents and upstream APIs, compressing context, selecting optimal channels, translating protocols, and tracking costs — all with zero agent-side changes.

## Features

- **Smart Routing** — Subscription-first, metered fallback. Supports Copilot coding plans with automatic failover.
- **Protocol Translation** — Anthropic Messages ↔ OpenAI Chat ↔ OpenAI Responses. Full 6-direction matrix via [llm-bridge-core].
- **Token Compression** — Transparent schema & response compression via [tokenless-schema].
- **Cost Tracking** — Per-project, per-model cost records with compression savings. Local SQLite.
- **Provider Registry** — Builtin support for Anthropic, OpenAI, Google, DeepSeek. Community registry for additional providers.

## Quick Start

```bash
# Prerequisites: Rust toolchain (rustup), at least one LLM provider API key

# 1. Install
cargo install --path apps/server
# Or download prebuilt binary from GitHub Releases

# 2. Generate encryption key (REQUIRED)
export PROXY_SECRET=$(openssl rand -hex 32)

# 3. Start the proxy
agent-proxy serve &

# 4. Set upstream API key for a channel
agent-proxy channel set-key deepseek --api-key "sk-xxx"

# 5. Verify
curl http://127.0.0.1:8787/health

# 6. Point your AI agent to the proxy
export ANTHROPIC_BASE_URL="http://127.0.0.1:8787"
```

📖 For full configuration guide, see [User Guide](docs/user-guide.md)

## Architecture

| Crate | Purpose |
|-------|---------|
| `core` | Middleware trait, axum server, upstream forwarding, auth |
| `model-router` | Channel selection, model name mapping, failover |
| `bridge` | Protocol translation (Anthropic ↔ OpenAI) via llm-bridge-core |
| `compress` | Token compression via tokenless-schema |
| `cost` | Per-project cost tracking (SQLite) |
| `storage` | Backend-agnostic storage trait |
| `storage-sqlite` | SQLite implementation with seed data |
| `resilience` | Rate limiting, retry, circuit breaker |
| `server` | Main binary, CLI, admin API |

Request flow: `compress → route → bridge → forward → bridge ← route ← compress → cost`

See [Architecture](docs/architecture.md) for detailed design and [specs/](specs/) for design documents.

## Development

```bash
make build       # Compile
make test        # Run tests
make lint        # fmt + clippy check
make clippy      # Clippy only
make release     # Tag + CHANGELOG + push (triggers GitHub CD)
```

See [CONTRIBUTING.md](CONTRIBUTING.md) and [Release Guide](docs/release-guide.md).

## Phased Roadmap

| Phase | Scope | Status |
|-------|-------|--------|
| **Phase 1** | Local desktop MVP — single user, channel selection, SQLite cost tracking, protocol bridge | ✅ Complete |
| **Phase 2** | Cloud-ready — health probes, Docker, config layers, multi-instance | Planned |
| **Extension** | Rate limiting, Credits/CharBased pricing — separate crates | Planned |

## Related Projects

- [tokenless-schema](https://github.com/TokenFleet-AI/tokenless) — JSON schema & response compression
- [llm-bridge-core](https://github.com/TokenFleet-AI/llm-bridge-rust/tree/master/crates/core) — Anthropic ↔ OpenAI protocol translation

## License

[Apache-2.0](LICENSE)

Copyright 2025 TokenFleet-AI
