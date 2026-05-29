# 0015 - Health State Machine

## 1. Source

Migrated from `token-fleet-switch/crates/channel-engine/src/lib.rs`.

The existing agent-proxy-rust model-router uses a simplified binary health model
(Healthy/Unhealthy). The token-fleet-switch implementation provides a richer
4-tier state machine with 429/5xx differentiation.

## 2. State Transitions

```text
                    ┌─────────┐
        ┌──────────►│ Healthy │◄──────────┐
        │ success   └────┬─────┘           │
        │                │                  │
        │          429 failure               │ cooldown expired
        │           ┌────▼─────┐            │ (auto-recover)
        │           │ Degraded │            │
        │           │(1-2次失败)│            │
        │           └────┬─────┘            │
        │                │                  │
        │           ≥3 consecutive 429      │
        │           ┌────▼─────┐   ────────┘
        │ success   │ Cooldown │
        │(手动恢复)  │ (60s)   │
        │           └──────────┘
        │
        │           5xx (any)
        │           ┌────▼──────┐
        └───────────│Unavailable│
         success    │(manual or │
                    │ timeout)  │
                    └──────────┘
```

## 3. Failure Classification

| HTTP Status | Effect | Description |
|-------------|--------|-------------|
| **5xx (500-599)** | Immediate `Unavailable` | Server-side error, no retry reasonable |
| **429** | Counter increment | Rate limit; ≤2 → Degraded; ≥3 → Cooldown |
| **4xx (non-429)** | No effect | Client errors not counted as channel failures |

## 4. Persistence

Unlike the original token-fleet-switch HSM which is purely in-memory (DashMap),
the migrated version persists state to the `channels` table:

```sql
-- channels table columns for health tracking
health_status        TEXT    NOT NULL DEFAULT 'Healthy'
cooldown_until       TEXT    -- ISO 8601 or NULL
consecutive_failures INTEGER NOT NULL DEFAULT 0
```

**On startup:** Load all channels' health state from DB into in-memory cache.
**On state change:** Update both in-memory cache AND DB row.
**On cooldown check:** In-memory first (fast path), DB as backup.

## 5. Integration with ModelRouterMiddleware

```rust
pub struct ModelRouterMiddleware {
    channels: Vec<ResolvedChannel>,
    health: Arc<DashMap<String, ChannelState>>,   // in-memory cache
    quota_usage: Arc<DashMap<String, QuotaUsage>>,
}

impl ModelRouterMiddleware {
    /// Records a failure for a channel. Updates both cache and DB.
    pub async fn record_failure(&self, channel_id: &str, status_code: u16) {
        let mut state = self.health.entry(channel_id.to_string()).or_default();
        match status_code {
            500..=599 => {
                state.health = ChannelHealth::Unavailable;
                state.consecutive_failures = 3;
                state.failed_at = Some(Instant::now());
            }
            429 => {
                state.consecutive_failures += 1;
                if state.consecutive_failures >= 3 {
                    state.health = ChannelHealth::Cooldown;
                    state.failed_at = Some(Instant::now());
                } else {
                    state.health = ChannelHealth::Degraded;
                }
            }
            _ => {}
        }
        // Persist to DB
        self.storage.update_channel_health(
            channel_id,
            state.health,
            state.consecutive_failures,
            state.cooldown_until(),
        ).await;
    }

    /// Records a success. Resets to Healthy.
    pub async fn record_success(&self, channel_id: &str) {
        let mut state = self.health.entry(channel_id.to_string()).or_default();
        state.health = ChannelHealth::Healthy;
        state.consecutive_failures = 0;
        state.failed_at = None;
        self.storage.update_channel_health(channel_id, ChannelHealth::Healthy, 0, None).await;
    }

    /// Checks if the channel should be excluded from routing.
    fn is_cooling(&self, channel_id: &str) -> bool {
        let Some(mut state) = self.health.get_mut(channel_id) else { return false };
        match state.health {
            ChannelHealth::Unavailable => true,
            ChannelHealth::Cooldown => {
                if let Some(at) = state.failed_at {
                    if at.elapsed() >= COOLDOWN_DURATION {
                        // Auto-recover
                        state.health = ChannelHealth::Healthy;
                        state.consecutive_failures = 0;
                        state.failed_at = None;
                        // Async persist (fire-and-forget)
                        false
                    } else {
                        true
                    }
                } else {
                    false
                }
            }
            _ => false,
        }
    }
}
```

## 6. Channel Selection with Health Gating

```rust
fn select_channel(
    model_name: &str,
    channels: &[Channel],
    health: &HealthStateMachine,
    is_quota_exceeded: impl Fn(&str) -> bool,
) -> Result<SelectedChannel> {
    // 1. Filter enabled channels
    // 2. Partition into flatfee / metered
    // 3. Sort each group by priority (ascending)

    // Step 1: FlatFee first
    for ch in &flatfee {
        if health.is_cooling(&ch.id) { continue; }
        if ch.monthly_quota.is_some() && is_quota_exceeded(&ch.id) {
            match ch.quota_policy {
                Block => return Err(QuotaExhausted),
                FallbackToMetered => continue,
            }
        }
        return Ok(SelectedChannel { channel: ch.clone() });
    }

    // Step 2: Metered fallback
    for ch in &metered {
        if health.is_cooling(&ch.id) { continue; }
        return Ok(SelectedChannel { channel: ch.clone() });
    }

    Err(NoChannelAvailable(model_name.into()))
}
```

## 7. Cooldown Recovery

Cooldown duration: **60 seconds** (configurable).

On expiry:
1. State auto-transitions to `Healthy`
2. `consecutive_failures` reset to 0
3. Channel becomes eligible for routing again
4. DB row updated

## 8. Health API Endpoints

Available via Admin API:

| Endpoint | Description |
|----------|-------------|
| `GET /admin/channels/{id}` | Returns channel with current health status |
| `POST /admin/channels/{id}/healthy` | Manually reset channel to Healthy |
| `POST /admin/channels/{id}/failure` | Record a failure (for testing/admin) |
| `GET /admin/health` | Aggregate health: counts of Healthy/Degraded/Cooldown/Unavailable |

## 9. Testing

| Test Scenario | Expected |
|---------------|----------|
| 5xx → immediate Unavailable | health = Unavailable, failures = 3 |
| 429 × 1 → Degraded | health = Degraded, failures = 1 |
| 429 × 3 → Cooldown | health = Cooldown, is_cooling = true |
| Cooldown expiry → auto-recover | After 60s, health = Healthy |
| Success resets Degraded | health = Healthy, failures = 0 |
| Non-429 4xx → no effect | health unchanged |
| Unavailable → requires manual reset | is_cooling stays true (or POST /healthy) |
| All channels in cooldown → error | NoChannelAvailable |
