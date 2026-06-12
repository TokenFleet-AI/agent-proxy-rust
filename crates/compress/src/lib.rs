//! Compress middleware — token compression via tokenless-schema.
//!
//! - `on_request`: compresses tool definitions with [`tokenless_schema::SchemaCompressor`]
//! - `on_response`: compresses the response body with [`tokenless_schema::ResponseCompressor`]
//!
//! Token counts are tracked via [`agent_proxy_rust_core::CompressionStats`]
//! and written to `ctx.extensions` under the `EXT_COMPRESSION_STATS` key.

mod token_counter;

use std::{
    io::Write,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use agent_proxy_rust_core::{
    CompressionStats, ProxyError,
    extensions::{EXT_COMPRESSION_STATS, EXT_STATS_RECORD},
    middleware::ProxyMiddleware,
    types::{ConnectionContext, ProxyRequest, ProxyResponse},
};
use async_trait::async_trait;
use bytes::Bytes;
use tokenless_schema::{ResponseCompressor, SchemaCompressor};
use tracing::debug;

/// Middleware that compresses LLM API payloads to reduce token consumption.
///
/// Must be registered **first** in the middleware chain (before model-router
/// and bridge) so that compression runs on the original request body.
#[derive(Debug)]
pub struct CompressMiddleware {
    schema_compressor: SchemaCompressor,
    response_compressor: ResponseCompressor,
    /// Response stats collected during `on_response` (ctx is immutable there).
    response_stats: Mutex<Option<ResponseStats>>,
    /// When `false`, both `on_request` and `on_response` are no-ops.
    enabled: Arc<AtomicBool>,
}

/// Token counts collected during response compression.
#[derive(Debug, Clone, Copy, Default)]
#[allow(dead_code)]
pub struct ResponseStats {
    /// Estimated tokens before compression.
    pub pre_tokens: u64,
    /// Estimated tokens after compression.
    pub post_tokens: u64,
}

impl Default for CompressMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

impl CompressMiddleware {
    /// Creates a new [`CompressMiddleware`] with compression enabled.
    #[must_use]
    pub fn new() -> Self {
        Self {
            schema_compressor: SchemaCompressor::new()
                .with_func_desc_max_len(256)
                .with_param_desc_max_len(160)
                .with_drop_titles(true)
                .with_drop_examples(true)
                .with_drop_markdown(true)
                .with_max_enum_items(100),
            response_compressor: ResponseCompressor::new(),
            response_stats: Mutex::new(None),
            enabled: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Creates a new [`CompressMiddleware`] with compression **disabled**.
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            enabled: Arc::new(AtomicBool::new(false)),
            ..Self::new()
        }
    }

    /// Returns a shared handle to the enabled flag, allowing external
    /// callers (e.g. admin API) to toggle compression at runtime.
    #[must_use]
    pub fn enabled_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.enabled)
    }

    /// Returns `true` if compression is currently enabled.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Enables or disables compression at runtime.
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    /// Takes the response compression stats collected during the last
    /// `on_response` call. Returns `None` if no response has been processed.
    #[must_use]
    pub fn take_response_stats(&self) -> Option<ResponseStats> {
        self.response_stats.lock().ok()?.take()
    }
}

#[async_trait]
impl ProxyMiddleware for CompressMiddleware {
    async fn on_request(
        &self,
        req: &mut ProxyRequest,
        ctx: &mut ConnectionContext,
    ) -> Result<(), ProxyError> {
        if !self.is_enabled() {
            return Ok(());
        }
        let mut body: serde_json::Value = serde_json::from_slice(&req.body)
            .map_err(|e| ProxyError::BadRequest(format!("invalid JSON: {e}")))?;

        // Estimate pre-compression tokens
        let pre_tokens = token_counter::count(&req.body);

        // Compress tools array if present
        let mut compressed = false;
        let after_snapshot;
        if let Some(tools) = body.get_mut("tools").and_then(|v| v.as_array_mut()) {
            let tools_count = tools.len();
            let before_snapshot = abbreviate_tools_for_log(tools);

            for tool in tools.iter_mut() {
                let original = tool.clone();
                let result = self.schema_compressor.compress(tool);
                if result != original {
                    *tool = result;
                    compressed = true;
                }
            }

            if compressed {
                after_snapshot = abbreviate_tools_for_log(tools);

                let new_body = serde_json::to_vec(&body)
                    .map_err(|e| ProxyError::BadRequest(format!("serialize error: {e}")))?;
                let post_tokens = token_counter::count(&new_body);
                let saved = pre_tokens.saturating_sub(post_tokens);

                debug!(
                    pre = pre_tokens,
                    post = post_tokens,
                    saved,
                    "compress: schema compression"
                );

                req.body = Bytes::from(new_body);

                let mut stats = ctx
                    .get::<CompressionStats>(EXT_COMPRESSION_STATS)
                    .cloned()
                    .unwrap_or_default();
                let tokenless_post = stats.tokenless_post;
                stats.proxy_req_pre = pre_tokens;
                stats.proxy_req_post = post_tokens;
                ctx.insert(EXT_COMPRESSION_STATS, stats);

                let legacy = serde_json::json!({
                    "input_tokens": pre_tokens + tokenless_post,
                    "output_tokens": post_tokens,
                });
                ctx.insert(EXT_STATS_RECORD, legacy);

                write_schema_debug_log(
                    tools_count,
                    pre_tokens,
                    post_tokens,
                    saved,
                    &before_snapshot,
                    &after_snapshot,
                );
            }
        }

        Ok(())
    }

    async fn on_response(
        &self,
        res: &mut ProxyResponse,
        _ctx: &ConnectionContext,
    ) -> Result<(), ProxyError> {
        if !self.is_enabled() {
            return Ok(());
        }

        // Skip streaming responses
        if res.is_streaming {
            return Ok(());
        }

        let pre_tokens = token_counter::count(&res.body);
        let body_val: serde_json::Value = match serde_json::from_slice(&res.body) {
            Ok(v) => v,
            Err(_) => return Ok(()), // non-JSON body, skip
        };

        let compressed = self.response_compressor.compress(&body_val);
        if compressed != body_val {
            let new_body = serde_json::to_vec(&compressed)
                .map_err(|e| ProxyError::BadRequest(format!("serialize error: {e}")))?;
            let post_tokens = token_counter::count(&new_body);

            debug!(
                pre = pre_tokens,
                post = post_tokens,
                saved = pre_tokens.saturating_sub(post_tokens),
                "compress: response compression"
            );

            res.body = Bytes::from(new_body);

            if let Ok(mut guard) = self.response_stats.lock() {
                *guard = Some(ResponseStats {
                    pre_tokens,
                    post_tokens,
                });
            }
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        "compress"
    }
}

// ── Debug log helpers ──────────────────────────────────────────────────────

/// Build an abbreviated snapshot of tool names + descriptions for debug logging.
fn abbreviate_tools_for_log(tools: &[serde_json::Value]) -> String {
    let max_desc_len = 400;
    let mut items: Vec<serde_json::Value> = Vec::with_capacity(tools.len());

    for tool in tools {
        let name = tool.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let desc = tool
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let desc_short: String = if desc.len() > max_desc_len {
            format!("{}...({} chars)", &desc[..max_desc_len], desc.len())
        } else {
            desc.to_string()
        };
        let prop_count = tool
            .pointer("/input_schema/properties")
            .and_then(|v| v.as_object())
            .map_or(0, serde_json::Map::len);
        items.push(serde_json::json!({
            "name": name,
            "desc": desc_short,
            "props": prop_count,
        }));
    }

    serde_json::to_string(&items).unwrap_or_default()
}

/// Write schema compression before/after to a debug log file.
///
/// Log file: `~/.tokenfleet-ai/agent-proxy/schema-compress-debug.log` (JSON Lines).
/// Truncated at 200 KB to prevent unbounded growth.
fn write_schema_debug_log(
    tools_count: usize,
    pre_tokens: u64,
    post_tokens: u64,
    saved: u64,
    before_snapshot: &str,
    after_snapshot: &str,
) {
    #[allow(clippy::disallowed_methods)]
    let Some(home) = dirs::home_dir() else {
        return;
    };
    let log_dir = home.join(".tokenfleet-ai").join("agent-proxy");
    #[allow(clippy::disallowed_methods)]
    if std::fs::create_dir_all(&log_dir).is_err() {
        return;
    }
    let log_path = log_dir.join("schema-compress-debug.log");

    // Truncate if over 200 KB
    #[allow(clippy::disallowed_methods)]
    if let Ok(meta) = std::fs::metadata(&log_path)
        && meta.len() > 200_000
    {
        let _ = std::fs::remove_file(&log_path);
    }

    let entry = serde_json::json!({
        "ts": chrono::Utc::now().to_rfc3339(),
        "tools_count": tools_count,
        "pre_tokens": pre_tokens,
        "post_tokens": post_tokens,
        "saved": saved,
        "before": before_snapshot,
        "after": after_snapshot,
    });

    #[allow(clippy::disallowed_methods, clippy::disallowed_types)]
    let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    else {
        return;
    };
    #[allow(clippy::disallowed_methods)]
    let _ = writeln!(f, "{}", serde_json::to_string(&entry).unwrap_or_default());
}
