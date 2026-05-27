//! Domain types for model routing, pricing, and channel health.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Pricing & Billing
// ---------------------------------------------------------------------------

/// Pricing formulas for cost calculation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Pricing {
    /// Per-token pricing (Anthropic, `OpenAI`, most providers).
    PerToken {
        /// Price per million input tokens (USD).
        input_per_mtok: f64,
        /// Price per million output tokens (USD).
        output_per_mtok: f64,
        /// Price per million cache write tokens (USD).
        #[serde(default)]
        cache_write_per_mtok: Option<f64>,
        /// Price per million cache read tokens (USD).
        #[serde(default)]
        cache_read_per_mtok: Option<f64>,
        /// Price per million thinking tokens (USD).
        #[serde(default)]
        thinking_per_mtok: Option<f64>,
    },
    /// Credit-based pricing (some resellers, internal platforms).
    Credits {
        /// Credits per million input tokens.
        #[serde(default)]
        credits_per_mtok_input: Option<f64>,
        /// Credits per million output tokens.
        #[serde(default)]
        credits_per_mtok_output: Option<f64>,
        /// Fixed credits per request.
        #[serde(default)]
        credits_per_request: Option<f64>,
    },
    /// Character-based pricing (some Chinese providers charge in CNY per million chars).
    CharBased {
        /// Price per million characters (CNY).
        price_per_million_chars: f64,
        /// Output character multiplier (output chars cost more).
        #[serde(default)]
        output_multiplier: Option<f64>,
    },
}

/// Billing mode for a channel mapping.
#[derive(Debug, Clone)]
pub enum ChannelBilling {
    /// Pay-per-use: cost calculated per token at request time.
    Metered {
        /// The pricing formula used to compute cost.
        pricing: Pricing,
    },
    /// Fixed fee: monthly subscription, prepaid bundle, free tier, or
    /// enterprise contract. Per-request cost = 0.
    FlatFee {
        /// Optional monthly cost for display purposes.
        monthly_cost_hint: Option<f64>,
        /// Usage quota before the channel is exhausted.
        quota: Option<Quota>,
        /// Action to take when the quota is exhausted.
        on_exhausted: ExhaustedAction,
    },
}

impl ChannelBilling {
    /// Parse from the storage-layer billing string and pricing JSON.
    ///
    /// # Errors
    ///
    /// Returns a string error if `billing_str` is unknown or `pricing_json`
    /// fails to parse.
    pub fn from_storage(billing_str: &str, pricing_json: &str) -> Result<Self, String> {
        match billing_str {
            "metered" => {
                let pricing: Pricing = serde_json::from_str(pricing_json)
                    .map_err(|e| format!("failed to parse pricing JSON: {e}"))?;
                Ok(Self::Metered { pricing })
            }
            "flatfee" => {
                #[derive(Deserialize)]
                struct FlatFeeConfig {
                    monthly_cost_hint: Option<f64>,
                    quota: Option<Quota>,
                    on_exhausted: Option<ExhaustedAction>,
                }
                let cfg: FlatFeeConfig =
                    serde_json::from_str(pricing_json).unwrap_or(FlatFeeConfig {
                        monthly_cost_hint: None,
                        quota: None,
                        on_exhausted: None,
                    });
                Ok(Self::FlatFee {
                    monthly_cost_hint: cfg.monthly_cost_hint,
                    quota: cfg.quota,
                    on_exhausted: cfg
                        .on_exhausted
                        .unwrap_or(ExhaustedAction::FallbackToMetered),
                })
            }
            other => Err(format!("unknown billing type: {other}")),
        }
    }

    /// Returns `true` when this is a flat-fee billing mode.
    #[must_use]
    pub fn is_flat_fee(&self) -> bool {
        matches!(self, Self::FlatFee { .. })
    }

    /// Returns the pricing if this is a metered billing mode.
    #[must_use]
    pub fn pricing(&self) -> Option<&Pricing> {
        match self {
            Self::Metered { pricing } => Some(pricing),
            Self::FlatFee { .. } => None,
        }
    }
}

/// Usage quota for a flat-fee channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Quota {
    /// No limit.
    Unlimited,
    /// Maximum number of requests per month.
    MaxRequests {
        /// Request limit per calendar month.
        per_month: u64,
    },
    /// Maximum number of tokens per month.
    MaxTokens {
        /// Token limit per calendar month.
        per_month: u64,
    },
}

/// Action when a flat-fee channel's quota is exhausted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExhaustedAction {
    /// Fall through to metered channels.
    FallbackToMetered,
    /// Return an error (503).
    Block,
}

// ---------------------------------------------------------------------------
// Channel Health
// ---------------------------------------------------------------------------

/// Binary health status for a channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelHealth {
    /// Channel is responding normally.
    Healthy,
    /// Channel has failed and is in cooldown.
    Unhealthy,
}

/// Runtime health state for a channel, tracked in-memory.
#[derive(Debug, Clone)]
pub struct ChannelState {
    /// Current health.
    pub health: ChannelHealth,
    /// When the channel last failed (used for cooldown calculation).
    pub failed_at: Option<Instant>,
    /// Number of consecutive failures seen. Resets to 0 on success.
    pub consecutive_failures: u32,
}

impl ChannelState {
    /// Creates a new healthy channel state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            health: ChannelHealth::Healthy,
            failed_at: None,
            consecutive_failures: 0,
        }
    }

    /// Records a failure. After 3 consecutive failures the channel is
    /// marked Unhealthy with a cooldown.
    pub fn record_failure(&mut self) {
        self.consecutive_failures += 1;
        if self.consecutive_failures >= 3 {
            self.health = ChannelHealth::Unhealthy;
            self.failed_at = Some(Instant::now());
        }
    }

    /// Records a success: resets the failure counter and marks the
    /// channel Healthy.
    pub fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.health = ChannelHealth::Healthy;
        self.failed_at = None;
    }

    /// Marks the channel as unhealthy immediately (used for 5xx errors).
    pub fn mark_unhealthy(&mut self) {
        self.consecutive_failures = 3; // treat as 3 failures
        self.health = ChannelHealth::Unhealthy;
        self.failed_at = Some(Instant::now());
    }

    /// Returns `true` when the channel can be tried.
    #[must_use]
    pub fn is_tryable(&self, cooldown: Duration) -> bool {
        match self.health {
            ChannelHealth::Healthy => true,
            ChannelHealth::Unhealthy => self.failed_at.is_none_or(|t| t.elapsed() >= cooldown),
        }
    }
}

impl Default for ChannelState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::unwrap_in_result,
    clippy::unchecked_duration_subtraction,
    clippy::panic
)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn test_channel_state_default_healthy() {
        let state = ChannelState::default();
        assert!(matches!(state.health, ChannelHealth::Healthy));
        assert!(state.failed_at.is_none());
    }

    #[test]
    fn test_channel_state_mark_unhealthy() {
        let mut state = ChannelState::default();
        state.mark_unhealthy();
        assert!(matches!(state.health, ChannelHealth::Unhealthy));
        assert!(state.failed_at.is_some());
    }

    #[test]
    fn test_channel_state_record_success_clears_failure() {
        let mut state = ChannelState::default();
        state.mark_unhealthy();
        state.record_success();
        assert!(matches!(state.health, ChannelHealth::Healthy));
        assert!(state.failed_at.is_none());
        assert_eq!(state.consecutive_failures, 0);
    }

    #[test]
    fn test_channel_state_cooldown_expired() {
        let state = ChannelState {
            health: ChannelHealth::Unhealthy,
            failed_at: Some(Instant::now() - Duration::from_secs(61)),
            consecutive_failures: 3,
        };
        assert!(state.is_tryable(Duration::from_secs(60)));
    }

    #[test]
    fn test_channel_state_within_cooldown() {
        let mut state = ChannelState::default();
        state.mark_unhealthy();
        assert!(!state.is_tryable(Duration::from_secs(60)));
    }

    #[test]
    fn test_channel_billing_is_flat_fee() {
        let billing = ChannelBilling::FlatFee {
            monthly_cost_hint: Some(20.0),
            quota: Some(Quota::MaxRequests { per_month: 1000 }),
            on_exhausted: ExhaustedAction::FallbackToMetered,
        };
        assert!(billing.is_flat_fee());
        assert!(billing.pricing().is_none());
    }

    #[test]
    fn test_channel_billing_metered_has_pricing() {
        let billing = ChannelBilling::Metered {
            pricing: Pricing::PerToken {
                input_per_mtok: 3.0,
                output_per_mtok: 15.0,
                cache_write_per_mtok: None,
                cache_read_per_mtok: None,
                thinking_per_mtok: None,
            },
        };
        assert!(!billing.is_flat_fee());
        assert!(billing.pricing().is_some());
    }

    #[test]
    fn test_channel_billing_from_storage_metered() {
        let json = r#"{"type":"per_token","input_per_mtok":3.0,"output_per_mtok":15.0}"#;
        let billing = ChannelBilling::from_storage("metered", json).unwrap();
        assert!(!billing.is_flat_fee());
    }

    #[test]
    fn test_channel_billing_from_storage_flatfee() {
        let json = r#"{"monthly_cost_hint":20.0,"quota":{"MaxRequests":{"per_month":1000}},"on_exhausted":"fallback_to_metered"}"#;
        let billing = ChannelBilling::from_storage("flatfee", json).unwrap();
        assert!(billing.is_flat_fee());
    }

    #[test]
    fn test_channel_billing_from_storage_unknown_billing() {
        let err = ChannelBilling::from_storage("unknown", "{}").unwrap_err();
        assert!(err.contains("unknown billing type"));
    }

    #[test]
    fn test_pricing_serde_per_token() {
        let json = r#"{"type":"per_token","input_per_mtok":3.0,"output_per_mtok":15.0}"#;
        let pricing: Pricing = serde_json::from_str(json).unwrap();
        match pricing {
            Pricing::PerToken {
                input_per_mtok,
                output_per_mtok,
                ..
            } => {
                assert!((input_per_mtok - 3.0).abs() < f64::EPSILON);
                assert!((output_per_mtok - 15.0).abs() < f64::EPSILON);
            }
            _ => panic!("expected PerToken"),
        }
    }

    #[test]
    fn test_pricing_serde_credits() {
        let json =
            r#"{"type":"credits","credits_per_mtok_input":1.0,"credits_per_mtok_output":2.0}"#;
        let pricing: Pricing = serde_json::from_str(json).unwrap();
        assert!(matches!(pricing, Pricing::Credits { .. }));
    }

    #[test]
    fn test_pricing_serde_char_based() {
        let json = r#"{"type":"char_based","price_per_million_chars":2.0}"#;
        let pricing: Pricing = serde_json::from_str(json).unwrap();
        assert!(matches!(pricing, Pricing::CharBased { .. }));
    }
}
