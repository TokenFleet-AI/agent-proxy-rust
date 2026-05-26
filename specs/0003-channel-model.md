# 0003 — Channel Model & Selection Strategy

> **Phase 1**: Simple priority list (subscription → metered), binary Healthy/Unhealthy, 5 default channels.
> **Phase 2**: Health probe loop, three-state degradation, weighted random with standby, last-resort fallback.

---

## Data Model

```
Channel
┌──────────────────────────────────────┐
│ name: "Anthropic Official"           │
│ url: "https://api.anthropic.com"     │
│ api_key: "sk-ant-xxx"                │
│ protocol: AnthropicMessages           │  ← determines if bridge is needed
│ is_builtin: false                    │
│ enabled: true                        │
│                                      │
│ model_mappings: [                    │
│   ┌────────────────────────────────┐ │
│   │ client_name: "claude-sonnet"   │ │  ← model name from client request
│   │ upstream_name: "claude-4-7"    │ │  ← model name sent to upstream
│   │ billing: Metered                │ │
│   │ pricing: PerToken { ... }      │ │
│   │ enabled: true                   │ │
│   └────────────────────────────────┘ │
│   ...                                │
│ ]                                    │
└──────────────────────────────────────┘
```

## Channel Billing

```rust
enum ChannelBilling {
    /// Pay-per-use: weighted random selection, cost calculated per token
    Metered {
        pricing: Pricing,
    },
    /// Fixed fee: prioritized, per-request cost = 0
    /// Covers monthly subscriptions, prepaid bundles, free tiers, enterprise contracts
    FlatFee {
        monthly_cost_hint: Option<f64>,
        quota: Option<Quota>,
        on_exhausted: ExhaustedAction,
    },
}

enum Quota {
    Unlimited,
    MaxRequests { per_month: u64 },
    MaxTokens { per_month: u64 },
}

enum ExhaustedAction {
    FallbackToMetered,
    Block,
}
```

## Pricing Modes

```rust
enum Pricing {
    PerToken {
        input_per_mtok: f64,
        output_per_mtok: f64,
        cache_write_per_mtok: Option<f64>,
        cache_read_per_mtok: Option<f64>,
        thinking_per_mtok: Option<f64>,
    },
}
```

> **扩展**: Add `Credits` and `CharBased` pricing variants if needed (not planned for Phase 2 cloud).

---

## Selection Strategy (Phase 1)

```
find_mapping(client_name: "claude-sonnet")
        │
        ▼
Scan all channels for model_mappings where client_name == "claude-sonnet"
  && channel.enabled && mapping.enabled
        │
        ▼
┌─ FlatFee channels with quota > 0 and healthy?
│   → Yes: use first available
│   → Exhausted with FallbackToMetered → fall through
│   → Exhausted with Block → return error
│
│ (none available)
│
┌─ Metered channels healthy?
│   → Yes: use first healthy   (Phase 1: simple first-match, no weights)
│   → No: return error
```

Phase 1 uses **first-match** for metered channels. No weighted random — with 2-3 channels on a single-user machine, simple ordering is sufficient. Users control priority by channel order.

## Selection Strategy (Phase 2)

> **Reserved design — not implemented in Phase 1.**

```
Phase 1 flow → after Metered channels found:
│
├─ Weighted random selection (weight field)
├─ Standby channels (weight=0) excluded unless all others unhealthy
└─ Last resort: any channel, health ignored (better than returning error)
```

```rust
fn weighted_random(candidates: &[&ModelMapping]) -> Option<&ModelMapping> {
    let total: u32 = candidates.iter().map(|m| m.weight).sum();
    if total == 0 { return None; }
    let pick = rand::thread_rng().gen_range(0..total);
    let mut cumulative = 0;
    for m in candidates {
        cumulative += m.weight;
        if pick < cumulative { return Some(m); }
    }
    candidates.last().copied()
}
```

---

## Channel Health (Phase 1)

```rust
struct ChannelState {
    health: ChannelHealth,
    failed_at: Option<Instant>,
}

enum ChannelHealth {
    Healthy,
    Unhealthy,
}
```

Simple cooldown-based recovery. No separate probe requests:

1. Request fails (connection error, timeout, 5xx, 429) → mark Unhealthy, record `failed_at`.
2. Next request: skip Unhealthy channels.
3. If all channels are Unhealthy, or cooldown (60s) has passed since `failed_at` → try the channel anyway on the next real request. The actual user request IS the probe.
4. Request succeeds → mark Healthy, clear `failed_at`. Request fails → reset `failed_at` timer.

No background loop. No separate `GET /v1/models` probe. No Degraded state. The next real request tests whether the channel has recovered.

## Channel Health (Phase 2)

> **Reserved design — not implemented in Phase 1.**

```rust
enum ChannelHealth {
    Healthy,
    Degraded { consecutive_failures: u32 },
    Unhealthy { until: Instant },
}

// Transitions:
// Healthy + 429 → Degraded (failures=1)
// Degraded + 429 → Degraded (failures++)
// Degraded failures >= 3 → Unhealthy (until: now + 60s)
// Unhealthy + 60s passed + health_check() succeeds → Healthy
```

60s background health probe loop with per-protocol probe endpoints (`GET /v1/models` for Anthropic, `GET /models` for OpenAI). Error code differentiation: 429 counts toward degradation, 5xx counts double, 4xx (non-429) is client fault — not counted.

---

## Protocol Detection

```
Channel.protocol vs Client request path:
  AnthropicMessages + POST /v1/messages       → passthrough
  AnthropicMessages + POST /v1/chat/completions → anthropic_to_openai()
  AnthropicMessages + POST /v1/responses       → anthropic_to_responses()
  OpenaiChat        + POST /v1/messages       → openai_to_anthropic()
  OpenaiChat        + POST /v1/chat/completions → passthrough
  OpenaiChat        + POST /v1/responses       → openai_to_responses()
  OpenaiResponses   + POST /v1/messages       → responses_to_anthropic()
  OpenaiResponses   + POST /v1/chat/completions → responses_to_openai()
  OpenaiResponses   + POST /v1/responses       → passthrough
```

Bridge decision: `channel.protocol != detected_format`.

## API Key Storage

Phase 1: OS file permissions (`chmod 600` on SQLite db). In memory, keys held in `secrecy::SecretString` for safe `Debug` output.

> **Phase 2**: AES-256-GCM encryption at rest (see 0008).

## Configuration Source

```sql
CREATE TABLE channels (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    url TEXT NOT NULL,
    api_key TEXT NOT NULL,
    protocol TEXT NOT NULL,
    is_builtin BOOLEAN DEFAULT 0,
    enabled BOOLEAN DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE model_mappings (
    id TEXT PRIMARY KEY,
    channel_id TEXT REFERENCES channels(id) ON DELETE CASCADE,
    client_name TEXT NOT NULL,
    upstream_name TEXT NOT NULL,
    billing TEXT NOT NULL,           -- "metered" | "flatfee"
    pricing_json TEXT NOT NULL,
    weight INTEGER DEFAULT 100,      -- Phase 2: used by weighted random
    enabled BOOLEAN DEFAULT 1
);
```

## Default Channels (Builtin)

On first run, proxy seeds default channels for 4 core providers (no API key):

| channel_id | provider | url | protocol |
|------------|----------|-----|----------|
| anthropic-official | anthropic | https://api.anthropic.com | anthropic_messages |
| openai-official | openai | https://api.openai.com | openai_responses |
| gemini-official | google | https://generativelanguage.googleapis.com | openai_chat |
| deepseek-official | deepseek | https://api.deepseek.com | openai_chat |

All seeded with `api_key=""` and `is_builtin=true`. See `0005-builtin-channels.md` for provider registry.

> **Phase 2**: Add OpenRouter default channel, expand to 10+ providers via community registry fetch.

## Provider → Channel Mapping

Each channel references a provider. Multiple channels can point to the same provider:

| Provider | Official | Reseller / Proxy | Cloud Platform |
|----------|----------|------------------|----------------|
| Anthropic | api.anthropic.com | OpenRouter, DashScope | AWS Bedrock |
| OpenAI | api.openai.com | OpenRouter, Copilot | Azure OpenAI |
| Google | generativelanguage.googleapis.com | OpenRouter | GCP Vertex AI |
| DeepSeek | api.deepseek.com | DashScope, 百炼 | — |

## Seed Logic (Phase 1)

```rust
fn seed_on_startup(db: &Database) -> Result<()> {
    // Seed providers + models from embedded data
    let builtin = include_str!("../data/builtin-providers.json");
    seed_providers(db, builtin)?;

    // Seed default channels (4 core providers)
    let default_channels = include_str!("../data/default-channels.json");
    seed_channels(db, default_channels)?;

    Ok(())
}
```

> **Phase 2**: Add `fetch_pricing_registry()` for community repo sync (3s timeout, best-effort).

## User Workflow

```bash
# First run: 4 default channels seeded
agent-proxy serve

# List channels
agent-proxy channel list

# Set API key
agent-proxy channel set-key anthropic-official --api-key "sk-ant-xxx"

# Add a new channel
agent-proxy channel add openrouter \
  --url "https://openrouter.ai/api/v1" \
  --protocol anthropic_messages \
  --api-key "sk-or-xxx"
```

## Data Separation Summary

| What | Where | Contains Secrets? |
|------|-------|-------------------|
| Provider definitions | Embedded JSON (`builtin-providers.json`) | No |
| Model list & pricing | Embedded JSON (`builtin-providers.json`) | No |
| Default channels (4) | Embedded JSON (`default-channels.json`) | No |
| User channels | SQLite `channels` table | **Yes** (api_key) |
| User model overrides | SQLite `model_mappings` table | No |
