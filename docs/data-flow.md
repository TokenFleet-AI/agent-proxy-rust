# Data Flow

agent-proxy-rust 完整数据流，基于 13 份设计文档。

## 架构总览

```
Agent (Claude Code / Codex / Gemini CLI / ...)
        │
        │  POST /v1/messages  /  /v1/chat/completions  /  /v1/responses
        ▼
┌──────────────────────────────────────────────────────┐
│                   agent-proxy-rust                    │
│                                                      │
│  Tower Layer                                         │
│  ├── auth check (proxy_api_key?)                     │
│  ├── rate limit (token bucket)                       │
│  ├── body size limit (16MB)                          │
│  └── input validation                                │
│                                                      │
│  Session correlation                                 │
│  ├── 读 X-Claude-Code-Session-Id header              │
│  ├── rename-then-read 消费 tokenless report 文件     │
│  └── 注入 ctx: session_id, tokenless_saved_tokens    │
│                                                      │
│  on_request 链 (注册顺序)                             │
│  ├── ① CompressMiddleware    schema 瘦身 60-70%      │
│  ├── ② ModelRouterMiddleware 选通道 + 模型名映射     │
│  └── ③ BridgeMiddleware      协议格式互转            │
│                                                      │
│  Forward → upstream (reqwest HTTP/1.1)               │
│                                                      │
│  on_response 链 (反向顺序)                            │
│  ├── ③ BridgeMiddleware      协议逆转换              │
│  ├── ② ModelRouterMiddleware 记录通道健康/延迟       │
│  └── ① CompressMiddleware    响应压缩 (非流式)       │
│                                                      │
│  CostMiddleware (on_response 之后)                    │
│  └── 计算花费 → SQLite                                │
└──────────────────────────────────────────────────────┘
        │
        ▼
   Upstream LLM API
```

## 启动流程

```
launchd/systemd 拉起 agent-proxy serve
        │
   ┌────┴────┐
   │ seed 4  │  Anthropic / OpenAI / Google / DeepSeek
   │ channels│  api_key 均为空, is_builtin=true
   ├─────────┤
   │ init DB │  SQLite WAL mode + PRAGMA
   ├─────────┤
   │ load    │  YAML → env → CLI flags → validate
   │ config  │
   └────┬────┘
        │
   listen 127.0.0.1:8787
```

## 五种请求场景

### 场景一：同协议直通（最常见）

```
Claude Code → POST /v1/messages (Anthropic 格式)
                model: "claude-sonnet"

CompressMiddleware
  body.tools[] 提取
  SchemaCompressor::compress() 每个 tool
    • 删 title/examples
    • 截断 description (func 256 / param 160 chars)
    • 去 markdown + 合并空白
  12000 tokens → ~4500 tokens
  写 ctx.extensions["stats_record"]

ModelRouterMiddleware
  client_name="claude-sonnet"
  查 model_mappings →
    FlatFee 有配额? 无
    Metered: Anthropic Official (Healthy) → 选中
  req.body.model → "claude-4-7"
  写 ctx["selected_channel"]
  ctx.target_protocol = AnthropicMessages

BridgeMiddleware
  detected = AnthropicMessages
  target   = AnthropicMessages
  → PASSTHROUGH

Forward
  Authorization: Bearer sk-ant-xxx
  upstream_url → https://api.anthropic.com

SSE 流返回...
  BridgeMiddleware.on_response: PASSTHROUGH
  ModelRouterMiddleware: 记录 Healthy + latency
  CompressMiddleware: 流式 → 跳过 ResponseCompressor

CostRecorder (after on_response chain, NOT in middleware)
  读 ctx.session_id → "sess_abc" (from X-Claude-Code-Session-Id header)
  读 ctx.tokenless_saved_tokens → 950 (from report file via rename-then-read)
  读 ctx.tokenless_breakdown_json → [{"op":"CompressSchema","method":"ToonHrv","saved":800},...]
  读 ctx.extensions["compression_stats"] → proxy_schema_saved=7500
  读 ctx.extensions["selected_mapping"] → pricing
  读 response usage → input=1450, output=380

  calc_cost:
    (1450/1M × $3.00) + (380/1M × $15.00) = $0.01005
  total_saved = tokenless_saved(950) + proxy_saved(7500) = 8450
  before_tokens = after(1830) + total_saved(8450) = 10280 (估算)

  INSERT INTO cost_records (
    project, user_id, agent_type,
    channel_id,
    input_tokens, output_tokens, cache_write_tokens, cache_read_tokens, thinking_tokens,
    cost, unit,
    schema_saved_tokens, response_saved_tokens, rtk_saved_tokens,
    pre_compress_tokens, post_compress_tokens, compression_tokens_saved,
    pricing_snapshot_json,
    session_id, before_tokens, after_tokens, tokens_saved, compression_breakdown_json,
    timestamp
  )
```

### 场景二：跨协议桥接

```
Claude Code (Anthropic 格式)
  → 选中 DashScope OpenAI Chat channel
    channel.protocol = OpenaiChat
    detected_format = AnthropicMessages
    → DIFFERENT!

BridgeMiddleware.on_request()
  anthropic_to_openai()
    tool_use    → tool_calls
    system      → messages[0].role="system"
    content[]   → messages[]
  写 ctx["bridge_reverse"] { source=OpenaiChat, target=AnthropicMessages }

Forward → DashScope (收到的是 OpenAI 格式)

BridgeMiddleware.on_response()
  读 ctx["bridge_reverse"]
  openai_to_anthropic() ← 逆转换回 Anthropic 格式

↑ Claude Code 完全不知道发生过转换
```

### 场景三：故障转移

```
ModelRouterMiddleware 选通道:

claude-sonnet:
  ├── Copilot Subscription  → quota exhausted, FallbackToMetered
  ├── Anthropic Official    → Unhealthy (上 3 次 5xx)
  ├── DashScope Anthropic   → Healthy → 选中 ✓
  └── OpenRouter            → 未检查 (已选中)

↑ 客户端仍在请求 "claude-sonnet"，无感知
   60s 后 cooldown 过期 → 下次真实请求自动探测 Anthropic Official
```

### 场景四：包月通道

```
POST /v1/chat/completions
  model: "claude-sonnet"

ModelRouterMiddleware:
  → 选中 Copilot channel (FlatFee / Subscription)
  channel_kind = Subscription

  ... 中间件链 + 转发上游 ...

CostMiddleware:
  channel_kind = Subscription
  → actual_cost = 0
  (月费在 subscription_fees 表单独管理)
```

### 场景五：非流式完整链路（压缩生效全路径）

```
POST /v1/messages (非流式 stream:false)
  body: { tools: [...], messages: [...] }

on_request:
  CompressMiddleware: schema 压缩 ~65%
  ModelRouterMiddleware: 选通道
  BridgeMiddleware: PASSTHROUGH

Forward → upstream (30s timeout)

upstream returns 200 OK (完整 JSON, ~800 tokens)

on_response:
  BridgeMiddleware: PASSTHROUGH
  ModelRouterMiddleware: 记录通道健康
  CompressMiddleware:
    ResponseCompressor::compress()
      • 删 debug/trace/stack/logs 字段
      • 截断 >512 字符的字符串
      • 截断 >16 元素的数组
      • 删 null/空值
      • 深度 >8 截断
    ~800 tokens → ~600 tokens

CostMiddleware:
  pre_compress_tokens: 12800 (request + response)
  post_compress_tokens: 5100
  compression_tokens_saved: 7700
```

## SSE 流式帧处理

```
Anthropic Messages SSE 帧    → Bridge 处理    → Compress 处理
─────────────────────────────────────────────────────────────
message_start                passthrough      跳过
content_block_start          passthrough      跳过
content_block_delta          transform_stream 跳过 (内容帧不能截断)
content_block_stop           passthrough      跳过
message_delta (含 usage)     passthrough      跳过
message_stop                 passthrough      跳过

CostMiddleware: 从 message_delta 解析 usage
```

## 中间件数据传递

```
ctx.extensions (HashMap<String, Box<dyn Any>>)

Key                    Writer                Reader
─────────────────────────────────────────────────────────
stats_record           CompressMiddleware     CostMiddleware
selected_channel       ModelRouterMiddleware  CostMiddleware
selected_mapping       ModelRouterMiddleware  CostMiddleware
bridge_reverse         BridgeMiddleware       BridgeMiddleware
```

## 通道选择决策树

```
find_mapping(client_name)
        │
        ▼
FlatFee (Subscription) ?
  ├── quota > 0 && healthy → 选中
  └── exhausted:
        ├── FallbackToMetered → 继续↓
        └── Block → 返回 503
        │
Metered channels ?
  ├── healthy → 选中第一个
  └── all unhealthy:
        └── cooldown > 60s? → 试一个 (真实请求 = probe)
        └── 全部失败 → 503

Phase 2 扩展: 加权随机 + standby + last-resort
```

## 计费公式

```
PerToken:
  cost = (input/1M × input_price) + (output/1M × output_price)
       + (cache_write/1M × cache_write_price)
       + (cache_read/1M × cache_read_price)
       + (thinking/1M × thinking_price)

Subscription:
  cost = 0 (月费在 subscription_fees 表单独记录)

Credits:
  credits = credits_used (不转 $, 除非有汇率)

CharBased:
  chars ≈ tokens × 0.75
  cost = (input_chars/1M × price) + (output_chars/1M × price × multiplier)
```

## 安全边界

```
客户端 → proxy:
  ① auth check (proxy_api_key?)
  ② rate limit (token bucket)
  ③ body size ≤ 16MB
  ④ model name regex: ^[a-zA-Z0-9._-]{1,128}$
  ⑤ message count ≤ 1000
  ⑥ header value ≤ 8KB
  ⑦ unknown path → 404

proxy → upstream:
  ① API key: secrecy::SecretString
  ② SQLite chmod 600 (Phase 1) / AES-256-GCM (Phase 2)
  ③ TLS: rustls aws-lc-rs
  ④ SSRF: 拒绝 loopback/private/multicast IP
  ⑤ header 重建: drop hop-by-hop, 重建 authorization/content-length

proxy → 客户端 (error):
  ① 从不泄露 API key
  ② 从不泄露内部 hostname / 堆栈
  ③ 统一 JSON: {"error": {"code":"...","message":"..."}}
```

## 数据目录布局

```
{data_dir}/
├── agent-proxy.db         SQLite: channels + model_mappings + cost_records
├── agent-proxy.db-wal     WAL journal
├── agent-proxy.db-shm     WAL shared memory
└── config.yaml            运行时配置

{data_dir} 默认值:
  Linux:   ~/.local/share/agent-proxy/
  macOS:   ~/Library/Application Support/com.agent-proxy/
```
