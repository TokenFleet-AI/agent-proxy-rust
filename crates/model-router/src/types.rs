//! Domain types for model routing, pricing, and channel health.

use std::time::{Duration, Instant};

use chrono::Datelike;
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
        /// Pricing currency (e.g. "USD", "CNY").
        #[serde(default = "default_currency")]
        currency: String,
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
    /// Flat per-unit pricing without tiers (video duration, image count, etc.).
    PerUnit {
        /// What is being metered.
        metric: BillingDimension,
        /// Price per unit (e.g. per second, per image).
        per_unit: f64,
        /// Currency.
        #[serde(default = "default_currency")]
        currency: String,
    },
    /// Tiered/gradient pricing — rates change based on cumulative usage volume.
    Tiered {
        /// What dimension is being metered.
        dimension: BillingDimension,
        /// Tiers ordered from lowest to highest range. The first tier whose
        /// `up_to` covers the cumulative usage is selected.
        tiers: Vec<PricingTier>,
        /// Currency.
        #[serde(default = "default_currency")]
        currency: String,
    },
}

/// The billing dimension for metered and tiered pricing.
///
/// Carries optional parameters relevant to that dimension (e.g. video
/// resolution, image quality).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BillingDimension {
    /// Token-based billing (LLM text APIs).
    Tokens,
    /// Duration-based billing (video/audio APIs).
    Duration {
        /// Optional resolution e.g. `"720p"`, `"1080p"`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resolution: Option<String>,
    },
    /// Image-count-based billing (image generation APIs).
    Images {
        /// Optional quality e.g. `"standard"`, `"hd"`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        quality: Option<String>,
    },
}

/// Per-unit price within a tier, discriminated by dimension type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TierPrice {
    /// Token-based pricing fields.
    Token {
        /// Price per million input tokens.
        input_per_mtok: f64,
        /// Price per million output tokens.
        output_per_mtok: f64,
        /// Price per million cache write tokens.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache_write_per_mtok: Option<f64>,
        /// Price per million cache read tokens.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache_read_per_mtok: Option<f64>,
        /// Price per million thinking tokens.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thinking_per_mtok: Option<f64>,
    },
    /// Generic per-unit pricing (for duration/image tiers).
    Unit {
        /// Price per unit (second, image, etc.).
        per_unit: f64,
    },
}

/// A single tier in a [`Pricing::Tiered`] schedule.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PricingTier {
    /// Upper bound of this tier (inclusive). `None` means unlimited.
    pub up_to: Option<u64>,
    /// Per-unit price for this tier.
    pub price: TierPrice,
}

/// Default currency for `Pricing::PerToken`.
fn default_currency() -> String {
    "USD".to_string()
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

/// Per-mapping monthly quota consumption tracker.
#[derive(Debug, Clone)]
pub struct QuotaUsage {
    /// Number of requests this month.
    pub requests_this_month: u64,
    /// Number of tokens consumed this month.
    pub tokens_this_month: u64,
    /// The month this counter is tracking (year * 12 + month).
    pub month_key: u32,
}

impl QuotaUsage {
    /// Returns the current month key (year * 12 + month, Jan = 1).
    #[must_use]
    pub fn current_month_key() -> u32 {
        let now = chrono::Utc::now();
        u32::try_from(now.year()).unwrap_or(2026) * 12 + now.month()
    }

    /// Creates a new counter for the current month.
    #[must_use]
    pub fn new() -> Self {
        Self {
            requests_this_month: 0,
            tokens_this_month: 0,
            month_key: Self::current_month_key(),
        }
    }

    /// Resets if the month has changed.
    pub fn maybe_reset(&mut self) {
        let current = Self::current_month_key();
        if current != self.month_key {
            self.requests_this_month = 0;
            self.tokens_this_month = 0;
            self.month_key = current;
        }
    }

    /// Records a request with its token usage.
    pub fn record_usage(&mut self, tokens: u64) {
        self.maybe_reset();
        self.requests_this_month += 1;
        self.tokens_this_month += tokens;
    }

    /// Checks whether `quota` has been exceeded.
    #[must_use]
    pub fn is_within_quota(&self, quota: Option<&Quota>) -> bool {
        match quota {
            None | Some(Quota::Unlimited) => true,
            Some(Quota::MaxRequests { per_month }) => self.requests_this_month < *per_month,
            Some(Quota::MaxTokens { per_month }) => self.tokens_this_month < *per_month,
        }
    }
}

impl Default for QuotaUsage {
    fn default() -> Self {
        Self::new()
    }
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

    /// Marks the channel as rate-limited with a short fixed cooldown.
    ///
    /// Used for HTTP 429 responses. Sets a 30-second cooldown so the
    /// channel is retried quickly without exponential backoff.
    pub fn mark_rate_limited(&mut self) {
        self.consecutive_failures = 1;
        self.health = ChannelHealth::Unhealthy;
        self.failed_at = Some(Instant::now());
    }

    /// Returns `true` when the channel can be tried after the cooldown has passed.
    ///
    /// Rate-limited channels (1 failure) use a 30-second cooldown.
    /// Other failures use exponential backoff based on `base_cooldown`.
    #[must_use]
    pub fn is_tryable_past_cooldown(&self) -> bool {
        const RATE_LIMIT_COOLDOWN: Duration = Duration::from_secs(30);
        const BASE_COOLDOWN: Duration = Duration::from_secs(60);

        match self.health {
            ChannelHealth::Healthy => true,
            ChannelHealth::Unhealthy => {
                let effective = if self.consecutive_failures <= 1 {
                    RATE_LIMIT_COOLDOWN
                } else {
                    exponential_cooldown(self.consecutive_failures, BASE_COOLDOWN)
                };
                self.failed_at.is_none_or(|t| t.elapsed() >= effective)
            }
        }
    }

    /// Returns `true` when the channel can be tried.
    #[must_use]
    pub fn is_tryable(&self, base_cooldown: Duration) -> bool {
        match self.health {
            ChannelHealth::Healthy => true,
            ChannelHealth::Unhealthy => {
                let effective = exponential_cooldown(self.consecutive_failures, base_cooldown);
                self.failed_at.is_none_or(|t| t.elapsed() >= effective)
            }
        }
    }
}

impl Default for ChannelState {
    fn default() -> Self {
        Self::new()
    }
}

/// Computes exponential backoff cooldown based on consecutive failures.
///
/// - 1st failure: `base`
/// - 2nd failure: `base * 5`
/// - 3rd+ failure: `base * 15`
#[must_use]
pub fn exponential_cooldown(consecutive_failures: u32, base: Duration) -> Duration {
    match consecutive_failures {
        0..=1 => base,
        2 => base * 5,
        _ => base * 15,
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
    fn test_exponential_cooldown_first_failure_60s() {
        assert_eq!(
            exponential_cooldown(1, Duration::from_secs(60)),
            Duration::from_secs(60)
        );
    }

    #[test]
    fn test_exponential_cooldown_second_failure_300s() {
        assert_eq!(
            exponential_cooldown(2, Duration::from_secs(60)),
            Duration::from_secs(300)
        );
    }

    #[test]
    fn test_exponential_cooldown_third_failure_900s() {
        assert_eq!(
            exponential_cooldown(3, Duration::from_secs(60)),
            Duration::from_secs(900)
        );
    }

    #[test]
    fn test_channel_state_cooldown_expired_first_failure() {
        // 1 failure → 60s cooldown, 61s elapsed → tryable
        let state = ChannelState {
            health: ChannelHealth::Unhealthy,
            failed_at: Some(Instant::now() - Duration::from_secs(61)),
            consecutive_failures: 1,
        };
        assert!(state.is_tryable(Duration::from_secs(60)));
    }

    #[test]
    fn test_channel_state_cooldown_not_expired_third_failure() {
        // 3 failures → 900s cooldown, 61s elapsed → NOT tryable
        let state = ChannelState {
            health: ChannelHealth::Unhealthy,
            failed_at: Some(Instant::now() - Duration::from_secs(61)),
            consecutive_failures: 3,
        };
        assert!(!state.is_tryable(Duration::from_secs(60)));
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
                currency: "USD".to_string(),
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

    #[test]
    fn test_pricing_serde_per_unit() {
        let json = r#"{"type":"per_unit","metric":{"type":"duration","resolution":"1080p"},"per_unit":0.5,"currency":"USD"}"#;
        let pricing: Pricing = serde_json::from_str(json).unwrap();
        match pricing {
            Pricing::PerUnit {
                metric,
                per_unit,
                currency,
            } => {
                assert!((per_unit - 0.5).abs() < f64::EPSILON);
                assert_eq!(currency, "USD");
                assert_eq!(
                    metric,
                    BillingDimension::Duration {
                        resolution: Some("1080p".into())
                    }
                );
            }
            _ => panic!("expected PerUnit"),
        }
    }

    #[test]
    fn test_pricing_serde_tiered_tokens() {
        let json = r#"{"type":"tiered","dimension":{"type":"tokens"},"currency":"CNY","tiers":[{"up_to":1000000000,"price":{"type":"token","input_per_mtok":1.0,"output_per_mtok":2.0}},{"up_to":null,"price":{"type":"token","input_per_mtok":0.5,"output_per_mtok":1.0}}]}"#;
        let pricing: Pricing = serde_json::from_str(json).unwrap();
        match pricing {
            Pricing::Tiered {
                dimension,
                tiers,
                currency,
            } => {
                assert_eq!(dimension, BillingDimension::Tokens);
                assert_eq!(currency, "CNY");
                assert_eq!(tiers.len(), 2);
                assert_eq!(tiers[0].up_to, Some(1_000_000_000));
                assert!(matches!(tiers[0].price, TierPrice::Token { .. }));
                assert_eq!(tiers[1].up_to, None);
            }
            _ => panic!("expected Tiered"),
        }
    }

    #[test]
    fn test_billing_dimension_serde_tokens() {
        let json = r#"{"type":"tokens"}"#;
        let dim: BillingDimension = serde_json::from_str(json).unwrap();
        assert_eq!(dim, BillingDimension::Tokens);
    }

    #[test]
    fn test_billing_dimension_serde_images_with_quality() {
        let json = r#"{"type":"images","quality":"hd"}"#;
        let dim: BillingDimension = serde_json::from_str(json).unwrap();
        assert_eq!(
            dim,
            BillingDimension::Images {
                quality: Some("hd".into())
            }
        );
    }

    #[test]
    fn test_tier_price_serde_token() {
        let json = r#"{"type":"token","input_per_mtok":3.0,"output_per_mtok":15.0,"cache_read_per_mtok":0.3}"#;
        let price: TierPrice = serde_json::from_str(json).unwrap();
        match price {
            TierPrice::Token {
                input_per_mtok,
                output_per_mtok,
                cache_read_per_mtok,
                ..
            } => {
                assert!((input_per_mtok - 3.0).abs() < f64::EPSILON);
                assert!((output_per_mtok - 15.0).abs() < f64::EPSILON);
                assert_eq!(cache_read_per_mtok, Some(0.3));
            }
            TierPrice::Unit { .. } => panic!("expected Token, got Unit"),
        }
    }

    #[test]
    fn test_tier_price_serde_unit() {
        let json = r#"{"type":"unit","per_unit":0.04}"#;
        let price: TierPrice = serde_json::from_str(json).unwrap();
        match price {
            TierPrice::Unit { per_unit } => {
                assert!((per_unit - 0.04).abs() < f64::EPSILON);
            }
            TierPrice::Token { .. } => panic!("expected Unit, got Token"),
        }
    }
}
