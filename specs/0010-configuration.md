# 0010 — Configuration

> **Phase 1**: clap CLI flags only. `--config`, `--listen`, `--data-dir`, `--proxy-api-key`. No env vars, no YAML merge.
> **Phase 2**: `config` crate with layered sources (CLI > env > YAML), auto-discovery, nested env var expansion.

## Overview

Configuration merges from three sources in priority order (highest first):

1. CLI flags
2. Environment variables
3. Configuration file (YAML)

## CLI Flags

```
agent-proxy serve [OPTIONS]

OPTIONS:
  -c, --config <PATH>            Path to config file [default: auto-detect]
  -l, --listen <ADDR>            Listen address [env: AGENT_PROXY_LISTEN]
  -d, --data-dir <PATH>          Data directory [env: AGENT_PROXY_DATA_DIR]
      --max-body-size <BYTES>    Max request body size [env: AGENT_PROXY_MAX_BODY_SIZE]
      --upstream-timeout <SECS>  Upstream request timeout [env: AGENT_PROXY_UPSTREAM_TIMEOUT]
      --upstream-connect-timeout <SECS> Upstream connect timeout [env: AGENT_PROXY_UPSTREAM_CONNECT_TIMEOUT]
      --proxy-api-key <KEY>      Proxy auth key [env: AGENT_PROXY_API_KEY]
      --log-format <FORMAT>      Log format: pretty, json [env: AGENT_PROXY_LOG_FORMAT]
      --disable-compression      Disable token compression middleware
      --disable-bridge           Disable protocol bridge middleware
      --disable-cost-tracking    Disable cost tracking
```

## Environment Variables

All env vars are prefixed with `AGENT_PROXY_`. Nested config keys use `__` (double underscore) as separator.

| Env Var | Config Key | Type | Default |
|---------|-----------|------|---------|
| `AGENT_PROXY_LISTEN` | `listen` | SocketAddr | `127.0.0.1:8787` |
| `AGENT_PROXY_DATA_DIR` | `data_dir` | PathBuf | OS-specific |
| `AGENT_PROXY_MAX_BODY_SIZE` | `max_body_size` | usize | `16777216` |
| `AGENT_PROXY_UPSTREAM_TIMEOUT` | `upstream_timeout` | u64 (secs) | `30` |
| `AGENT_PROXY_UPSTREAM_CONNECT_TIMEOUT` | `upstream_connect_timeout` | u64 (secs) | `10` |
| `AGENT_PROXY_PROXY_API_KEY` | `proxy_api_key` | String | None |
| `AGENT_PROXY_LOG_FORMAT` | `log_format` | String | `pretty` |
| `AGENT_PROXY_COMPRESS__ENABLED` | `compress.enabled` | bool | `true` |
| `AGENT_PROXY_COMPRESS__MIN_SCHEMA_SIZE` | `compress.min_schema_size` | usize | `512` |
| `AGENT_PROXY_BRIDGE__MAX_CONVERSION_SIZE` | `bridge.max_conversion_body_size` | usize | `1048576` |
| `AGENT_PROXY_RATE_LIMIT__REQUESTS_PER_SEC` | `rate_limit.requests_per_second` | u32 | `50` |
| `AGENT_PROXY_RATE_LIMIT__BURST_SIZE` | `rate_limit.burst_size` | u32 | `100` |
| `AGENT_PROXY_COST__RETENTION_DAYS` | `cost.retention_days` | u32 | `90` |
| `AGENT_PROXY_PROXY_SECRET` | (not in config) | String | **required** |
| `RUST_LOG` | (standard) | String | `info` |

`PROXY_SECRET` must be set as an environment variable — it is NEVER read from config files (avoids committing secrets).

## Configuration File (YAML)

Auto-detected from:
1. `--config` flag
2. `AGENT_PROXY_CONFIG` env var
3. `{data_dir}/config.yaml`
4. `~/.config/agent-proxy/config.yaml`
5. `./agent-proxy.yaml` (current directory)

```yaml
# agent-proxy config
# All values shown are defaults.

listen: "127.0.0.1:8787"

max_body_size: 16777216  # 16 MB
upstream_timeout: 30
upstream_connect_timeout: 10

log_format: "pretty"  # pretty | json

# Optional proxy-level authentication
proxy_api_key: null

compress:
  enabled: true
  min_schema_size: 512
  compress_responses: true

bridge:
  max_conversion_body_size: 1048576  # 1 MB
  log_warnings: true

rate_limit:
  enabled: true
  requests_per_second: 50
  burst_size: 100

cost:
  retention_days: 90

# TLS (optional — both must be set to enable)
tls:
  cert: null    # /path/to/cert.pem
  key: null     # /path/to/key.pem
```

## Merge Rules

```
CLI flag
  └── overrides ──→ Environment variable
                      └── overrides ──→ Config file value
                                          └── overrides ──→ Default
```

Example:

```bash
# Config file says listen: "127.0.0.1:8787"
# Env var sets AGENT_PROXY_LISTEN=0.0.0.0:9090
# Result: 0.0.0.0:9090 (env wins over file)

# CLI flag --listen 0.0.0.0:8080
# Result: 0.0.0.0:8080 (CLI wins over env and file)
```

Booleans in env vars accept: `true`, `false`, `1`, `0`.

## Config Validation

On startup, the merged config is validated:

- `listen` must be a valid `SocketAddr`.
- `max_body_size` must be between 1 KB and 64 MB.
- `upstream_timeout` must be between 1s and 300s.
- `rate_limit.requests_per_second` must be >= 1.
- `tls` requires both `cert` and `key` if either is set.
- `proxy_secret` (env only) must be non-empty.

Validation errors are fatal — the proxy refuses to start and prints all validation errors at once.

## Implementation

Uses the `config` crate with layered sources:

```rust
use config::{Config, Environment, File};

fn load_config(cli: CliArgs) -> Result<ProxyConfig> {
    let mut builder = Config::builder()
        .add_source(File::with_name("config").required(false))       // ./config.yaml
        .add_source(File::with_name(&data_config_path()).required(false))  // ~/.config/...
        .add_source(Environment::with_prefix("AGENT_PROXY").separator("__"));

    if let Some(path) = cli.config {
        builder = builder.add_source(File::with_name(&path).required(true));
    }

    let config: ProxyConfig = builder.build()?.try_deserialize()?;
    config.validate()?;
    Ok(config)
}
```
