# 0014 — Storage Abstraction

> **Phase 1**: `Storage` trait + SQLite implementation. Channel CRUD, cost record insert/query, subscription fees.
> **Phase 2**: PostgreSQL implementation, connection pool config, migration tooling.

## Motivation

当前 0003（Channel Model）和 0004（Cost Tracking）直接假定 SQLite。但代理在设计上需要一个可插拔的存储后端：

- **Phase 1**：SQLite（零配置，开发者体验最优）
- **Phase 2 cloud**：PostgreSQL（多实例共享、连接池、监控）
- **测试**：in-memory mock（每个测试独立数据，无需 `#[serial]`）

在 `ProxyMiddleware` 架构中，CostMiddleware 和 ModelRouterMiddleware 都是 trait 消费者 —— 它们不应该知道底层是 SQLite 还是 PostgreSQL。现在定义 `Storage` trait，避免后面大规模重写中间件。

## Storage Trait

```rust
/// Backend-agnostic storage for channels, cost records, and subscription fees.
#[async_trait]
pub trait Storage: Send + Sync + Debug {
    // ── Channel ─────────────────────────────────────────────

    /// List all enabled channels with their model mappings.
    async fn list_channels(&self) -> Result<Vec<Channel>, StorageError>;

    /// Get a single channel by ID.
    async fn get_channel(&self, id: &str) -> Result<Option<Channel>, StorageError>;

    /// Insert or replace a channel (upsert)。
    async fn upsert_channel(&self, channel: &Channel) -> Result<(), StorageError>;

    /// Toggle a channel enabled/disabled.
    async fn set_channel_enabled(&self, id: &str, enabled: bool) -> Result<(), StorageError>;

    /// Update just the API key for a channel.
    async fn set_channel_api_key(&self, id: &str, key: &SecretString) -> Result<(), StorageError>;

    /// Delete a channel and its model mappings (cascade).
    async fn delete_channel(&self, id: &str) -> Result<(), StorageError>;

    // ── Model Mapping ───────────────────────────────────────

    /// List all model mappings for a channel.
    async fn list_mappings(&self, channel_id: &str) -> Result<Vec<ModelMapping>, StorageError>;

    /// Upsert a single model mapping.
    async fn upsert_mapping(&self, mapping: &ModelMapping) -> Result<(), StorageError>;

    /// Toggle a model mapping enabled/disabled.
    async fn set_mapping_enabled(&self, id: &str, enabled: bool) -> Result<(), StorageError>;

    /// Delete a model mapping.
    async fn delete_mapping(&self, id: &str) -> Result<(), StorageError>;

    // ── Cost Records ────────────────────────────────────────

    /// Record a completed request.
    async fn insert_cost_record(&self, record: &CostRecord) -> Result<(), StorageError>;

    /// Query cost records with optional filters.
    async fn query_cost_records(
        &self,
        filter: CostFilter,
    ) -> Result<Vec<CostRecord>, StorageError>;

    /// Aggregate costs per project × model × month.
    async fn aggregate_costs(
        &self,
        group_by: CostGroupBy,
        range: TimeRange,
    ) -> Result<Vec<CostAggregate>, StorageError>;

    /// Delete records older than N days, returning count of deleted rows.
    async fn prune_cost_records(&self, older_than_days: u32) -> Result<u64, StorageError>;

    // ── Subscription Fees ───────────────────────────────────

    /// Record a monthly subscription fee.
    async fn insert_subscription_fee(&self, fee: &SubscriptionFee) -> Result<(), StorageError>;

    /// Query subscription fees by channel and month.
    async fn query_subscription_fees(
        &self,
        channel: Option<&str>,
        month: Option<&str>,
    ) -> Result<Vec<SubscriptionFee>, StorageError>;

    // ── Lifecycle ───────────────────────────────────────────

    /// Run migrations / schema init. Idempotent — safe to call on every startup.
    async fn migrate(&self) -> Result<(), StorageError>;

    /// Health check — returns true if the backend is reachable.
    async fn health_check(&self) -> Result<bool, StorageError>;

    /// Maximum number of concurrent connections this backend supports.
    /// SQLite returns 1; PostgreSQL returns pool size.
    fn max_connections(&self) -> usize;
}
```

## StorageError

```rust
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("backend error: {0}")]
    Backend(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("duplicate: {0}")]
    Duplicate(String),

    #[error("connection error: {0}")]
    Connection(String),

    #[error("migration error: {0}")]
    Migration(String),
}
```

## Data Types (storage-crate-owned)

```rust
pub struct Channel {
    pub id: String,
    pub name: String,
    pub url: String,
    pub api_key: SecretString,
    pub protocol: String,   // "anthropic_messages" | "openai_chat" | "openai_responses"
    pub is_builtin: bool,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub struct ModelMapping {
    pub id: String,
    pub channel_id: String,
    pub client_name: String,
    pub upstream_name: String,
    pub billing: String,   // "metered" | "flatfee"
    pub pricing_json: String,  // serialized Pricing
    pub weight: u32,
    pub enabled: bool,
}

pub struct CostRecord {
    pub id: i64,
    pub timestamp: DateTime<Utc>,
    pub user_name: String,
    pub project_path: String,
    pub project_name: String,
    pub agent_type: String,
    pub channel_name: String,
    pub channel_kind: String,
    pub model_name: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_read_tokens: u64,
    pub thinking_tokens: u64,
    pub actual_cost: f64,
    pub unit: String,
    pub pre_compress_tokens: u64,
    pub post_compress_tokens: u64,
    pub compression_tokens_saved: u64,
}

pub struct SubscriptionFee {
    pub id: i64,
    pub channel_name: String,
    pub month: String,  // "2026-05"
    pub monthly_price: f64,
    pub currency: String,
}

pub struct CostFilter {
    pub project_path: Option<String>,
    pub model_name: Option<String>,
    pub channel_name: Option<String>,
    pub time_range: Option<TimeRange>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

pub struct TimeRange {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

pub enum CostGroupBy {
    Project,
    Model,
    Channel,
    ProjectModelMonth,
}

pub struct CostAggregate {
    pub group_key: String,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_actual_cost: f64,
    pub total_compression_tokens_saved: u64,
    pub request_count: u64,
}
```

## Crate Map Update

```
crates/
├── core/               Middleware trait + axum server engine
├── storage/        ←   Storage trait + data types (NEW)
├── storage-sqlite/ ←   SQLite implementation (NEW)
├── model-router/       Channel management (depends on storage trait)
├── compress/           Token compression middleware
├── bridge/             Protocol translation middleware
└── cost/               Cost tracking middleware (depends on storage trait)
```

Dependency direction:

```
storage ← storage-sqlite
    ↑
    ├── model-router  (calls list_channels, upsert_channel, ...)
    └── cost          (calls insert_cost_record, query_cost_records, ...)
```

`core` has zero knowledge of storage. Middlewares depend on `Box<dyn Storage>` injected at construction time.

## SQLite Implementation

`storage-sqlite` crate provides `SqliteStorage` implementing `Storage`:

```rust
pub struct SqliteStorage {
    pool: r2d2::Pool<SqliteConnectionManager>,
}

impl SqliteStorage {
    pub fn new(path: &Path) -> Result<Self, StorageError>;
    pub fn new_in_memory() -> Result<Self, StorageError>;  // for testing
}
```

Migration is embedded as `include_str!("migrations/001_init.sql")` and applied on `migrate()`. Uses `PRAGMA user_version` for version tracking.

Key SQLite specifics:
- Single writer (WAL mode allows concurrent readers)
- `max_connections()` returns **1** — the pool size is 1, but WAL lets readers work while one writer is active
- API keys encrypted via AES-256-GCM before write, decrypted after read (see 0008)
- Channel/model_mapping seed on first run (embedded JSON files)

## PostgreSQL Implementation (Phase 2)

`storage-postgres` crate provides `PgStorage`:

```rust
pub struct PgStorage {
    pool: sqlx::PgPool,
}
```

Differences from SQLite:
- `max_connections()` returns pool size (typically 10-20)
- Migrations via embedded SQL files or `sqlx::migrate!()`
- JSONB for `pricing_json` instead of TEXT
- `INSERT ... ON CONFLICT` instead of `INSERT OR REPLACE`
- `RETURNING` clause for insert-and-get-id patterns
- Connection string from config: `AGENT_PROXY_DATABASE_URL`

## Configuration

```rust
struct StorageConfig {
    /// Storage backend: "sqlite" (default) | "postgres" (Phase 2).
    backend: String,

    /// Path for SQLite database file (Phase 1).
    sqlite_path: Option<PathBuf>,

    /// Connection URL for PostgreSQL (Phase 2).
    database_url: Option<String>,

    /// Max connections in the pool (Phase 2).
    pool_size: usize,  // default: 10
}
```

```yaml
# config.yaml
storage:
  backend: sqlite
  sqlite_path: null   # uses data dir default
  # Phase 2:
  # backend: postgres
  # database_url: "postgres://user:pass@localhost/agent_proxy"
  # pool_size: 10
```

## Testing

Each middleware test uses `SqliteStorage::new_in_memory()`, giving every test an isolated database. No `#[serial]` needed — unlike shared-file SQLite.

```rust
#[tokio::test]
async fn test_cost_middleware_writes_record() {
    let storage = Arc::new(SqliteStorage::new_in_memory().unwrap());
    storage.migrate().await.unwrap();

    let middleware = CostMiddleware::new(Arc::clone(&storage));
    // ... test ...
}
```

For unit tests that don't need real DB, use a mock:

```rust
// tests/common/mock_storage.rs
pub struct MockStorage { ... }
impl Storage for MockStorage { ... }
```

## Migration from 0003/0004

Existing specs 0003 and 0004 define SQLite tables directly. With this storage abstraction:

- **0003 §Configuration Source** — the `CREATE TABLE channels/model_mappings` SQL moves to `storage-sqlite/migrations/`. Channel seed logic stays in model-router (uses `upsert_channel`/`upsert_mapping`).
- **0004 §SQLite Schema** — the `CREATE TABLE cost_records/subscription_fees` SQL moves to storage-sqlite migrations. Aggregation queries become methods on the `Storage` trait.
- **0004 §SQLite Configuration** — WAL pragmas move into `SqliteStorage::new()`.

## Related Specs

- `0002-middleware-engine.md` — ProxyMiddleware trait (same abstraction pattern)
- `0003-channel-model.md` — Channel model (consumer of Storage trait)
- `0004-cost-tracking.md` — Cost tracking (consumer of Storage trait)
- `0008-security.md` — API key encryption (encryption layer sits above Storage trait, not inside it)
