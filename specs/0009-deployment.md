# 0009 — Deployment

> **Phase 1**: `cargo install agent-proxy && agent-proxy serve`. Pretty console logging. Simple health endpoint.
> **Phase 2**: Docker image, systemd unit, launchd plist, Prometheus metrics, JSON structured logging.

## Overview

The proxy runs as a long-lived service. It supports multiple deployment modes: standalone binary (dev), systemd/launchd (server), Docker (containerized), and sidecar (per-machine agent companion).

## Deployment Modes

| Mode | Use Case | Process Model |
|------|----------|--------------|
| Standalone | Development, single-user | Foreground, Ctrl-C to stop |
| Systemd | Linux server | Background, socket activation |
| Launchd | macOS server | Background, plist-managed |
| Docker | Containerized | Single process in container |
| Sidecar | Per-developer machine | Launched alongside IDE/agent |

## Standalone (CLI Binary)

```bash
# Install
cargo install agent-proxy

# Run
agent-proxy serve --config ~/.config/agent-proxy/config.yaml

# Or with env vars
AGENT_PROXY_LISTEN=127.0.0.1:8787 PROXY_SECRET=xxx agent-proxy serve
```

## Systemd

```ini
# /etc/systemd/system/agent-proxy.service
[Unit]
Description=Agent Proxy - AI API Middleware
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=agent-proxy
Group=agent-proxy
EnvironmentFile=/etc/agent-proxy/env
ExecStart=/usr/local/bin/agent-proxy serve --config /etc/agent-proxy/config.yaml
Restart=on-failure
RestartSec=5s
LimitNOFILE=65536
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/agent-proxy
NoNewPrivileges=true

[Install]
WantedBy=multi-user.target
```

Key systemd hardening: `ProtectSystem=strict`, `ProtectHome=true`, `NoNewPrivileges=true`. Only the data directory (`/var/lib/agent-proxy`) is writable.

### Socket Activation

Optional — systemd can bind the port and pass the socket fd:

```ini
# /etc/systemd/system/agent-proxy.socket
[Socket]
ListenStream=8787
BindIPv6Only=both
NoDelay=true

[Install]
WantedBy=sockets.target
```

The proxy detects `LISTEN_FDS=1` and uses the inherited socket instead of binding.

## Launchd (macOS)

```xml
<!-- ~/Library/LaunchAgents/com.agent-proxy.plist -->
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.agent-proxy</string>
    <key>ProgramArguments</key>
    <array>
        <string>/usr/local/bin/agent-proxy</string>
        <string>serve</string>
        <string>--config</string>
        <string>/Users/Shared/agent-proxy/config.yaml</string>
    </array>
    <key>EnvironmentVariables</key>
    <dict>
        <key>PROXY_SECRET</key>
        <string>replace-me</string>
    </dict>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/usr/local/var/log/agent-proxy/stdout.log</string>
    <key>StandardErrorPath</key>
    <string>/usr/local/var/log/agent-proxy/stderr.log</string>
</dict>
</plist>
```

Load with: `launchctl load ~/Library/LaunchAgents/com.agent-proxy.plist`

## Docker

```dockerfile
# Dockerfile
FROM rust:1.91-slim-bookworm AS builder
WORKDIR /app
COPY . .
RUN cargo build --release --bin agent-proxy

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/agent-proxy /usr/local/bin/
USER 1000:1000
EXPOSE 8787
VOLUME /var/lib/agent-proxy
ENTRYPOINT ["agent-proxy", "serve", "--config", "/etc/agent-proxy/config.yaml"]
```

```yaml
# docker-compose.yml
services:
  agent-proxy:
    build: .
    ports:
      - "127.0.0.1:8787:8787"
    environment:
      - PROXY_SECRET=${PROXY_SECRET}
    volumes:
      - ./config.yaml:/etc/agent-proxy/config.yaml:ro
      - agent-proxy-data:/var/lib/agent-proxy
    restart: unless-stopped

volumes:
  agent-proxy-data:
```

## Data Directory

The proxy needs a writable directory for SQLite data. Default locations:

| OS | Path |
|----|------|
| Linux | `$XDG_DATA_HOME/agent-proxy/` or `~/.local/share/agent-proxy/` |
| macOS | `~/Library/Application Support/com.agent-proxy/` |
| Windows | `%APPDATA%\agent-proxy\` |
| Docker | `/var/lib/agent-proxy/` |

Override: `--data-dir` flag or `AGENT_PROXY_DATA_DIR` env var.

Contents:
```
{data_dir}/
├── agent-proxy.db        # SQLite: channels, model_mappings, cost_records
├── agent-proxy.db-wal    # WAL journal
├── agent-proxy.db-shm    # WAL shared memory
└── config.yaml           # Optional: generated from env if not provided
```

## Logging

Uses `tracing` with two output modes:

```rust
enum LogFormat {
    /// Human-readable, colored (default in dev).
    Pretty,
    /// JSON structured (default in production / systemd).
    Json,
}
```

```bash
# JSON logging (systemd journal)
AGENT_PROXY_LOG_FORMAT=json agent-proxy serve

# Pretty logging (development)
AGENT_PROXY_LOG_FORMAT=pretty agent-proxy serve
```

Log levels via `RUST_LOG` env var (standard `tracing-subscriber` env filter):

```bash
RUST_LOG=agent_proxy=debug,info agent-proxy serve
```

### What Gets Logged

| Level | Content |
|-------|---------|
| `ERROR` | Upstream failures, DB errors, startup failures |
| `WARN` | Deprecated model usage, quota exhaustion, channel degraded |
| `INFO` | Startup, shutdown, channel health transitions, periodic stats |
| `DEBUG` | Request routing decisions, protocol conversion details |
| `TRACE` | Full request/response bodies (disabled in production) |

API keys and secrets are NEVER logged — they are wrapped in `secrecy::SecretString` which redacts via `Debug`.

## Metrics

`GET /metrics` exposes Prometheus-format metrics:

```prometheus
# HELP agent_proxy_requests_total Total requests handled
# TYPE agent_proxy_requests_total counter
agent_proxy_requests_total{channel="anthropic-official",model="claude-sonnet",status="200"} 1523

# HELP agent_proxy_request_duration_seconds Request duration histogram
# TYPE agent_proxy_request_duration_seconds histogram
agent_proxy_request_duration_seconds_bucket{channel="anthropic-official",le="0.5"} 1200
agent_proxy_request_duration_seconds_bucket{channel="anthropic-official",le="1.0"} 1450

# HELP agent_proxy_channel_health Channel health status (1=healthy, 0=unhealthy)
# TYPE agent_proxy_channel_health gauge
agent_proxy_channel_health{channel="anthropic-official"} 1
agent_proxy_channel_health{channel="dashscope"} 0

# HELP agent_proxy_tokens_total Total tokens processed
# TYPE agent_proxy_tokens_total counter
agent_proxy_tokens_total{direction="input"} 5000000
agent_proxy_tokens_total{direction="output"} 1200000

# HELP agent_proxy_compression_savings_ratio Compression token savings ratio
# TYPE agent_proxy_compression_savings_ratio gauge
agent_proxy_compression_savings_ratio 0.42
```

## Health Check

`GET /health` returns:

```json
{
  "status": "ok",
  "uptime_seconds": 86400,
  "channels_total": 12,
  "channels_healthy": 10,
  "db_connected": true
}
```

Returns `503 Service Unavailable` if the database is unreachable or all channels are unhealthy.

## Resource Limits

| Resource | Default | Flag |
|----------|---------|------|
| Max body size | 16 MB | `--max-body-size` |
| Upstream timeout | 30s | `--upstream-timeout` |
| Upstream connect timeout | 10s | `--upstream-connect-timeout` |
| Max concurrent connections | 512 | `--max-connections` |
| Max open files | inherited | systemd `LimitNOFILE` |
