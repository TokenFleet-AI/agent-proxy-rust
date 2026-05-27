# 0004 — Cost Tracking

> **Phase 1 only** — Cloud does not include cost tracking.

## Overview

Cost tracking records every proxied request with its actual spend and compression savings. Aggregated per project × model × date for dashboard display.

No "channel saving" calculations — only one saving dimension: **tokenless compression**.

## CostRecord

```rust
struct CostRecord {
    id: i64,
    timestamp: DateTime<Local>,

    // User
    user_name: String,          // from git config user.name

    // Project
    project_path: String,       // /Users/xxx/my-app
    project_name: String,       // my-app (from git remote or dir name)

    // Agent & Role
    agent_type: String,         // claude / codex / gemini
    agent_role: Option<String>, // Ruflo swarm role: architect / coder / tester / ...
                                // Detected from x-api-key → role_mapping config.
                                // None for non-Ruflo / standalone agent usage.

    // Channel
    channel_name: String,
    channel_kind: ChannelKind,  // subscription / metered

    // Model
    model_name: String,         // client-side model name: "claude-sonnet"

    // Token usage (from API response usage)
    input_tokens: u64,
    output_tokens: u64,
    cache_write_tokens: u64,
    cache_read_tokens: u64,
    thinking_tokens: u64,

    // Cost
    actual_cost: f64,           // 0 for subscription, real for metered
    unit: String,               // "USD" | "CNY" | "credits"

    // Compression (from tokenless)
    pre_compress_tokens: u64,   // token count BEFORE tokenless compression
    post_compress_tokens: u64,  // token count AFTER tokenless compression
    compression_tokens_saved: u64,
}
```

## SQLite Schema

```sql
CREATE TABLE cost_records (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp INTEGER NOT NULL,

    user_name TEXT NOT NULL DEFAULT '',

    project_path TEXT NOT NULL,
    project_name TEXT NOT NULL DEFAULT '',

    agent_type TEXT NOT NULL,
    agent_role TEXT,              -- NULL when not set (standalone agent, non-Ruflo)
    channel_name TEXT NOT NULL,
    channel_kind TEXT NOT NULL,

    model_name TEXT NOT NULL,

    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cache_write_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    thinking_tokens INTEGER NOT NULL DEFAULT 0,

    actual_cost REAL NOT NULL DEFAULT 0.0,
    unit TEXT NOT NULL DEFAULT 'USD',

    pre_compress_tokens INTEGER NOT NULL DEFAULT 0,
    post_compress_tokens INTEGER NOT NULL DEFAULT 0,
    compression_tokens_saved INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_cost_project_date ON cost_records(project_path, timestamp);
CREATE INDEX idx_cost_user_date ON cost_records(user_name, timestamp);
CREATE INDEX idx_cost_model_date ON cost_records(model_name, timestamp);
CREATE INDEX idx_cost_role_date ON cost_records(agent_role, timestamp);
```

## User Detection

Priority order:
1. `X-User-Name` header（Agent 包装脚本显式传入）
2. `git config user.name`（项目目录下的 git 用户名）
3. 操作系统当前用户名（macOS/Linux/Windows 通用）
4. `"unknown"`（以上都拿不到）

## Project Detection

Priority order:
1. `X-Project-Path` header (set by agent wrapper)
2. `X-Workspace` header (Claude Code native)
3. Request body `cwd` field (if present)
4. `std::env::current_dir()` of the proxy process (fallback, inaccurate for multi-project)

Project name extracted from:
1. `git remote get-url origin` → parse repo name
2. Directory basename

## Role Detection (Ruflo Swarm)

In Ruflo swarm deployments, multiple roles (architect, coder, tester, reviewer) each spawn their own agent instance. All use the same client type (e.g., all use Claude Code), so `agent_type` alone cannot distinguish roles.

### Mechanism: API Key Mapping

Each role's agent instance is configured with a unique proxy API key:

```
Ruflo agent_spawn:
  architect → ANTHROPIC_API_KEY=sk-proxy-architect  (→ x-api-key header)
  coder     → ANTHROPIC_API_KEY=sk-proxy-coder      (→ x-api-key header)
  tester    → ANTHROPIC_API_KEY=sk-proxy-tester     (→ x-api-key header)
```

The proxy maintains a role mapping in config:

```yaml
# config.yaml
role_mapping:
  sk-proxy-architect: architect
  sk-proxy-coder: coder
  sk-proxy-tester: tester
  sk-proxy-reviewer: reviewer
```

Detection flow in the auth Tower layer:

```
Request arrives with x-api-key: sk-proxy-coder
  ↓
Tower auth layer:
  1. Look up x-api-key in role_mapping → role = "coder"
  2. Inject role into ConnectionContext.agent_role
  3. Replace x-api-key with real channel API key for upstream forwarding
  ↓
CostMiddleware reads ctx.agent_role → writes to CostRecord.agent_role
```

No custom headers required — every AI agent client already sends its API key. The proxy reuses this existing header for dual purpose (auth + role identification), then swaps to the real key before forwarding upstream.

For non-Ruflo / standalone agent usage, `agent_role` is `None` (no mapping entry matches).

## Cost Calculation

```rust
fn calc_cost(usage: &Usage, mapping: &ModelMapping) -> (f64, &str) {
    match &mapping.pricing {
        Pricing::PerToken { input_per_mtok, output_per_mtok, cache_write, cache_read, thinking } => {
            let input_cost  = usage.input_tokens as f64 / 1_000_000.0 * input_per_mtok;
            let output_cost = usage.output_tokens as f64 / 1_000_000.0 * output_per_mtok;
            let cache_cost  = usage.cache_write_tokens as f64 / 1_000_000.0 * cache_write.unwrap_or(0.0)
                            + usage.cache_read_tokens as f64 / 1_000_000.0 * cache_read.unwrap_or(0.0);
            let thinking_cost = usage.thinking_tokens as f64 / 1_000_000.0 * thinking.unwrap_or(0.0);
            (input_cost + output_cost + cache_cost + thinking_cost, "USD")
        }
        Pricing::Credits { credits_per_mtok_input, credits_per_mtok_output, credits_per_request } => {
            let credits = usage.input_tokens as f64 / 1_000_000.0 * credits_per_mtok_input.unwrap_or(0.0)
                        + usage.output_tokens as f64 / 1_000_000.0 * credits_per_mtok_output.unwrap_or(0.0)
                        + credits_per_request.unwrap_or(0.0);
            (credits, "credits")
        }
        Pricing::CharBased { price_per_million_chars, output_multiplier } => {
            // Requires char count from upstream — most APIs don't provide this.
            // Fall back to token-based estimate: 1 token ≈ 0.75 chars (English avg)
            let input_chars = usage.input_tokens as f64 * 0.75;
            let output_chars = usage.output_tokens as f64 * 0.75 * output_multiplier.unwrap_or(1.0);
            ((input_chars + output_chars) / 1_000_000.0 * price_per_million_chars, "CNY")
        }
    }
}

fn calc_subscription(_mapping: &ModelMapping) -> (f64, &str) {
    (0.0, "USD")  // Monthly fee tracked separately, individual calls cost 0
}
```

### Usage Extraction

Different API formats have different usage JSON structures. The cost crate normalizes them into a unified `Usage` struct:

```rust
struct Usage {
    input_tokens: u64,
    output_tokens: u64,
    cache_write_tokens: u64,
    cache_read_tokens: u64,
    thinking_tokens: u64,
}
```

**Non-streaming extraction by format**:

- **Anthropic Messages**: `response.usage.input_tokens`, `.output_tokens`, `.cache_creation_input_tokens`, `.cache_read_input_tokens`
- **OpenAI Chat**: `response.usage.prompt_tokens`, `.completion_tokens`, `.prompt_tokens_details.cached_tokens`
- **OpenAI Responses**: `response.usage.input_tokens`, `.output_tokens`, `.input_tokens_details.cached_tokens`

**Streaming extraction**:

- **Anthropic**: parse `message_delta.usage` or final `message_stop` SSE event
- **OpenAI Chat**: parse final `[DONE]` chunk's `usage` field
- **OpenAI Responses**: parse `response.completed` event's `usage` field

```json
{
  "type": "message_delta",
  "usage": {
    "input_tokens": 1500,
    "output_tokens": 300
  }
}
```

## Compression Savings

```
CompressMiddleware BEFORE forwarding:
  Request body tokens: 12000
  After SchemaCompressor: 4500

CompressMiddleware AFTER upstream response:
  Response body tokens: 800
  After ResponseCompressor: 600

CostRecord:
  pre_compress_tokens: 12000 + 800 = 12800
  post_compress_tokens: 4500 + 600 = 5100
  compression_tokens_saved: 7700
```

Compression saving is tokens-saved — not converted to $ (the saving depends on which channel was selected, and for subscription channels there's no per-token cost).

## Subscription Monthly Tracking

Subscription channels have `actual_cost = 0` per request. Monthly fees are tracked in a separate table:

```sql
CREATE TABLE subscription_fees (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    channel_name TEXT NOT NULL,
    month TEXT NOT NULL,              -- "2026-05"
    monthly_price REAL NOT NULL,
    currency TEXT NOT NULL DEFAULT 'USD',
    created_at INTEGER NOT NULL
);
```

Aggregation queries join `subscription_fees` with `cost_records` to show total cost (metered + subscription):

```sql
SELECT
    strftime('%Y-%m', datetime(timestamp, 'unixepoch')) as month,
    SUM(cr.actual_cost) + COALESCE(sf.total_sub, 0) as total_cost
FROM cost_records cr
LEFT JOIN (
    SELECT month, SUM(monthly_price) as total_sub
    FROM subscription_fees
    GROUP BY month
) sf ON strftime('%Y-%m', datetime(cr.timestamp, 'unixepoch')) = sf.month
GROUP BY month;
```

## SQLite Configuration

The database MUST be opened in WAL mode for concurrent read/write across multiple agent connections:

```sql
PRAGMA journal_mode=WAL;
PRAGMA busy_timeout=5000;
PRAGMA foreign_keys=ON;
```

Use a single connection pool (e.g. `r2d2-sqlite` with `SqliteConnectionManager`) rather than opening per-request connections.

## Data Retention

Default retention policy: keep 90 days of detailed records, then aggregate and prune:

1. Daily: no action.
2. Weekly (background task): aggregate records older than 90 days into `cost_records_daily` (per project × model × day summary rows).
3. After aggregation, delete the original rows from `cost_records`.
4. Daily aggregates are kept indefinitely (negligible row count).

```sql
CREATE TABLE cost_records_daily (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    date TEXT NOT NULL,               -- "2026-05-26"
    project_path TEXT NOT NULL,
    project_name TEXT NOT NULL,
    agent_role TEXT,                  -- NULL = not set
    channel_name TEXT NOT NULL,
    model_name TEXT NOT NULL,
    total_input_tokens INTEGER NOT NULL,
    total_output_tokens INTEGER NOT NULL,
    total_cache_write_tokens INTEGER NOT NULL,
    total_cache_read_tokens INTEGER NOT NULL,
    total_thinking_tokens INTEGER NOT NULL,
    total_actual_cost REAL NOT NULL,
    total_compression_tokens_saved INTEGER NOT NULL,
    request_count INTEGER NOT NULL
);
```

## Aggregation Queries

```sql
-- Per-project monthly summary
SELECT
    project_name,
    SUM(input_tokens) as total_input,
    SUM(output_tokens) as total_output,
    SUM(actual_cost) as total_cost,
    SUM(compression_tokens_saved) as total_saved_tokens
FROM cost_records
WHERE timestamp >= ? AND timestamp < ?
GROUP BY project_path
ORDER BY total_cost DESC;

-- Per-model daily trend
SELECT
    date(timestamp, 'unixepoch') as day,
    model_name,
    SUM(input_tokens + output_tokens) as total_tokens,
    SUM(actual_cost) as cost
FROM cost_records
WHERE timestamp >= ? AND timestamp < ?
GROUP BY day, model_name
ORDER BY day;

-- Per-role cost breakdown (Ruflo swarm)
SELECT
    agent_role,
    SUM(input_tokens + output_tokens) as total_tokens,
    SUM(actual_cost) as total_cost,
    COUNT(*) as request_count
FROM cost_records
WHERE timestamp >= ? AND timestamp < ?
  AND agent_role IS NOT NULL
GROUP BY agent_role
ORDER BY total_cost DESC;

-- Compression savings summary
SELECT
    SUM(pre_compress_tokens) as total_pre,
    SUM(post_compress_tokens) as total_post,
    SUM(compression_tokens_saved) as total_saved,
    ROUND(100.0 * SUM(compression_tokens_saved) / SUM(pre_compress_tokens), 1) as savings_pct
FROM cost_records
WHERE timestamp >= ? AND timestamp < ?;
```
