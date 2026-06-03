# Specs Index

| # | Document | Phase 1 | Phase 2 | Status | Description |
|---|----------|---------|---------|--------|-------------|
| 0001 | [architecture](0001-architecture.md) | local MVP | cloud-ready | done | Overall architecture, crate map, request flow, phased strategy |
| 0002 | [middleware-engine](0002-middleware-engine.md) | core trait | cloud extensions | done | ProxyMiddleware trait (6 hooks), axum server, SSE streaming |
| 0003 | [channel-model](0003-channel-model.md) | simple selection + cooldown | health probes, standby | done | Channel model, FlatFee-first selection, 60s cooldown, 13 builtin channels |
| 0004 | [cost-tracking](0004-cost-tracking.md) | full + role tracking | WAL, aggregation, retention | done | CostRecord schema, usage extraction, 3 pricing modes, per-role tracking |
| 0005 | [provider-registry](0005-builtin-channels.md) | 7 vendors, 27 models | community fetch | draft | Provider registry: Anthropic/OAI/DeepSeek/GLM/Qwen/Kimi/MiniMax |
| 0006 | [compress-crate](0006-compress-crate.md) | full with 13 tests | — | done | Tokenless schema + response compression middleware |
| 0007 | [bridge-crate](0007-bridge-crate.md) | full 6-direction + streaming | — | done | Protocol translation bridge wrapping llm-bridge-core |
| 0008 | [security](0008-security.md) | auth + role mapping | AES encryption, TLS | in-progress | API key protection, proxy auth, role mapping, input validation |
| 0009 | [deployment](0009-deployment.md) | cargo install + serve | Docker, systemd, Prometheus | in-progress | CLI binary + config merge done; Docker/systemd pending |
| 0010 | [configuration](0010-configuration.md) | full | — | done | clap CLI + env vars + YAML config, 3-layer merge |
| 0011 | [error-handling](0011-error-handling.md) | full | — | done | ProxyError type hierarchy, error codes, HTTP status mapping |
| 0012 | [testing-strategy](0012-testing-strategy.md) | 151 tests, E2E wiremock | fuzz, CI matrix | in-progress | Unit/integration/E2E/property tests; fuzz pending |
| 0013 | [rate-limiting](0013-rate-limiting.md) | — | cloud extension | draft | Client/project/channel token buckets, fairness |
| 0014 | [storage-abstraction](0014-storage-abstraction.md) | SQLite trait + 46 tests | PostgreSQL impl | done | Pluggable Storage trait, SqliteStorage, 13 channels + 65 mappings seed |
| 0015 | [health-state-machine](0015-health-state-machine.md) | full | — | in-progress | Channel health: Degraded/Cooldown state machine, failure counting |
| 0016 | [admin-api-extension](0016-admin-api-extension.md) | full | — | done | Admin API: channels, providers, models, mappings CRUD |
| 0017 | [stats-reporting](0017-stats-reporting.md) | — | cloud extension | draft | 请求完成后回传消耗/压缩统计到 tokenless |

## Phase Strategy

- **Phase 1 (local MVP)**: Runs on developer's laptop. Single user. `cargo install + serve`. Simple SQLite, CLI flags only, OS file permissions.
- **Phase 2 (cloud-ready)**: Extensions for multi-user deployments. Feature-gated or separate crates. Docker, config layers, AES encryption, health probes, Prometheus.

Design principle: Phase 1 is simple but **not dead-end**. Trait-based middleware allows Phase 2 features to plug in without rewriting core logic.

## Status Legend

- `draft` — design phase
- `in-progress` — implementation started
- `done` — implemented and verified
