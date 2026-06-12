# User Guide

agent-proxy-rust 是一个可组合的 AI Agent API 中间件代理。它位于 AI 编程 Agent 与上游 LLM 提供商之间，提供压缩、路由、协议桥接和计费追踪。

## 安装

```bash
# 从源码安装（需要 Rust 工具链）
git clone https://github.com/TokenFleet-AI/agent-proxy-rust.git
cd agent-proxy-rust
cargo install --path apps/server

# 或直接
cargo install agent-proxy
```

### 环境变量

启动前**必须**设置 `PROXY_SECRET`，用于加密存储通道 API Key：

| 变量 | 必填 | 说明 | 默认 |
|------|:----:|------|------|
| `PROXY_SECRET` | ✅ | 加密主密钥，至少 32 字符 | — |
| `DATABASE_URL` | — | SQLite 数据库路径 | `./agent-proxy-rust.db` |
| `AGENT_PROXY_LISTEN` | — | 监听地址 | `127.0.0.1:8787` |
| `AGENT_PROXY_LOG_FORMAT` | — | 日志格式 `pretty` / `json` | `pretty` |
| `AGENT_PROXY_PROXY_API_KEY` | — | 代理认证密钥（客户端需携带） | — |

生成 `PROXY_SECRET`：

```bash
export PROXY_SECRET=$(openssl rand -hex 32)
```

> **注意**：`PROXY_SECRET` 丢失后已加密的 API Key 无法解密。请妥善备份。

## 快速上手

```bash
# 0. 设置加密密钥（必须）
export PROXY_SECRET=$(openssl rand -hex 32)

# 1. 启动代理（9 个内置通道自动 seed，36 个模型、57 条映射就绪）
agent-proxy serve

# 2. 设置通道 API Key（按你使用的通道设置）
agent-proxy channel set-key tokenfleet-ai --api-key "sk-tf-xxx"
agent-proxy channel set-key deepseek --api-key "sk-xxx"
agent-proxy channel set-key dashscope-payg --api-key "sk-xxx"

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

代理启动时自动创建 9 个通道（API Key 为空，需手动设置）：

| 通道 ID | 名称 | Provider | 优先级 | 协议 |
|---------|------|----------|:------:|------|
| `tokenfleet-ai` | TokenFleet AI | tokenfleet | 1 | `anthropic_messages` · `openai_chat` · `openai_responses` |
| `tokenfleet-cn` | TokenFleet CN | tokenfleet | 2 | `anthropic_messages` · `openai_chat` |
| `dashscope-coding` | DashScope Coding Plan | alibaba-bailian | 10 | `openai_chat` · `anthropic_messages` |
| `dashscope-payg` | DashScope 按量计费 | alibaba-bailian | 10 | `openai_chat` · `anthropic_messages` |
| `dashscope-token` | DashScope Token Plan | alibaba-bailian | 10 | `openai_chat` · `anthropic_messages` |
| `deepseek` | DeepSeek Official | deepseek | 10 | `openai_chat` · `anthropic_messages` |
| `glm-official` | 智谱 GLM Official | zhipu | 10 | `openai_chat` |
| `kimi-official` | Kimi Official | moonshot | 10 | `openai_chat` |
| `minimax-official` | MiniMax Official | minimax | 10 | `openai_chat` |

### TokenFleet 通道（推荐）

**`tokenfleet-ai`**（优先级 1）和 **`tokenfleet-cn`**（优先级 2）是首选通道，具有以下优势：

- **聚合多厂商模型**：一个通道同时提供 Anthropic Claude、OpenAI GPT、DeepSeek、Kimi、GLM、MiniMax 等模型
- **双协议支持**：同时支持 `anthropic_messages` 和 `openai_chat`，无需桥接
- **自动故障转移**：TokenFleet 通道优先级最高，请求优先走 TokenFleet；当 TokenFleet 不可用时自动回退到直连通道
- **差异化定价**：TokenFleet 通道的价格可能与官方直连不同（如 Claude Opus 4-8 在 `tokenfleet-ai` 上 input $5/MTok，低于官方定价）
- **`tokenfleet-ai`** 面向海外节点（tokenfleet.ai），提供 Claude 和 GPT 全系列
- **`tokenfleet-cn`** 面向国内节点（tokenfleet.cn），提供 DeepSeek、Kimi、GLM、MiniMax、MiMo 等国产模型

---

## 内置模型映射（36 个模型，57 条通道映射）

代理内置 36 个模型定义，通过 57 条映射分配到各通道。同一模型可在多个通道上可用（如 `kimi-k2.5` 同时映射到 `tokenfleet-ai`、`tokenfleet-cn`、`dashscope-token`、`dashscope-coding`、`kimi-official`）。

价格单位：CNY/MTok（人民币每百万 Token）或 USD/MTok（美元每百万 Token）。

### Anthropic Claude（7 个）

| 模型 ID | 输入价格 | 输出价格 | 货币 | 上下文窗口 |
|---------|:--------:|:--------:|:----:|:----------:|
| `claude-opus-4-8` | 5.0 | 25.0 | USD | 200K |
| `claude-opus-4-7` | 5.0 | 25.0 | USD | 200K |
| `claude-opus-4-6` | 15.0 | 75.0 | USD | 200K |
| `claude-opus-4-5-reverse` | 15.0 | 75.0 | USD | 200K |
| `claude-sonnet-4-6` | 3.0 | 15.0 | USD | 200K |
| `claude-sonnet-4-5` | 3.0 | 15.0 | USD | 200K |
| `claude-sonnet-4-5-reverse` | 3.0 | 15.0 | USD | 200K |

> Claude 系列通过 `tokenfleet-ai` 通道使用。

### OpenAI GPT（7 个）

| 模型 ID | 输入价格 | 输出价格 | 货币 | 上下文窗口 |
|---------|:--------:|:--------:|:----:|:----------:|
| `gpt-5.5` | 5.0 | 30.0 | USD | 400K |
| `gpt-5.4` | 2.5 | 15.0 | USD | 256K |
| `gpt-5.4-mini` | 0.6 | 2.4 | USD | 256K |
| `gpt-5.3-codex` | 3.0 | 20.0 | USD | 256K |
| `gpt-5.2` | 1.75 | 14.0 | USD | 256K |
| `gpt-5.2-chat` | 1.25 | 10.0 | USD | 256K |
| `gpt-5.2-codex` | 1.25 | 10.0 | USD | 256K |

> GPT 系列通过 `tokenfleet-ai` 通道使用。

### DeepSeek（4 个）

| 模型 ID | 输入价格 | 输出价格 | 货币 | 上下文窗口 |
|---------|:--------:|:--------:|:----:|:----------:|
| `deepseek-v4-pro` | 3.0 | 6.0 | CNY | 1M |
| `deepseek-v4-flash` | 1.0 | 2.0 | CNY | 1M |
| `deepseek-v3.2` | 2.0 | 8.0 | CNY | 1M |
| `deepseek-v3.1` | 2.0 | 8.0 | CNY | 1M |

> DeepSeek 可通过 `tokenfleet-cn`、`deepseek`、`dashscope-token` 通道使用。

### 百炼 Qwen（7 个）

| 模型 ID | 输入价格 | 输出价格 | 货币 | 上下文窗口 |
|---------|:--------:|:--------:|:----:|:----------:|
| `qwen3.7-max` | 6.0 | 18.0 | CNY | 256K |
| `qwen3.7-plus` | 1.6 | 6.4 | CNY | 256K |
| `qwen3.6-max` | 9.0 | 54.0 | CNY | 256K |
| `qwen3.6-plus` | 2.0 | 12.0 | CNY | 1M |
| `qwen3.6-flash` | 1.2 | 7.2 | CNY | 1M |
| `qwen3.5-plus` | 0.8 | 4.8 | CNY | 1M |
| `qwen3.5-flash` | 0.2 | 2.0 | CNY | 1M |

> Qwen 系列通过 `dashscope-payg`、`dashscope-coding`、`dashscope-token` 通道使用。

### 智谱 GLM（5 个）

| 模型 ID | 输入价格 | 输出价格 | 货币 | 上下文窗口 |
|---------|:--------:|:--------:|:----:|:----------:|
| `glm-5.1` | 6.0 | 24.0 | CNY | 200K |
| `glm-5-turbo` | 5.0 | 22.0 | CNY | 200K |
| `glm-5` | 4.0 | 18.0 | CNY | 200K |
| `glm-5v-turbo` | 5.0 | 20.0 | CNY | 200K |
| `glm-4.7-flash` | 免费 | 免费 | CNY | 200K |

> GLM 可通过 `tokenfleet-cn`、`glm-official`、`dashscope-coding`、`dashscope-token` 通道使用。

### Kimi / Moonshot（2 个）

| 模型 ID | 输入价格 | 输出价格 | 货币 | 上下文窗口 |
|---------|:--------:|:--------:|:----:|:----------:|
| `kimi-k2.6` | 6.5 | 27.0 | CNY | 256K |
| `kimi-k2.5` | 4.0 | 21.0 | CNY | 256K |

> Kimi 可通过 `tokenfleet-ai`、`tokenfleet-cn`、`kimi-official`、`dashscope-token`、`dashscope-coding` 通道使用。

### MiniMax（2 个）

| 模型 ID | 输入价格 | 输出价格 | 货币 | 上下文窗口 |
|---------|:--------:|:--------:|:----:|:----------:|
| `minimax-m2.7` | 2.1 | 8.4 | CNY | 256K |
| `minimax-m2.5` | 1.5 | 6.0 | CNY | 256K |

> MiniMax 可通过 `tokenfleet-cn`、`minimax-official`、`dashscope-token`、`dashscope-coding` 通道使用。

### 小米 MiMo（2 个）

| 模型 ID | 输入价格 | 输出价格 | 货币 | 上下文窗口 |
|---------|:--------:|:--------:|:----:|:----------:|
| `mimo-v2.5-pro` | 1.0 | 6.0 | CNY | 1M |
| `mimo-v2.5` | 1.0 | 2.0 | CNY | 1M |

> MiMo 通过 `tokenfleet-cn` 通道使用。

---

## 通道管理

### 列出所有通道

```bash
agent-proxy channel list
```

输出：
```
ID                    NAME                        PRIORITY  ENABLED
tokenfleet-ai         TokenFleet AI               1         true
tokenfleet-cn         TokenFleet CN               2         true
dashscope-coding      DashScope Coding Plan       10        true
deepseek              DeepSeek Official           10        true
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

所有配置变量前缀 `AGENT_PROXY_`，嵌套用 `__` 分隔。`PROXY_SECRET` 为独立变量（详见上方[安装](#环境变量)章节）。

| 变量 | 说明 | 默认 |
|------|------|------|
| `PROXY_SECRET` | 加密主密钥（**必填**，至少 32 字符） | — |
| `AGENT_PROXY_LISTEN` | 监听地址 | `127.0.0.1:8787` |
| `AGENT_PROXY_DATA_DIR` | 数据目录 | OS 相关 |
| `AGENT_PROXY_UPSTREAM_TIMEOUT` | 上游超时（秒） | `30` |
| `AGENT_PROXY_LOG_FORMAT` | 日志格式 `pretty` / `json` | `pretty` |
| `AGENT_PROXY_PROXY_API_KEY` | 代理认证密钥（客户端需携带） | — |
| `AGENT_PROXY_COMPRESS__ENABLED` | 启用压缩 | `true` |
| `AGENT_PROXY_BRIDGE__MAX_CONVERSION_SIZE` | 最大转换体（字节） | `1048576` |

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
    SUM(cost) as total_cost,
    COUNT(*) as requests,
    SUM(tokens_saved) as total_saved
FROM cost_records
WHERE agent_role IS NOT NULL
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
    project,
    SUM(cost) as total_cost,
    SUM(input_tokens + output_tokens) as total_tokens,
    SUM(tokens_saved) as total_saved
FROM cost_records
WHERE timestamp >= date('now', 'start of month')
GROUP BY project
ORDER BY total_cost DESC;

# 各通道花费
SELECT
    channel_id,
    COUNT(*) as requests,
    SUM(cost) as cost,
    SUM(tokens_saved) as total_saved
FROM cost_records
GROUP BY channel_id
ORDER BY cost DESC;

# 按 session 汇总
SELECT
    session_id,
    COUNT(*) as requests,
    SUM(cost) as total_cost,
    SUM(tokens_saved) as total_saved
FROM cost_records
WHERE session_id IS NOT NULL
GROUP BY session_id;

# 压缩节省
SELECT
    SUM(before_tokens) as before,
    SUM(after_tokens) as after,
    ROUND(100.0 * SUM(tokens_saved) / NULLIF(SUM(after_tokens + tokens_saved), 0), 1) as pct_saved
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
  "channels_total": 9,
  "channels_healthy": 9,
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
  ├── tokenfleet-ai → Unhealthy（3 连败 5xx）→ 跳过
  ├── deepseek → Healthy → 选中 ✓
  └── dashscope-payg → 未检查（已选中）
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

---

Owner: baoyx · 版本：v1.1 · 生效日期：2026-05-21 · 最后更新：2026-06-12
