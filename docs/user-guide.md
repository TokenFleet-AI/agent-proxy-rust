# User Guide

agent-proxy-rust 是一个可组合的 AI Agent API 中间件代理。它位于 AI 编程 Agent 与上游 LLM 提供商之间，提供压缩、路由、协议桥接和计费追踪。

## 安装

```bash
# 从源码安装（需要 Rust 工具链）
git clone https://github.com/TokenFleet-AI/agent-proxy-rust.git
cd agent-proxy-rust
cargo install --path apps/cli

# 或直接
cargo install agent-proxy
```

## 快速上手

```bash
# 1. 启动代理（13 个内置通道自动 seed，65 个模型映射就绪）
agent-proxy serve

# 2. 设置通道 API Key
agent-proxy channel set-key anthropic-official --api-key "sk-ant-xxx"
agent-proxy channel set-key openai-official --api-key "sk-xxx"
agent-proxy channel set-key dashscope-payg-openai --api-key "sk-xxx"

# 3. 配置你的 AI Agent 使用代理
#    API Endpoint: http://127.0.0.1:8787
```

## 配置你的 AI Agent

### Claude Code

```bash
export ANTHROPIC_BASE_URL="http://127.0.0.1:8787"
export ANTHROPIC_API_KEY="sk-proxy-xxx"  # 你的 proxy_api_key（若启用认证）
```

### Codex (OpenAI)

```bash
export OPENAI_BASE_URL="http://127.0.0.1:8787/v1"
export OPENAI_API_KEY="sk-proxy-xxx"
```

### Gemini CLI

```bash
export GOOGLE_API_BASE="http://127.0.0.1:8787"
```

---

## 内置通道

代理启动时自动创建 13 个通道（API Key 为空，需手动设置）：

| 通道 ID | 名称 | 协议 |
|----------|------|------|
| `anthropic-official` | Anthropic Official | `anthropic_messages` |
| `openai-official` | OpenAI Official | `openai_responses` |
| `deepseek-openai` | DeepSeek Official (OpenAI) | `openai_chat` |
| `deepseek-anthropic` | DeepSeek Official (Anthropic) | `anthropic_messages` |
| `dashscope-token-openai` | DashScope Token Plan (OpenAI) | `openai_chat` |
| `dashscope-token-anthropic` | DashScope Token Plan (Anthropic) | `anthropic_messages` |
| `dashscope-coding-openai` | DashScope Coding Plan (OpenAI) | `openai_chat` |
| `dashscope-coding-anthropic` | DashScope Coding Plan (Anthropic) | `anthropic_messages` |
| `dashscope-payg-openai` | DashScope 按量计费 (OpenAI) | `openai_chat` |
| `dashscope-payg-anthropic` | DashScope 按量计费 (Anthropic) | `anthropic_messages` |
| `glm-official` | 智谱 GLM Official | `openai_chat` |
| `kimi-official` | Kimi Official | `openai_chat` |
| `minimax-official` | MiniMax Official | `openai_chat` |

---

## 内置模型映射（65 个）

### Anthropic（3 个）
`claude-opus-4-7` · `claude-sonnet-4-6` · `claude-haiku-4-5`

### OpenAI（3 个）
`gpt-5.5` · `gpt-5.4` · `gpt-5-codex`

### DeepSeek（2 个）
`deepseek-v4-flash` · `deepseek-v4-pro`

### 百炼 Qwen（6 个）
`qwen3.7-max` · `qwen3.6-max` · `qwen3.6-plus` · `qwen3.6-flash` · `qwen3.5-plus` · `qwen3.5-flash`

### 智谱 GLM（7 个）
`glm-5.1` · `glm-5-turbo` · `glm-5` · `glm-4.7` · `glm-4.5-air` · `glm-4.7-flashx` · `glm-4.7-flash`

### Kimi（2 个）
`kimi-k2.6` · `kimi-k2.5`

### MiniMax（4 个）
`minimax-m2.7` · `minimax-m2.7-highspeed` · `minimax-m2.5` · `minimax-m2.5-highspeed`

---

## 通道管理

### 列出所有通道

```bash
agent-proxy channel list
```

输出：
```
ID                              NAME                            PROTOCOL              ENABLED
anthropic-official              Anthropic Official              anthropic_messages    true
openai-official                 OpenAI Official                 openai_responses      true
deepseek-openai                 DeepSeek Official (OpenAI)      openai_chat           true
...
```

### 设置 API Key

```bash
agent-proxy channel set-key <channel-id> --api-key "<key>"
```

### 添加自定义通道

```bash
agent-proxy channel add <id> \
  --name "My Channel" \
  --url "https://api.example.com/v1" \
  --protocol anthropic_messages \
  --api-key "sk-xxx"
```

支持的协议：`anthropic_messages` / `openai_chat` / `openai_responses`

### 启用/禁用通道

```bash
agent-proxy channel enable <channel-id>
agent-proxy channel disable <channel-id>
```

---

## 配置

### 配置文件（YAML）

```yaml
# ~/.config/agent-proxy/config.yaml
listen: "127.0.0.1:8787"

upstream_timeout: 30         # 上游超时（秒）
upstream_connect_timeout: 10 # 连接超时（秒）

log_format: pretty           # pretty | json

# 代理级认证（可选）
proxy_api_key: "sk-my-secret-key"

# 按角色认证（Ruflo 集群模式）
proxy_auth:
  keys:
    sk-proxy-architect:
      role: architect
    sk-proxy-coder:
      role: coder
    sk-proxy-tester:
      role: tester

# 压缩
compress:
  enabled: true
  min_schema_size: 512
  compress_responses: true

# 桥接
bridge:
  max_conversion_body_size: 1048576

# 计费
cost:
  retention_days: 90
```

### 配置优先级

```
CLI flags > 环境变量 > YAML 配置文件 > 默认值
```

### 配置发现路径

代理启动时依次查找配置文件：
1. `--config <PATH>` 命令行指定
2. `AGENT_PROXY_CONFIG` 环境变量
3. `{data_dir}/config.yaml`
4. `~/.config/agent-proxy/config.yaml`
5. `./agent-proxy.yaml`

### 环境变量

所有变量前缀 `AGENT_PROXY_`，嵌套用 `__` 分隔：

| 变量 | 说明 | 默认 |
|------|------|------|
| `AGENT_PROXY_LISTEN` | 监听地址 | `127.0.0.1:8787` |
| `AGENT_PROXY_DATA_DIR` | 数据目录 | OS 相关 |
| `AGENT_PROXY_UPSTREAM_TIMEOUT` | 上游超时 | `30` |
| `AGENT_PROXY_LOG_FORMAT` | 日志格式 | `pretty` |
| `AGENT_PROXY_PROXY_API_KEY` | 代理认证密钥 | — |
| `AGENT_PROXY_COMPRESS__ENABLED` | 启用压缩 | `true` |
| `AGENT_PROXY_BRIDGE__MAX_CONVERSION_SIZE` | 最大转换体 | `1048576` |
| `PROXY_SECRET` | 加密主密钥（必填） | — |

---

## Ruflo 集群模式

### 按角色计费

在 Ruflo swarm 中，每个角色（architect / coder / tester / reviewer）分配独立的 proxy API Key，代理自动追踪每个角色的花费。

```yaml
# config.yaml
proxy_auth:
  keys:
    sk-proxy-architect: { role: architect }
    sk-proxy-coder:     { role: coder }
    sk-proxy-tester:    { role: tester }
    sk-proxy-reviewer:  { role: reviewer }
```

Ruflo 启动 agent 时注入对应 Key：

```
Ruflo agent_spawn:
  architect → ANTHROPIC_API_KEY=sk-proxy-architect
  coder     → ANTHROPIC_API_KEY=sk-proxy-coder
  tester    → ANTHROPIC_API_KEY=sk-proxy-tester
```

### 查询角色花费

```sql
SELECT
    agent_role,
    SUM(actual_cost) as total_cost,
    COUNT(*) as requests
FROM cost_records
GROUP BY agent_role;
```

---

## 计费追踪

费用记录在 SQLite 数据库 `{data_dir}/agent-proxy.db` 中。

### 查询示例

```bash
# 进入数据库
sqlite3 ~/.local/share/agent-proxy/agent-proxy.db

# 本月各项目花费
SELECT
    project_name,
    SUM(actual_cost) as total_cost,
    SUM(input_tokens + output_tokens) as total_tokens
FROM cost_records
WHERE date(timestamp, 'unixepoch') >= date('now', 'start of month')
GROUP BY project_path
ORDER BY total_cost DESC;

# 各通道花费
SELECT
    channel_name,
    COUNT(*) as requests,
    SUM(actual_cost) as cost
FROM cost_records
GROUP BY channel_name
ORDER BY cost DESC;

# 压缩节省
SELECT
    SUM(pre_compress_tokens) as before,
    SUM(post_compress_tokens) as after,
    ROUND(100.0 * SUM(compression_tokens_saved) / SUM(pre_compress_tokens), 1) as pct_saved
FROM cost_records;
```

---

## 健康检查

```bash
curl http://127.0.0.1:8787/health
```

```json
{
  "status": "ok",
  "uptime_seconds": 86400,
  "channels_total": 13,
  "channels_healthy": 10,
  "db_connected": true
}
```

---

## 请求流程

```
Agent 发送 POST /v1/messages
        │
   ┌────┴────┐
   │ 认证层   │  proxy_api_key 验证 + role 识别
   ├─────────┤
   │ 压缩     │  tool schema 瘦身 ~60-70%
   ├─────────┤
   │ 路由     │  FlatFee 优先 → Metered → 故障转移
   ├─────────┤
   │ 桥接     │  Anthropic ↔ OpenAI 格式互转（如需要）
   ├─────────┤
   │ 转发     │  → 上游 API
   ├─────────┤
   │ 响应     │  ← 桥接逆转换 → 响应压缩（非流式）
   ├─────────┤
   │ 计费     │  写 cost_records
   └─────────┘
        │
   Agent 收到响应（完全透明）
```

---

## 故障转移

当上游返回 5xx 或连接超时，3 次连续失败后标记通道为 Unhealthy，60s cooldown：

```
claude-sonnet 请求:
  ├── Anthropic Official → Unhealthy（3 连败 5xx）→ 跳过
  ├── DeepSeek Anthropic → Healthy → 选中 ✓
  └── DashScope Anthropic → 未检查（已选中）
```

无需用户干预，60s 后自动探测恢复。

---

## 数据目录

| OS | 路径 |
|----|------|
| Linux | `~/.local/share/agent-proxy/` |
| macOS | `~/Library/Application Support/com.agent-proxy/` |
| Windows | `%APPDATA%\agent-proxy\` |

内容：
```
{data_dir}/
├── agent-proxy.db         # SQLite 数据库
├── agent-proxy.db-wal     # WAL 日志
└── config.yaml            # 配置文件
```

---

## 日志

```bash
# JSON 日志（systemd / 生产环境）
AGENT_PROXY_LOG_FORMAT=json agent-proxy serve

# 调整日志级别
RUST_LOG=agent_proxy=debug agent-proxy serve
```

日志级别：`ERROR` > `WARN` > `INFO` > `DEBUG` > `TRACE`
API Key 和密钥永不出现在日志中。

---

## 故障排查

### 代理无法启动

```bash
# 检查 PROXY_SECRET 是否设置
echo $PROXY_SECRET

# 检查配置文件语法
agent-proxy serve --config /path/to/config.yaml --log-format json
```

### 请求返回 503

```bash
# 检查通道健康状态
curl http://127.0.0.1:8787/health

# 检查通道 API Key 是否已设置
agent-proxy channel list
```

### 跨协议请求失败

检查通道协议与客户端请求格式是否匹配。如果客户端用 Anthropic 格式但通道只有 OpenAI 协议，代理会自动桥接——无需额外配置。桥接失败时检查 `llm-bridge-core` 依赖版本。

### 压缩未生效

```bash
# 确认压缩已启用
grep "compress" config.yaml

# 检查 tools 数组大小是否 > min_schema_size（默认 512 bytes）
```
