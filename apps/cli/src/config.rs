//! Configuration loading with three-layer merge.
//!
//! Priority (highest first):
//! 1. CLI flags
//! 2. Environment variables (`AGENT_PROXY_` prefix)
//! 3. YAML configuration file (auto-discovered)
//!
//! `proxy_secret` is read from `AGENT_PROXY_PROXY_SECRET` env var only —
//! never from config files.

use std::{fmt, net::SocketAddr, path::PathBuf};

use config::{Config, Environment, File};
use serde::Deserialize;

/// CLI arguments for the `serve` subcommand.
///
/// Every field is `Option<T>` — when `None`, the value comes from env or config file.
#[derive(clap::Args, Debug)]
pub struct ServeArgs {
    /// Path to YAML configuration file (skips auto-discovery).
    #[arg(short = 'c', long = "config")]
    pub config_path: Option<String>,

    /// Listen address for the HTTP server (e.g., 127.0.0.1:8787).
    #[arg(short = 'l', long = "listen", env = "AGENT_PROXY_LISTEN")]
    pub listen: Option<String>,

    /// Data directory for storage and logs.
    #[arg(short = 'd', long = "data-dir", env = "AGENT_PROXY_DATA_DIR")]
    pub data_dir: Option<String>,

    /// Maximum request body size in bytes (1 KB – 64 MB).
    #[arg(long = "max-body-size", env = "AGENT_PROXY_MAX_BODY_SIZE")]
    pub max_body_size: Option<usize>,

    /// Upstream per-read timeout in seconds (1–3600).
    #[arg(
        long = "upstream-read-timeout",
        env = "AGENT_PROXY_UPSTREAM_READ_TIMEOUT"
    )]
    pub upstream_read_timeout: Option<u64>,

    /// Upstream connect timeout in seconds (1–300).
    #[arg(
        long = "upstream-connect-timeout",
        env = "AGENT_PROXY_UPSTREAM_CONNECT_TIMEOUT"
    )]
    pub upstream_connect_timeout: Option<u64>,

    /// Proxy-level API key for authenticating clients.
    #[arg(long = "proxy-api-key", env = "AGENT_PROXY_API_KEY")]
    pub proxy_api_key: Option<String>,

    /// Log output format: `pretty` or `json`.
    #[arg(long = "log-format", env = "AGENT_PROXY_LOG_FORMAT")]
    pub log_format: Option<String>,

    /// Disable token compression middleware.
    #[arg(long = "disable-compression")]
    pub disable_compression: bool,

    /// Disable protocol bridge middleware.
    #[arg(long = "disable-bridge")]
    pub disable_bridge: bool,

    /// Disable cost tracking middleware.
    #[arg(long = "disable-cost-tracking")]
    pub disable_cost_tracking: bool,
}

// ── Config structs with serde + defaults ────────────────────────

/// Complete proxy configuration after three-layer merge.
///
/// Fields are consumed by the axum server engine.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct ProxyConfig {
    /// Listen address.
    pub listen: SocketAddr,

    /// Data directory path.
    pub data_dir: PathBuf,

    /// Maximum request body size in bytes.
    pub max_body_size: usize,

    /// Upstream per-read timeout in seconds.
    pub upstream_read_timeout: u64,

    /// Upstream connect timeout in seconds.
    pub upstream_connect_timeout: u64,

    /// Proxy-level API key (optional).
    pub proxy_api_key: Option<String>,

    /// Proxy secret — required, env-only (never read from YAML).
    #[serde(skip)]
    pub proxy_secret: String,

    /// Log output format.
    #[serde(default)]
    pub log_format: LogFormat,

    /// Compression middleware config.
    #[serde(default)]
    pub compress: CompressConfig,

    /// Bridge middleware config.
    #[serde(default)]
    pub bridge: BridgeConfig,

    /// Rate limiting config.
    #[serde(default)]
    pub rate_limit: RateLimitConfig,

    /// Cost tracking config.
    #[serde(default)]
    pub cost: CostConfig,

    /// TLS config (both cert and key must be set to enable).
    #[serde(default)]
    pub tls: Option<TlsConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    Pretty,
    Json,
}

impl Default for LogFormat {
    fn default() -> Self {
        Self::Pretty
    }
}

impl fmt::Display for LogFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pretty => f.write_str("pretty"),
            Self::Json => f.write_str("json"),
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct CompressConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_min_schema_size")]
    pub min_schema_size: usize,
}

impl Default for CompressConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_schema_size: 512,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct BridgeConfig {
    #[serde(default = "default_max_conversion_size")]
    pub max_conversion_body_size: usize,
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            max_conversion_body_size: 1_048_576, // 1 MB
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct RateLimitConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_requests_per_second")]
    pub requests_per_second: u32,
    #[serde(default = "default_burst_size")]
    pub burst_size: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            requests_per_second: 50,
            burst_size: 100,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct CostConfig {
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
}

impl Default for CostConfig {
    fn default() -> Self {
        Self { retention_days: 90 }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct TlsConfig {
    pub cert: PathBuf,
    pub key: PathBuf,
}

// ── Default helper fns (const fn not available for these types) ──

const fn default_true() -> bool {
    true
}
const fn default_min_schema_size() -> usize {
    512
}
const fn default_max_conversion_size() -> usize {
    1_048_576
}
const fn default_requests_per_second() -> u32 {
    50
}
const fn default_burst_size() -> u32 {
    100
}
const fn default_retention_days() -> u32 {
    90
}

// ── Public API ──────────────────────────────────────────────────

/// Load configuration with the three-layer merge strategy.
///
/// # Errors
///
/// Returns an error if:
/// - The YAML config file is invalid or unreadable (when explicitly specified)
/// - `proxy_secret` env var is missing or empty
/// - Validation fails (all errors collected)
pub fn load_config(args: &ServeArgs) -> Result<ProxyConfig, anyhow::Error> {
    // 1. Build config from: defaults → YAML → env
    let mut builder = Config::builder();

    // Defaults
    builder = builder
        .set_default("listen", "127.0.0.1:8787")?
        .set_default("max_body_size", 16_777_216_i64)?
        .set_default("upstream_read_timeout", 600_i64)?
        .set_default("upstream_connect_timeout", 10_i64)?;

    // YAML file (auto-discovery or explicit)
    let config_file_path = discover_config_file(args.config_path.as_deref());
    if let Some(ref path) = config_file_path {
        builder = builder.add_source(File::from(path.as_ref()).required(false));
    }

    // Environment variables (override YAML)
    builder = builder.add_source(Environment::with_prefix("AGENT_PROXY").separator("__"));

    // Build
    let cfg: config::Config = builder.build()?;

    // Deserialize to our struct
    let mut config: ProxyConfig = cfg.try_deserialize()?;

    // Set default data_dir if none configured
    if config.data_dir.as_os_str().is_empty() {
        config.data_dir = default_data_dir();
    }

    // 2. Apply CLI overrides (highest priority)
    apply_cli_overrides(&mut config, args);

    // 3. Read proxy_secret from env (never from config file)
    let proxy_secret = std::env::var("AGENT_PROXY_PROXY_SECRET").unwrap_or_default();
    config.proxy_secret = proxy_secret;

    // 4. Validate (collects all errors)
    validate(&config)?;

    Ok(config)
}

// ── Internals ───────────────────────────────────────────────────

/// Discover the YAML config file path, checking these locations in order:
/// 1. `--config` CLI flag
/// 2. `AGENT_PROXY_CONFIG` env var
/// 3. `{default_data_dir}/config.yaml`
/// 4. `~/.config/agent-proxy/config.yaml`
/// 5. `./agent-proxy.yaml`
fn discover_config_file(explicit_path: Option<&str>) -> Option<PathBuf> {
    // Priority 1: explicit --config flag
    if let Some(path) = explicit_path {
        return Some(PathBuf::from(path));
    }

    // Priority 2: AGENT_PROXY_CONFIG env var
    if let Ok(path) = std::env::var("AGENT_PROXY_CONFIG") {
        return Some(PathBuf::from(path));
    }

    // Priority 3–5: auto-discovery (first existing file wins)
    let candidates: [PathBuf; 3] = [
        default_data_dir().join("config.yaml"),
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join("agent-proxy")
            .join("config.yaml"),
        PathBuf::from("./agent-proxy.yaml"),
    ];

    for candidate in &candidates {
        if candidate.exists() {
            return Some(candidate.clone());
        }
    }

    None
}

/// Apply CLI flag overrides on top of the already-merged config.
fn apply_cli_overrides(config: &mut ProxyConfig, args: &ServeArgs) {
    if let Some(ref listen) = args.listen
        && let Ok(addr) = listen.parse::<SocketAddr>()
    {
        config.listen = addr;
    }

    if let Some(ref dir) = args.data_dir {
        config.data_dir = PathBuf::from(dir);
    }

    if let Some(size) = args.max_body_size {
        config.max_body_size = size;
    }

    if let Some(timeout) = args.upstream_read_timeout {
        config.upstream_read_timeout = timeout;
    }

    if let Some(timeout) = args.upstream_connect_timeout {
        config.upstream_connect_timeout = timeout;
    }

    if args.proxy_api_key.is_some() {
        config.proxy_api_key.clone_from(&args.proxy_api_key);
    }

    if let Some(ref fmt) = args.log_format {
        config.log_format = match fmt.to_lowercase().as_str() {
            "json" => LogFormat::Json,
            _ => LogFormat::Pretty,
        };
    }

    if args.disable_compression {
        config.compress.enabled = false;
    }
}

/// Validate the merged configuration, collecting all errors.
fn validate(config: &ProxyConfig) -> Result<(), anyhow::Error> {
    let mut errors: Vec<String> = Vec::new();

    // proxy_secret must be non-empty (env only)
    if config.proxy_secret.is_empty() {
        errors.push("AGENT_PROXY_PROXY_SECRET must be set (env-only, non-empty)".into());
    }

    // max_body_size: 1 KB – 64 MB
    if config.max_body_size < 1024 || config.max_body_size > 67_108_864 {
        errors.push(format!(
            "max_body_size ({}) must be between 1024 (1 KB) and 67108864 (64 MB)",
            config.max_body_size
        ));
    }

    // upstream_read_timeout: 1s – 3600s
    if config.upstream_read_timeout < 1 || config.upstream_read_timeout > 3600 {
        errors.push(format!(
            "upstream_read_timeout ({}) must be between 1 and 3600 seconds",
            config.upstream_read_timeout
        ));
    }

    // upstream_connect_timeout: 1s – 300s
    if config.upstream_connect_timeout < 1 || config.upstream_connect_timeout > 300 {
        errors.push(format!(
            "upstream_connect_timeout ({}) must be between 1 and 300 seconds",
            config.upstream_connect_timeout
        ));
    }

    // rate_limit.requests_per_second must be >= 1
    if config.rate_limit.requests_per_second < 1 {
        errors.push(format!(
            "rate_limit.requests_per_second ({}) must be >= 1",
            config.rate_limit.requests_per_second
        ));
    }

    // TLS requires both cert and key if either is set
    if let Some(ref tls) = config.tls
        && (tls.cert.as_os_str().is_empty() || tls.key.as_os_str().is_empty())
    {
        errors.push("TLS requires both cert and key to be set".into());
    }

    if !errors.is_empty() {
        let message = errors
            .iter()
            .enumerate()
            .map(|(i, e)| format!("  {}. {e}", i + 1))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(anyhow::anyhow!(
            "configuration validation failed:\n{message}",
        ));
    }

    Ok(())
}

/// Default data directory (OS-specific).
fn default_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("agent-proxy")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    // ── Validation tests ────────────────────────────────────

    fn valid_config() -> ProxyConfig {
        ProxyConfig {
            listen: "127.0.0.1:8787".parse().expect("valid socket addr"),
            data_dir: PathBuf::from("/tmp/ap"),
            max_body_size: 16_777_216,
            upstream_read_timeout: 600,
            upstream_connect_timeout: 10,
            proxy_api_key: None,
            proxy_secret: "shh".into(),
            log_format: LogFormat::Pretty,
            compress: CompressConfig::default(),
            bridge: BridgeConfig::default(),
            rate_limit: RateLimitConfig::default(),
            cost: CostConfig::default(),
            tls: None,
        }
    }

    #[test]
    fn test_should_pass_validation_with_valid_config() {
        assert!(validate(&valid_config()).is_ok());
    }

    #[test]
    fn test_should_reject_empty_proxy_secret() {
        let mut cfg = valid_config();
        cfg.proxy_secret = String::new();
        let err = validate(&cfg).unwrap_err().to_string();
        assert!(err.contains("PROXY_SECRET"));
    }

    #[test]
    fn test_should_reject_max_body_size_too_small() {
        let mut cfg = valid_config();
        cfg.max_body_size = 512;
        let err = validate(&cfg).unwrap_err().to_string();
        assert!(err.contains("max_body_size"));
    }

    #[test]
    fn test_should_reject_max_body_size_too_large() {
        let mut cfg = valid_config();
        cfg.max_body_size = 100_000_000;
        let err = validate(&cfg).unwrap_err().to_string();
        assert!(err.contains("max_body_size"));
    }

    #[test]
    fn test_should_reject_upstream_read_timeout_too_low() {
        let mut cfg = valid_config();
        cfg.upstream_read_timeout = 0;
        let err = validate(&cfg).unwrap_err().to_string();
        assert!(err.contains("upstream_read_timeout"));
    }

    #[test]
    fn test_should_reject_upstream_read_timeout_too_high() {
        let mut cfg = valid_config();
        cfg.upstream_read_timeout = 3601;
        let err = validate(&cfg).unwrap_err().to_string();
        assert!(err.contains("upstream_read_timeout"));
    }

    #[test]
    fn test_should_reject_connect_timeout_too_low() {
        let mut cfg = valid_config();
        cfg.upstream_connect_timeout = 0;
        let err = validate(&cfg).unwrap_err().to_string();
        assert!(err.contains("upstream_connect_timeout"));
    }

    #[test]
    fn test_should_reject_rate_limit_zero() {
        let mut cfg = valid_config();
        cfg.rate_limit.requests_per_second = 0;
        let err = validate(&cfg).unwrap_err().to_string();
        assert!(err.contains("rate_limit.requests_per_second"));
    }

    #[test]
    fn test_should_reject_tls_without_cert() {
        let mut cfg = valid_config();
        cfg.tls = Some(TlsConfig {
            cert: PathBuf::new(),
            key: PathBuf::from("/key.pem"),
        });
        let err = validate(&cfg).unwrap_err().to_string();
        assert!(err.contains("TLS"));
    }

    #[test]
    fn test_should_collect_all_errors_at_once() {
        let mut cfg = valid_config();
        cfg.proxy_secret = String::new();
        cfg.max_body_size = 100;
        cfg.upstream_read_timeout = 0;
        let err = validate(&cfg).unwrap_err().to_string();
        assert!(err.contains("PROXY_SECRET"));
        assert!(err.contains("max_body_size"));
        assert!(err.contains("upstream_read_timeout"));
    }

    // ── Default values ──────────────────────────────────────

    #[test]
    fn test_compress_config_defaults() {
        let c = CompressConfig::default();
        assert!(c.enabled);
        assert_eq!(c.min_schema_size, 512);
    }

    #[test]
    fn test_bridge_config_defaults() {
        let c = BridgeConfig::default();
        assert_eq!(c.max_conversion_body_size, 1_048_576);
    }

    #[test]
    fn test_rate_limit_config_defaults() {
        let c = RateLimitConfig::default();
        assert!(c.enabled);
        assert_eq!(c.requests_per_second, 50);
        assert_eq!(c.burst_size, 100);
    }

    #[test]
    fn test_cost_config_defaults() {
        let c = CostConfig::default();
        assert_eq!(c.retention_days, 90);
    }
}
