//! Compression statistics tracking across three layers.
//!
//! # Layers
//!
//! - **tokenless** (external hook/rewrite): reported via `TOKENLESS_TOKENS` env var
//! - **compress middleware** (agent-proxy): `SchemaCompressor` + `ResponseCompressor`
//! - **upstream** (API response): usage fields extracted by cost module
//!
//! # Data flow
//!
//! ```text
//! tokenless env var → parse → CompressionStats → ctx.extensions
//!   → compress middleware appends schema/response stats
//!   → cost module reads to compute saved_cost
//! ```

/// Complete multi-layer compression statistics.
#[derive(Debug, Clone, Default)]
pub struct CompressionStats {
    // ── Layer 1: tokenless (from TOKENLESS_TOKENS env var) ──
    /// Request token count before tokenless compression.
    pub tokenless_pre: u64,
    /// Request token count after tokenless compression.
    pub tokenless_post: u64,
    /// Additional tokens saved by tokenless experimental mode.
    pub tokenless_experimental: u64,

    // ── Layer 2: compress middleware ──
    /// Request body token count before agent-proxy schema compression.
    pub proxy_req_pre: u64,
    /// Request body token count after agent-proxy schema compression.
    pub proxy_req_post: u64,
    /// Response body token count before agent-proxy response compression.
    pub proxy_res_pre: u64,
    /// Response body token count after agent-proxy response compression.
    pub proxy_res_post: u64,

    // ── External: RTK (from x-rtk-tokens env var) ──
    /// Tokens saved by RTK rewrite before the request reaches agent-proxy.
    pub rtk_saved: u64,
}

impl CompressionStats {
    /// Total tokens saved at the tokenless layer.
    #[must_use]
    pub fn tokenless_saved(&self) -> u64 {
        self.tokenless_pre.saturating_sub(self.tokenless_post)
    }

    /// Tokens saved by agent-proxy schema compression.
    #[must_use]
    pub fn proxy_schema_saved(&self) -> u64 {
        self.proxy_req_pre.saturating_sub(self.proxy_req_post)
    }

    /// Tokens saved by agent-proxy response compression.
    #[must_use]
    pub fn proxy_response_saved(&self) -> u64 {
        self.proxy_res_pre.saturating_sub(self.proxy_res_post)
    }

    /// Total tokens saved across all layers.
    #[must_use]
    pub fn total_saved(&self) -> u64 {
        self.tokenless_saved()
            + self.tokenless_experimental
            + self.proxy_schema_saved()
            + self.proxy_response_saved()
            + self.rtk_saved
    }
}

/// Parses the `TOKENLESS_TOKENS` environment variable into [`CompressionStats`].
///
/// Expected format: `{"pre":800,"post":300,"experimental":150}`
/// Returns [`CompressionStats::default()`] if the env var is not set or invalid.
#[must_use]
pub fn read_tokenless_stats() -> CompressionStats {
    let value = match std::env::var("TOKENLESS_TOKENS") {
        Ok(v) => v,
        Err(_) => return CompressionStats::default(),
    };
    if value.is_empty() {
        return CompressionStats::default();
    }
    parse_tokenless_json(&value).unwrap_or_default()
}

fn parse_tokenless_json(json: &str) -> Option<CompressionStats> {
    #[derive(serde::Deserialize)]
    struct TokenlessHeader {
        pre: Option<u64>,
        post: Option<u64>,
        experimental: Option<u64>,
    }
    let h: TokenlessHeader = serde_json::from_str(json).ok()?;
    Some(CompressionStats {
        tokenless_pre: h.pre.unwrap_or(0),
        tokenless_post: h.post.unwrap_or(0),
        tokenless_experimental: h.experimental.unwrap_or(0),
        ..Default::default()
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_zero() {
        let s = CompressionStats::default();
        assert_eq!(s.tokenless_pre, 0);
        assert_eq!(s.total_saved(), 0);
    }

    #[test]
    fn test_tokenless_saved() {
        let s = CompressionStats {
            tokenless_pre: 800,
            tokenless_post: 300,
            ..Default::default()
        };
        assert_eq!(s.tokenless_saved(), 500);
    }

    #[test]
    fn test_saturating_sub_prevents_underflow() {
        let s = CompressionStats {
            tokenless_pre: 100,
            tokenless_post: 200,
            ..Default::default()
        };
        assert_eq!(s.tokenless_saved(), 0);
    }

    #[test]
    fn test_total_saved_sums_all_layers() {
        let s = CompressionStats {
            tokenless_pre: 800,
            tokenless_post: 300,         // saved 500
            tokenless_experimental: 150, // extra 150
            proxy_req_pre: 300,
            proxy_req_post: 220, // saved 80
            proxy_res_pre: 1200,
            proxy_res_post: 800, // saved 400
            rtk_saved: 0,
        };
        // 500 + 150 + 80 + 400 + 0 = 1130
        assert_eq!(s.total_saved(), 1130);
    }

    #[test]
    fn test_parse_valid_json() {
        let stats = parse_tokenless_json(r#"{"pre":800,"post":300,"experimental":150}"#).unwrap();
        assert_eq!(stats.tokenless_pre, 800);
        assert_eq!(stats.tokenless_post, 300);
        assert_eq!(stats.tokenless_experimental, 150);
    }

    #[test]
    fn test_parse_partial_json() {
        let stats = parse_tokenless_json(r#"{"pre":500}"#).unwrap();
        assert_eq!(stats.tokenless_pre, 500);
        assert_eq!(stats.tokenless_post, 0);
        assert_eq!(stats.tokenless_experimental, 0);
    }

    #[test]
    fn test_parse_invalid_json_returns_none() {
        assert!(parse_tokenless_json("not json").is_none());
        assert!(parse_tokenless_json("").is_none());
    }

    #[test]
    fn test_parse_missing_all_fields_returns_default() {
        let stats = parse_tokenless_json(r"{}").unwrap();
        assert_eq!(stats.tokenless_pre, 0);
    }

    #[test]
    fn test_proxy_schema_saved() {
        let s = CompressionStats {
            proxy_req_pre: 500,
            proxy_req_post: 350,
            ..Default::default()
        };
        assert_eq!(s.proxy_schema_saved(), 150);
    }

    #[test]
    fn test_proxy_response_saved() {
        let s = CompressionStats {
            proxy_res_pre: 2000,
            proxy_res_post: 1200,
            ..Default::default()
        };
        assert_eq!(s.proxy_response_saved(), 800);
    }
}
