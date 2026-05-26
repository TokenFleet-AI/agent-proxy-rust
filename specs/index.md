# Specs Index

| # | Document | Phase 1 | Phase 2 | Status | Description |
|---|----------|---------|---------|--------|-------------|
| 0001 | [architecture](0001-architecture.md) | local MVP | cloud-ready | draft | Overall architecture, crate map, request flow, phased strategy |
| 0002 | [middleware-engine](0002-middleware-engine.md) | core trait | cloud extensions | draft | ProxyMiddleware trait, server engine, streaming path |
| 0003 | [channel-model](0003-channel-model.md) | simple selection | health checks, standby | draft | Channel data model, selection strategy, default channels |
| 0004 | [cost-tracking](0004-cost-tracking.md) | single table | WAL, aggregation, retention | draft | CostRecord schema, SQLite, aggregation queries |
| 0005 | [provider-registry](0005-builtin-channels.md) | 4 core builtin | community fetch, 10+ providers | draft | Provider registry: models, pricing, JSON format |
| 0006 | [compress-crate](0006-compress-crate.md) | full | — | draft | Tokenless schema compression middleware |
| 0007 | [bridge-crate](0007-bridge-crate.md) | 2 directions | full 6-direction matrix | draft | Protocol translation bridge (llm-bridge-core) |
| 0008 | [security](0008-security.md) | OS chmod + secrecy | AES encryption | draft | API key protection, proxy auth, TLS, input validation |
| 0009 | [deployment](0009-deployment.md) | cargo install + serve | Docker, systemd, Prometheus | draft | Deployment modes, logging, metrics |
| 0010 | [configuration](0010-configuration.md) | clap CLI flags | config crate, YAML, env merge | draft | CLI flags, env vars, config file merge rules |
| 0011 | [error-handling](0011-error-handling.md) | full | — | draft | ProxyError type hierarchy, error codes, HTTP status |
| 0012 | [testing-strategy](0012-testing-strategy.md) | full | — | draft | Unit, integration, E2E, property, fuzz tests |
| 0013 | [rate-limiting](0013-rate-limiting.md) | — | cloud extension | draft | Client/project/channel token buckets, fairness |

## Phase Strategy

- **Phase 1 (local MVP)**: Runs on developer's laptop. Single user. `cargo install + serve`. Simple SQLite, CLI flags only, OS file permissions.
- **Phase 2 (cloud-ready)**: Extensions for multi-user deployments. Feature-gated or separate crates. Docker, config layers, AES encryption, health probes, Prometheus.

Design principle: Phase 1 is simple but **not dead-end**. Trait-based middleware allows Phase 2 features to plug in without rewriting core logic.

## Status Legend

- `draft` — design phase
- `in-progress` — implementation started
- `done` — implemented and verified
