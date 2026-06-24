//! Cost tracking for agent-proxy-rust.
//!
//! Extracts token usage from upstream API responses, calculates cost using
//! the pricing formula from the selected model mapping, and writes cost records
//! to the storage backend.
//!
//! # Usage
//!
//! `CostMiddleware` implements [`agent_proxy_rust_core::CostRecorder`] and is
//! registered via [`AgentProxyBuilder::cost_recorder`]. The server engine calls
//! `record()` after the `on_response` chain completes.

#![forbid(unsafe_code)]
#![warn(missing_docs, missing_debug_implementations)]
// u64 → f64 conversion is inherent to token cost calculation;
// 1,000,000 tokens at $3/MTok = $3 — precision loss is negligible.
// u64 → i64 is safe because token values fit comfortably in i64.
// request_time_ms uses try_from to avoid u128→i64 truncation.
#![allow(clippy::cast_precision_loss, clippy::cast_possible_wrap)]

use std::sync::Arc;

use agent_proxy_rust_core::{
    CompressionStats, CostRecorder, ProxyError,
    extensions::{EXT_COMPRESSION_STATS, EXT_SELECTED_CHANNEL, EXT_SELECTED_MAPPING},
    types::{ApiFormat, ChannelConfig, ConnectionContext},
};
use agent_proxy_rust_model_router::{
    BillingDimension, Pricing, PricingTier, SelectedMappingInfo, TierPrice,
};
use agent_proxy_rust_storage::{CostFilter, CostRecord, Storage, TimeRange};
use chrono::Utc;

/// Unified token usage extracted from upstream API responses.
#[derive(Debug, Clone, Default)]
pub struct Usage {
    /// Input / prompt tokens.
    pub input_tokens: u64,
    /// Output / completion tokens.
    pub output_tokens: u64,
    /// Tokens written to the provider's cache.
    pub cache_write_tokens: u64,
    /// Tokens read from the provider's cache.
    pub cache_read_tokens: u64,
    /// Thinking / reasoning tokens (Anthropic extended thinking).
    pub thinking_tokens: u64,
}

/// Cost tracking middleware.
///
/// Called after the upstream response is received. Extracts usage from the
/// response body, calculates cost, and writes a `CostRecord` to storage.
#[derive(Debug)]
pub struct CostMiddleware {
    storage: Arc<dyn Storage>,
    user_name: String,
}

#[async_trait::async_trait]
impl CostRecorder for CostMiddleware {
    async fn record(
        &self,
        ctx: &ConnectionContext,
        response_body: &serde_json::Value,
    ) -> Result<(), ProxyError> {
        self.record(ctx, response_body).await
    }
}

impl CostMiddleware {
    /// Creates a new [`CostMiddleware`].
    #[must_use]
    pub fn new(storage: Arc<dyn Storage>, user_name: String) -> Self {
        Self { storage, user_name }
    }

    /// Records a cost entry for the completed request.
    ///
    /// # Errors
    ///
    /// Returns `ProxyError` if the storage backend write fails.
    pub async fn record(
        &self,
        ctx: &ConnectionContext,
        response_body: &serde_json::Value,
    ) -> Result<(), ProxyError> {
        let mapping_info = ctx.get::<SelectedMappingInfo>(EXT_SELECTED_MAPPING);
        let channel_config = ctx.get::<ChannelConfig>(EXT_SELECTED_CHANNEL);
        let stats = ctx.get::<CompressionStats>(EXT_COMPRESSION_STATS);

        let channel_id = mapping_info.map_or(String::new(), |m| m.channel_id.clone());
        // Upstream channel display name (e.g. "DeepSeek Official")
        let upstream_channel = channel_config.map_or(String::new(), |c| c.name.clone());
        // Upstream model name sent to the API (e.g. "deepseek-v4-pro")
        let upstream_model = mapping_info.map_or(String::new(), |m| m.upstream_name.clone());

        // Use the client-detected format for usage extraction.
        // After bridge conversion, the response body is in the client's format
        // (detected_format), not the upstream's format (target_protocol).
        // Fall back to auto-detection if detected_format is None (should not
        // normally happen since non-matching paths are rejected before proxying).
        let usage = extract_usage(response_body, ctx.detected_format);

        // Compute actual cost from pricing
        let (actual_cost, unit) = match mapping_info.and_then(|m| m.pricing.as_ref()) {
            Some(pricing) => calc_cost(&usage, pricing),
            None => (0.0, "USD".to_string()),
        };

        // Compression stats from all layers:
        // - proxy layer: schema/response compression from CompressMiddleware
        // - tokenless layer: rtk/response/schema from hook reports
        let schema_saved = ctx.tokenless_schema_saved
            + stats.map_or(
                0,
                agent_proxy_rust_core::CompressionStats::proxy_schema_saved,
            );
        let response_saved = ctx.tokenless_response_saved
            + stats.map_or(
                0,
                agent_proxy_rust_core::CompressionStats::proxy_response_saved,
            );
        let rtk_saved = ctx.tokenless_rtk_saved + stats.map_or(0, |s| s.rtk_saved);
        let tokenless_saved =
            stats.map_or(0, agent_proxy_rust_core::CompressionStats::tokenless_saved);
        let experimental_saved = stats.map_or(0, |s| s.tokenless_experimental);
        let pre_compress = stats.map_or(0, |s| s.tokenless_pre + s.proxy_req_pre);
        let post_compress = stats.map_or(0, |s| s.tokenless_post + s.proxy_req_post);
        let compression_saved =
            tokenless_saved + experimental_saved + schema_saved + response_saved + rtk_saved;
        let pricing_snapshot_json =
            mapping_info.map_or(String::new(), |m| m.pricing_snapshot_json.clone());

        // Total tokens consumed by upstream API
        let after_tokens = usage.input_tokens + usage.output_tokens;
        // Tokenless saved tokens (accumulated from hook reports)
        let tokenless_saved = ctx.tokenless_saved_tokens;
        // All compression layers combined
        let total_saved = tokenless_saved + compression_saved;

        let record = CostRecord {
            id: uuid::Uuid::now_v7().to_string(),
            channel_id,
            upstream_channel,
            upstream_model,
            request_time_ms: i64::try_from(ctx.started_at.elapsed().as_millis())
                .unwrap_or(i64::MAX),
            project: ctx.project_path.clone().unwrap_or_default(),
            user_id: ctx
                .user_name
                .clone()
                .unwrap_or_else(|| self.user_name.clone()),
            agent_type: ctx.agent_type.to_string(),
            input_tokens: usage.input_tokens as i64,
            output_tokens: usage.output_tokens as i64,
            cache_write_tokens: usage.cache_write_tokens as i64,
            cache_read_tokens: usage.cache_read_tokens as i64,
            thinking_tokens: usage.thinking_tokens as i64,
            cost: actual_cost,
            schema_saved_tokens: schema_saved as i64,
            response_saved_tokens: response_saved as i64,
            rtk_saved_tokens: rtk_saved as i64,
            pre_compress_tokens: pre_compress as i64,
            post_compress_tokens: post_compress as i64,
            compression_tokens_saved: compression_saved as i64,
            unit,
            timestamp: Utc::now().to_rfc3339(),
            pricing_snapshot_json,
            session_id: ctx.session_id.clone(),
            before_tokens: (after_tokens + total_saved) as i64,
            after_tokens: after_tokens as i64,
            tokens_saved: total_saved as i64,
            compression_breakdown_json: ctx.tokenless_breakdown_json.clone().unwrap_or_default(),
        };

        self.storage
            .insert_cost_record(&record)
            .await
            .map_err(|e| ProxyError::Internal(e.into()))?;

        Ok(())
    }

    /// Queries cost records with optional filters.
    ///
    /// # Errors
    ///
    /// Returns `ProxyError` if the storage query fails.
    pub async fn query(&self, filter: CostFilter) -> Result<Vec<CostRecord>, ProxyError> {
        self.storage
            .query_cost_records(filter)
            .await
            .map_err(|e| ProxyError::Internal(e.into()))
    }

    /// Aggregates costs grouped by dimension within a time range.
    ///
    /// # Errors
    ///
    /// Returns `ProxyError` if the storage query fails.
    pub async fn aggregate(
        &self,
        group_by: agent_proxy_rust_storage::CostGroupBy,
        range: TimeRange,
    ) -> Result<Vec<agent_proxy_rust_storage::CostAggregate>, ProxyError> {
        self.storage
            .aggregate_costs(group_by, range)
            .await
            .map_err(|e| ProxyError::Internal(e.into()))
    }
}

// ── Usage Extraction ────────────────────────────────────────────────

/// Extracts unified [`Usage`] from an upstream API response body.
///
/// Handles three formats:
/// - **Anthropic Messages**: `usage.input_tokens`, `.output_tokens`,
///   `.cache_creation_input_tokens`, `.cache_read_input_tokens`
/// - **`OpenAI` Chat**: `usage.prompt_tokens`, `.completion_tokens`,
///   `.prompt_tokens_details.cached_tokens`
/// - **`OpenAI` Responses**: `usage.input_tokens`, `.output_tokens`,
///   `.input_tokens_details.cached_tokens`
#[must_use]
pub fn extract_usage(body: &serde_json::Value, format: Option<ApiFormat>) -> Usage {
    match format {
        Some(ApiFormat::AnthropicMessages) => extract_anthropic(body),
        Some(ApiFormat::OpenaiChat) => extract_openai_chat(body),
        Some(ApiFormat::OpenaiResponses) => extract_openai_responses(body),
        None | Some(_) => auto_detect_usage(body),
    }
}

fn auto_detect_usage(body: &serde_json::Value) -> Usage {
    let Some(usage) = body.get("usage") else {
        return Usage::default();
    };
    // OpenAI Chat uses `prompt_tokens`; Anthropic and Responses use `input_tokens`.
    if usage.get("prompt_tokens").is_some() {
        return extract_openai_chat(body);
    }
    // OpenAI Responses has `input_tokens_details` (not present in Anthropic).
    if usage.get("input_tokens_details").is_some() {
        return extract_openai_responses(body);
    }
    // Anthropic has `cache_creation_input_tokens`; also the default fallback
    // when only `input_tokens` is present (both Anthropic and Responses share
    // the field name, but Anthropic is more common).
    if usage.get("input_tokens").is_some() {
        return extract_anthropic(body);
    }
    Usage::default()
}

fn extract_anthropic(body: &serde_json::Value) -> Usage {
    let Some(usage) = body.get("usage") else {
        return Usage::default();
    };
    Usage {
        input_tokens: usage
            .get("input_tokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        output_tokens: usage
            .get("output_tokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        cache_write_tokens: usage
            .get("cache_creation_input_tokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        cache_read_tokens: usage
            .get("cache_read_input_tokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        thinking_tokens: 0,
    }
}

fn extract_openai_chat(body: &serde_json::Value) -> Usage {
    let Some(usage) = body.get("usage") else {
        return Usage::default();
    };
    let cache_read = usage
        .get("prompt_tokens_details")
        .and_then(|d| d.get("cached_tokens"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);

    // Standard OpenAI: prompt_tokens + completion_tokens
    let prompt_tokens = usage
        .get("prompt_tokens")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let completion_tokens = usage
        .get("completion_tokens")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);

    // DashScope / some providers: total_tokens only (split as 50/50 if no breakdown)
    // Or use input/output as alternative field names
    let (input, output) = if prompt_tokens > 0 || completion_tokens > 0 {
        (prompt_tokens, completion_tokens)
    } else if let Some(total) = usage
        .get("total_tokens")
        .and_then(serde_json::Value::as_u64)
    {
        // Fallback: split total as 50/50 (conservative estimate)
        (total / 2, total / 2)
    } else {
        // Try DashScope-specific field names
        let input = usage
            .get("input")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let output = usage
            .get("output")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        (input, output)
    };

    Usage {
        input_tokens: input,
        output_tokens: output,
        cache_write_tokens: 0,
        cache_read_tokens: cache_read,
        thinking_tokens: 0,
    }
}

fn extract_openai_responses(body: &serde_json::Value) -> Usage {
    let Some(usage) = body.get("usage") else {
        return Usage::default();
    };
    let cache_read = usage
        .get("input_tokens_details")
        .and_then(|d| d.get("cached_tokens"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    Usage {
        input_tokens: usage
            .get("input_tokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        output_tokens: usage
            .get("output_tokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        cache_write_tokens: 0,
        cache_read_tokens: cache_read,
        thinking_tokens: 0,
    }
}

// ── Streaming Usage Extraction ──────────────────────────────────────

/// Extracts unified [`Usage`] from a streaming SSE response body.
///
/// Parses `data:` lines looking for usage-bearing events:
/// - **Anthropic**: `message_delta` event with `usage` field
/// - **`OpenAI` Chat**: final chunk with `usage` field (before `[DONE]`)
/// - **`OpenAI` Responses**: `response.completed` event with `usage` field
///
/// The last usage event wins (streams may emit intermediate usage).
#[must_use]
pub fn extract_usage_sse(body: &str) -> Usage {
    let mut usage = Usage::default();
    for line in body.lines() {
        let data = line.strip_prefix("data: ").unwrap_or(line);
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let Ok(event) = serde_json::from_str::<serde_json::Value>(data) else {
            continue;
        };
        // Anthropic message_delta
        if event.get("type").and_then(|v| v.as_str()) == Some("message_delta")
            && let Some(u) = event.get("usage")
        {
            usage = extract_anthropic_from_usage(u);
        }
        // OpenAI Responses completed
        if event.get("type").and_then(|v| v.as_str()) == Some("response.completed")
            && let Some(u) = event.get("response").and_then(|r| r.get("usage"))
        {
            usage = extract_openai_responses_from_usage(u);
        }
        // OpenAI Chat: has "choices" and "usage"
        if event.get("choices").is_some()
            && let Some(u) = event.get("usage")
        {
            usage = extract_openai_chat_from_usage(u);
        }
    }
    usage
}

fn extract_anthropic_from_usage(usage: &serde_json::Value) -> Usage {
    Usage {
        input_tokens: usage
            .get("input_tokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        output_tokens: usage
            .get("output_tokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        cache_write_tokens: usage
            .get("cache_creation_input_tokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        cache_read_tokens: usage
            .get("cache_read_input_tokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        thinking_tokens: 0,
    }
}

fn extract_openai_chat_from_usage(usage: &serde_json::Value) -> Usage {
    Usage {
        input_tokens: usage
            .get("prompt_tokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        output_tokens: usage
            .get("completion_tokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        cache_write_tokens: 0,
        cache_read_tokens: 0,
        thinking_tokens: 0,
    }
}

fn extract_openai_responses_from_usage(usage: &serde_json::Value) -> Usage {
    Usage {
        input_tokens: usage
            .get("input_tokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        output_tokens: usage
            .get("output_tokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        cache_write_tokens: 0,
        cache_read_tokens: 0,
        thinking_tokens: 0,
    }
}

// ── Cost Calculation ────────────────────────────────────────────────

/// Calculates the cost for a request given its usage and pricing formula.
///
/// Returns `(cost, unit)` where `unit` is `"USD"`, `"CNY"`, or `"credits"`.
#[must_use]
pub fn calc_cost(usage: &Usage, pricing: &Pricing) -> (f64, String) {
    match pricing {
        Pricing::PerToken {
            input_per_mtok,
            output_per_mtok,
            cache_write_per_mtok,
            cache_read_per_mtok,
            thinking_per_mtok,
            currency,
        } => calc_per_token(
            usage,
            *input_per_mtok,
            *output_per_mtok,
            *cache_write_per_mtok,
            *cache_read_per_mtok,
            *thinking_per_mtok,
            currency,
        ),
        Pricing::Credits {
            credits_per_mtok_input,
            credits_per_mtok_output,
            credits_per_request,
        } => {
            let input =
                usage.input_tokens as f64 / 1_000_000.0 * credits_per_mtok_input.unwrap_or(0.0);
            let output =
                usage.output_tokens as f64 / 1_000_000.0 * credits_per_mtok_output.unwrap_or(0.0);
            let per_req = credits_per_request.unwrap_or(0.0);
            (input + output + per_req, "credits".to_string())
        }
        Pricing::CharBased {
            price_per_million_chars,
            output_multiplier,
        } => {
            let input_chars = usage.input_tokens as f64 * 0.75;
            let output_chars = usage.output_tokens as f64 * 0.75 * output_multiplier.unwrap_or(1.0);
            (
                (input_chars + output_chars) / 1_000_000.0 * price_per_million_chars,
                "CNY".to_string(),
            )
        }
        Pricing::PerUnit {
            per_unit, currency, ..
        } => (*per_unit, currency.clone()),
        Pricing::Tiered {
            dimension: BillingDimension::Tokens,
            tiers,
            currency,
        } => calc_tiered_tokens(usage, tiers, currency),
        Pricing::Tiered { currency, .. } => (0.0, currency.clone()),
    }
}

/// Calculates per-token cost.
fn calc_per_token(
    usage: &Usage,
    input_per_mtok: f64,
    output_per_mtok: f64,
    cache_write_per_mtok: Option<f64>,
    cache_read_per_mtok: Option<f64>,
    thinking_per_mtok: Option<f64>,
    currency: &str,
) -> (f64, String) {
    let input_cost = usage.input_tokens as f64 / 1_000_000.0 * input_per_mtok;
    let output_cost = usage.output_tokens as f64 / 1_000_000.0 * output_per_mtok;
    let cache_write_cost =
        usage.cache_write_tokens as f64 / 1_000_000.0 * cache_write_per_mtok.unwrap_or(0.0);
    let cache_read_cost =
        usage.cache_read_tokens as f64 / 1_000_000.0 * cache_read_per_mtok.unwrap_or(0.0);
    let thinking_cost =
        usage.thinking_tokens as f64 / 1_000_000.0 * thinking_per_mtok.unwrap_or(0.0);
    (
        input_cost + output_cost + cache_write_cost + cache_read_cost + thinking_cost,
        currency.to_string(),
    )
}

/// Calculates cost using a tiered token pricing schedule.
///
/// Selects the first tier whose `up_to` bound covers the main token count
/// (input + output). Cache and thinking tokens are excluded from tier
/// selection because they are priced separately at lower rates and should
/// not push a request into a more expensive tier.
fn calc_tiered_tokens(usage: &Usage, tiers: &[PricingTier], currency: &str) -> (f64, String) {
    let total_tokens = usage.input_tokens + usage.output_tokens;
    let tier = tiers.iter().find(|t| match t.up_to {
        None => true,
        Some(limit) => total_tokens <= limit,
    });
    match tier.and_then(|t| match &t.price {
        TierPrice::Token {
            input_per_mtok,
            output_per_mtok,
            cache_write_per_mtok,
            cache_read_per_mtok,
            thinking_per_mtok,
        } => Some(calc_per_token(
            usage,
            *input_per_mtok,
            *output_per_mtok,
            *cache_write_per_mtok,
            *cache_read_per_mtok,
            *thinking_per_mtok,
            currency,
        )),
        TierPrice::Unit { .. } => None,
    }) {
        Some(result) => result,
        None => (0.0, currency.to_string()),
    }
}

/// Calculates cost for a subscription channel (zero per-request cost).
#[must_use]
pub fn calc_subscription_cost() -> (f64, String) {
    (0.0, "USD".to_string())
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::unwrap_in_result, clippy::panic)]
mod tests {
    use rstest::rstest;

    use super::*;

    // ── Usage extraction ────────────────────────────────────────

    #[test]
    fn test_extract_anthropic_usage() {
        let body = serde_json::json!({
            "id": "msg_123",
            "type": "message",
            "usage": {
                "input_tokens": 1500,
                "output_tokens": 300,
                "cache_creation_input_tokens": 500,
                "cache_read_input_tokens": 200
            }
        });
        let usage = extract_usage(&body, Some(ApiFormat::AnthropicMessages));
        assert_eq!(usage.input_tokens, 1500);
        assert_eq!(usage.output_tokens, 300);
        assert_eq!(usage.cache_write_tokens, 500);
        assert_eq!(usage.cache_read_tokens, 200);
        assert_eq!(usage.thinking_tokens, 0);
    }

    #[test]
    fn test_extract_openai_chat_usage() {
        let body = serde_json::json!({
            "id": "chatcmpl-123",
            "usage": {
                "prompt_tokens": 800,
                "completion_tokens": 150,
                "prompt_tokens_details": { "cached_tokens": 300 }
            }
        });
        let usage = extract_usage(&body, Some(ApiFormat::OpenaiChat));
        assert_eq!(usage.input_tokens, 800);
        assert_eq!(usage.output_tokens, 150);
        assert_eq!(usage.cache_read_tokens, 300);
        assert_eq!(usage.cache_write_tokens, 0);
    }

    #[test]
    fn test_extract_openai_responses_usage() {
        let body = serde_json::json!({
            "id": "resp_123",
            "usage": {
                "input_tokens": 1200,
                "output_tokens": 400,
                "input_tokens_details": { "cached_tokens": 500 }
            }
        });
        let usage = extract_usage(&body, Some(ApiFormat::OpenaiResponses));
        assert_eq!(usage.input_tokens, 1200);
        assert_eq!(usage.output_tokens, 400);
        assert_eq!(usage.cache_read_tokens, 500);
    }

    #[test]
    fn test_extract_usage_missing_returns_zero() {
        let body = serde_json::json!({"id": "msg_123"});
        let usage = extract_usage(&body, Some(ApiFormat::AnthropicMessages));
        assert_eq!(usage.input_tokens, 0);
        assert_eq!(usage.output_tokens, 0);
    }

    #[test]
    fn test_extract_usage_none_format_autodetects() {
        let body = serde_json::json!({
            "usage": { "prompt_tokens": 100, "completion_tokens": 50 }
        });
        let usage = extract_usage(&body, None);
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
    }

    #[test]
    fn test_should_autodetect_openai_responses_via_input_tokens_details() {
        let body = serde_json::json!({
            "usage": {
                "input_tokens": 1200,
                "output_tokens": 400,
                "input_tokens_details": { "cached_tokens": 500 }
            }
        });
        let usage = extract_usage(&body, None);
        assert_eq!(usage.input_tokens, 1200);
        assert_eq!(usage.output_tokens, 400);
        assert_eq!(usage.cache_read_tokens, 500);
    }

    #[test]
    fn test_should_autodetect_anthropic_via_cache_creation_tokens() {
        let body = serde_json::json!({
            "usage": {
                "input_tokens": 1500,
                "output_tokens": 300,
                "cache_creation_input_tokens": 500,
                "cache_read_input_tokens": 200
            }
        });
        let usage = extract_usage(&body, None);
        assert_eq!(usage.input_tokens, 1500);
        assert_eq!(usage.output_tokens, 300);
        assert_eq!(usage.cache_write_tokens, 500);
        assert_eq!(usage.cache_read_tokens, 200);
    }

    // ── Streaming extraction ────────────────────────────────────

    #[test]
    fn test_extract_anthropic_streaming_message_delta() {
        let sse = r#"data: {"type":"message_start"}
data: {"type":"content_block_delta","delta":{"text":"Hello"}}
data: {"type":"message_delta","usage":{"input_tokens":1500,"output_tokens":300}}
data: {"type":"message_stop"}
"#;
        let usage = extract_usage_sse(sse);
        assert_eq!(usage.input_tokens, 1500);
        assert_eq!(usage.output_tokens, 300);
    }

    #[test]
    fn test_extract_openai_chat_streaming() {
        let sse = r#"data: {"choices":[{"delta":{"content":"Hi"}}]}
data: {"choices":[{"delta":{}}],"usage":{"prompt_tokens":800,"completion_tokens":150}}
data: [DONE]
"#;
        let usage = extract_usage_sse(sse);
        assert_eq!(usage.input_tokens, 800);
        assert_eq!(usage.output_tokens, 150);
    }

    #[test]
    fn test_extract_openai_responses_streaming() {
        let sse = r#"data: {"type":"response.created"}
data: {"type":"response.output_text.delta","delta":"Hello"}
data: {"type":"response.completed","response":{"usage":{"input_tokens":1200,"output_tokens":400}}}
"#;
        let usage = extract_usage_sse(sse);
        assert_eq!(usage.input_tokens, 1200);
        assert_eq!(usage.output_tokens, 400);
    }

    // ── Cost calculation ────────────────────────────────────────

    #[test]
    fn test_calc_cost_per_token() {
        let usage = Usage {
            input_tokens: 1_000_000,
            output_tokens: 500_000,
            cache_write_tokens: 100_000,
            cache_read_tokens: 200_000,
            thinking_tokens: 0,
        };
        let pricing = Pricing::PerToken {
            input_per_mtok: 3.0,
            output_per_mtok: 15.0,
            cache_write_per_mtok: Some(3.75),
            cache_read_per_mtok: Some(0.3),
            thinking_per_mtok: None,
            currency: "USD".to_string(),
        };
        let (cost, unit) = calc_cost(&usage, &pricing);
        assert_eq!(unit, "USD");
        // 1M * 3.0/1M = 3.0 + 0.5M * 15.0/1M = 7.5 + 0.1M * 3.75/1M = 0.375 + 0.2M * 0.3/1M = 0.06
        let expected = 3.0 + 7.5 + 0.375 + 0.06;
        assert!((cost - expected).abs() < 0.001);
    }

    #[test]
    fn test_calc_cost_credits() {
        let usage = Usage {
            input_tokens: 1_000_000,
            output_tokens: 500_000,
            ..Default::default()
        };
        let pricing = Pricing::Credits {
            credits_per_mtok_input: Some(1.0),
            credits_per_mtok_output: Some(2.0),
            credits_per_request: Some(0.01),
        };
        let (cost, unit) = calc_cost(&usage, &pricing);
        assert_eq!(unit, "credits");
        let expected = 1.0 + 1.0 + 0.01;
        assert!((cost - expected).abs() < 0.001);
    }

    #[test]
    fn test_calc_cost_char_based() {
        let usage = Usage {
            input_tokens: 1_000_000,
            output_tokens: 500_000,
            ..Default::default()
        };
        let pricing = Pricing::CharBased {
            price_per_million_chars: 2.0,
            output_multiplier: Some(1.0),
        };
        let (cost, unit) = calc_cost(&usage, &pricing);
        assert_eq!(unit, "CNY");
        let expected = 2.25;
        assert!((cost - expected).abs() < 0.001);
    }

    #[test]
    fn test_calc_subscription_cost_zero() {
        let (cost, unit) = calc_subscription_cost();
        assert!((cost - 0.0).abs() < f64::EPSILON);
        assert_eq!(unit, "USD");
    }

    #[test]
    fn test_calc_cost_per_unit() {
        let usage = Usage::default();
        let pricing = Pricing::PerUnit {
            metric: BillingDimension::Duration {
                resolution: Some("1080p".into()),
            },
            per_unit: 0.50,
            currency: "USD".into(),
        };
        let (cost, unit) = calc_cost(&usage, &pricing);
        assert_eq!(unit, "USD");
        assert!((cost - 0.50).abs() < 0.001);
    }

    #[test]
    fn test_calc_cost_tiered_tokens_first_tier() {
        let usage = Usage {
            input_tokens: 1_000_000,
            output_tokens: 500_000,
            ..Default::default()
        };
        let pricing = Pricing::Tiered {
            dimension: BillingDimension::Tokens,
            currency: "CNY".into(),
            tiers: vec![
                PricingTier {
                    up_to: Some(1_000_000_000),
                    price: TierPrice::Token {
                        input_per_mtok: 1.0,
                        output_per_mtok: 2.0,
                        cache_write_per_mtok: None,
                        cache_read_per_mtok: Some(0.02),
                        thinking_per_mtok: None,
                    },
                },
                PricingTier {
                    up_to: Some(5_000_000_000),
                    price: TierPrice::Token {
                        input_per_mtok: 0.8,
                        output_per_mtok: 1.6,
                        cache_write_per_mtok: None,
                        cache_read_per_mtok: Some(0.015),
                        thinking_per_mtok: None,
                    },
                },
                PricingTier {
                    up_to: None,
                    price: TierPrice::Token {
                        input_per_mtok: 0.5,
                        output_per_mtok: 1.0,
                        cache_write_per_mtok: None,
                        cache_read_per_mtok: Some(0.01),
                        thinking_per_mtok: None,
                    },
                },
            ],
        };
        let (cost, unit) = calc_cost(&usage, &pricing);
        assert_eq!(unit, "CNY");
        // 1M input * 1.0 + 0.5M output * 2.0 = 1.0 + 1.0 = 2.0
        assert!((cost - 2.0).abs() < 0.001);
    }

    #[test]
    fn test_calc_cost_tiered_tokens_with_cache_read() {
        let usage = Usage {
            input_tokens: 1_000_000,
            output_tokens: 500_000,
            cache_read_tokens: 2_000_000,
            ..Default::default()
        };
        let pricing = Pricing::Tiered {
            dimension: BillingDimension::Tokens,
            currency: "USD".into(),
            tiers: vec![PricingTier {
                up_to: None,
                price: TierPrice::Token {
                    input_per_mtok: 3.0,
                    output_per_mtok: 15.0,
                    cache_write_per_mtok: None,
                    cache_read_per_mtok: Some(0.3),
                    thinking_per_mtok: None,
                },
            }],
        };
        let (cost, unit) = calc_cost(&usage, &pricing);
        assert_eq!(unit, "USD");
        // 1M * 3.0 + 0.5M * 15.0 + 2M * 0.3 = 3.0 + 7.5 + 0.6 = 11.1
        assert!((cost - 11.1).abs() < 0.001);
    }

    // ── Parameterized pricing ───────────────────────────────────

    #[rstest]
    #[case("per_token", 3.0, 15.0, 1_000_000, 500_000, 10.5)]
    #[case("credits", 1.0, 2.0, 1_000_000, 500_000, 2.0)]
    #[case("char_based", 2.0, 0.0, 1_000_000, 500_000, 2.25)]
    fn test_pricing_calculation(
        #[case] mode: &str,
        #[case] input_rate: f64,
        #[case] output_rate: f64,
        #[case] input_tokens: u64,
        #[case] output_tokens: u64,
        #[case] expected_approx: f64,
    ) {
        let usage = Usage {
            input_tokens,
            output_tokens,
            ..Default::default()
        };
        let pricing = match mode {
            "per_token" => Pricing::PerToken {
                input_per_mtok: input_rate,
                output_per_mtok: output_rate,
                cache_write_per_mtok: None,
                cache_read_per_mtok: None,
                thinking_per_mtok: None,
                currency: "USD".to_string(),
            },
            "credits" => Pricing::Credits {
                credits_per_mtok_input: Some(input_rate),
                credits_per_mtok_output: Some(output_rate),
                credits_per_request: None,
            },
            "char_based" => Pricing::CharBased {
                price_per_million_chars: input_rate,
                output_multiplier: Some(1.0),
            },
            _ => unreachable!(),
        };
        let (cost, _unit) = calc_cost(&usage, &pricing);
        assert!(
            (cost - expected_approx).abs() < 0.01,
            "mode={mode}: expected ≈{expected_approx}, got {cost}"
        );
    }
}
