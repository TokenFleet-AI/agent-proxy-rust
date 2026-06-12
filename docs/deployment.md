# 部署运维指南

本文档描述如何将 agent-proxy-rust 部署到生产环境，覆盖安装、系统服务配置、健康检查、日志、备份和故障排查。

## 安装方式

### 从源码编译

```bash
# 克隆仓库
git clone https://github.com/TokenFleet-AI/agent-proxy-rust.git
cd agent-proxy-rust

# Makefile 构建
make build

# 或直接 cargo install
cargo install --path apps/server
```

编译产物位于 `target/release/agent-proxy-rust-server`。

### 预编译二进制（计划中）

> **Phase 2 计划**：GitHub Releases 提供预编译二进制，支持 macOS ARM/Intel、Linux musl（x86_64/aarch64）、Windows。当前版本需从源码编译。

## 环境变量

| 变量 | 必填 | 默认值 | 说明 |
|------|:----:|--------|------|
| `PROXY_SECRET` | 是 | — | 加密存储 API Key 的主密钥，至少 32 字符 |
| `DATABASE_URL` | 否 | `./agent-proxy-rust.db` | SQLite 数据库路径 |
| `AGENT_PROXY_LISTEN` | 否 | `127.0.0.1:8787` | 监听地址 |
| `AGENT_PROXY_LOG_FORMAT` | 否 | `pretty` | 日志格式：`pretty` 或 `json` |
| `AGENT_PROXY_PROXY_API_KEY` | 否 | — | 代理认证密钥（客户端请求时需携带） |
| `AGENT_PROXY_DATA_DIR` | 否 | OS 相关 | 数据目录，存放数据库和配置 |
| `AGENT_PROXY_UPSTREAM_TIMEOUT` | 否 | `30` | 上游超时（秒） |
| `AGENT_PROXY_COMPRESS__ENABLED` | 否 | `true` | 启用 tool schema 压缩 |

生成主密钥：

```bash
export PROXY_SECRET=$(openssl rand -hex 32)
```

> **重要**：`PROXY_SECRET` 丢失后，已加密的通道 API Key 无法解密。请妥善备份此值。

## Linux systemd 服务

### service unit 文件

```ini
# /etc/systemd/system/agent-proxy.service
[Unit]
Description=Agent Proxy Rust — AI API Middleware
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=agent-proxy
Group=agent-proxy
EnvironmentFile=/etc/agent-proxy/env
ExecStart=/usr/local/bin/agent-proxy serve --config /etc/agent-proxy/config.yaml
Restart=on-failure
RestartSec=5
LimitNOFILE=65536
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/agent-proxy
NoNewPrivileges=true

[Install]
WantedBy=multi-user.target
```

### 安装步骤

```bash
# 1. 创建专用用户
sudo useradd -r -s /usr/sbin/nologin agent-proxy

# 2. 创建数据目录
sudo mkdir -p /var/lib/agent-proxy
sudo chown agent-proxy:agent-proxy /var/lib/agent-proxy

# 3. 创建配置目录和环境文件
sudo mkdir -p /etc/agent-proxy
echo "PROXY_SECRET=$(openssl rand -hex 32)" | sudo tee /etc/agent-proxy/env

# 4. 复制二进制
sudo cp target/release/agent-proxy /usr/local/bin/

# 5. 创建配置文件（可选，也可全部通过环境变量）
sudo tee /etc/agent-proxy/config.yaml << 'EOF'
listen: "127.0.0.1:8787"
log_format: json
upstream_timeout: 30
compress:
  enabled: true
EOF

# 6. 创建 service 文件（内容见上方）

# 7. 启动
sudo systemctl daemon-reload
sudo systemctl enable agent-proxy
sudo systemctl start agent-proxy

# 8. 验证
curl http://127.0.0.1:8787/health
```

### Socket 激活（可选）

systemd 可绑定端口并传递 socket fd，代理检测 `LISTEN_FDS=1` 后自动使用继承的 socket：

```ini
# /etc/systemd/system/agent-proxy.socket
[Socket]
ListenStream=8787
BindIPv6Only=both
NoDelay=true

[Install]
WantedBy=sockets.target
```

## macOS launchd 服务

### plist 文件

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.agent-proxy.server</string>
    <key>ProgramArguments</key>
    <array>
        <string>/usr/local/bin/agent-proxy</string>
        <string>serve</string>
    </array>
    <key>EnvironmentVariables</key>
    <dict>
        <key>PROXY_SECRET</key>
        <string>YOUR_SECRET_HERE</string>
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

### 安装步骤

```bash
# 创建日志目录
sudo mkdir -p /usr/local/var/log/agent-proxy

# 安装 plist
sudo cp com.agent-proxy.server.plist ~/Library/LaunchAgents/

# 加载
launchctl load ~/Library/LaunchAgents/com.agent-proxy.server.plist

# 验证
curl http://127.0.0.1:8787/health
```

## Docker 部署（计划中）

> **Phase 2 计划**：当前版本未提供官方 Dockerfile。以下为设计草案，待后续版本实现。

```dockerfile
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
ENTRYPOINT ["agent-proxy", "serve"]
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
      - agent-proxy-data:/var/lib/agent-proxy
    restart: unless-stopped

volumes:
  agent-proxy-data:
```

## 健康检查

代理提供 `GET /health` 端点：

```bash
# 简单检查
curl http://127.0.0.1:8787/health

# JSON 格式
curl -s http://127.0.0.1:8787/health | jq .
```

响应示例：

```json
{
  "status": "ok",
  "uptime_seconds": 86400,
  "channels_total": 9,
  "channels_healthy": 9,
  "db_connected": true
}
```

当数据库不可达或所有通道不健康时返回 `503 Service Unavailable`。

## 日志

```bash
# 生产环境 JSON 日志（适合 journald / 日志聚合）
AGENT_PROXY_LOG_FORMAT=json agent-proxy serve

# 调整日志级别
RUST_LOG=agent_proxy=debug agent-proxy serve

# 按模块精细控制
RUST_LOG=agent_proxy=info,agent_proxy_core=debug agent-proxy serve
```

日志级别：`ERROR` > `WARN` > `INFO` > `DEBUG` > `TRACE`。API Key 和密钥通过 `secrecy::SecretString` 自动脱敏，永不出现在日志中。

| 级别 | 内容 |
|------|------|
| `ERROR` | 上游故障、数据库错误、启动失败 |
| `WARN` | 废弃模型使用、配额耗尽、通道降级 |
| `INFO` | 启停、通道健康变化、周期统计 |
| `DEBUG` | 请求路由决策、协议转换详情 |
| `TRACE` | 完整请求/响应体（生产环境禁用） |

## 备份

```bash
# SQLite 在线备份（推荐）
sqlite3 agent-proxy.db ".backup backup-$(date +%Y%m%d).db"

# 或直接复制（需先停止写入或使用 WAL 模式）
cp agent-proxy.db backup-$(date +%Y%m%d).db
```

数据目录位置：

| OS | 路径 |
|----|------|
| Linux | `~/.local/share/agent-proxy/` |
| macOS | `~/Library/Application Support/com.agent-proxy/` |
| Docker | `/var/lib/agent-proxy/` |

## 资源限制

| 资源 | 默认值 | 配置方式 |
|------|--------|----------|
| 最大请求体 | 16 MB | `--max-body-size` |
| 上游超时 | 30s | `--upstream-timeout` |
| 上游连接超时 | 10s | `--upstream-connect-timeout` |
| 最大并发连接 | 512 | `--max-connections` |
| 最大文件描述符 | 继承系统 | systemd `LimitNOFILE` |

## 故障排查

### 服务无法启动

```bash
# 检查 PROXY_SECRET 是否设置
echo $PROXY_SECRET

# 检查端口是否被占用
lsof -i :8787

# 查看 systemd 日志
journalctl -u agent-proxy -f
```

### 健康检查返回 503

```bash
# 检查通道状态
curl -s http://127.0.0.1:8787/health | jq .

# 检查数据库文件权限
ls -la /var/lib/agent-proxy/
```

### 请求返回 503

通道不健康时代理返回 503。检查通道 API Key 是否已设置，以及上游服务是否可达。故障转移机制会在 3 次连续失败后标记通道为 Unhealthy，60s 后自动探测恢复。

### 性能调优

```bash
# 增加文件描述符限制
sudo tee -a /etc/security/limits.conf << 'EOF'
agent-proxy soft nofile 65536
agent-proxy hard nofile 65536
EOF

# SQLite WAL 模式（默认启用，确认即可）
sqlite3 agent-proxy.db "PRAGMA journal_mode=WAL;"
```

---

Owner: baoyx · 版本：v1.0 · 生效日期：2026-06-12 · 最后更新：2026-06-12
