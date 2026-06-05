# 计费模块改进分析

> 分析日期: 2026-06-04
> 分析范围: 上游渠道追踪、相关模型记录、SSE 首次消息 usage 提取失效

## 一、数据流全景图

```
请求到达 server.rs
  │
  ├─ 1. 检测 API 格式 (detected_format) ─── 来自 path: /v1/messages → AnthropicMessages
  ├─ 2. 检测 Agent 类型 (agent_type) ─────── 来自 headers: x-agent-type / user-agent
  ├─ 3. 提取 session_id, project_path, user_name ── 来自 headers + tokenless report
  ├─ 4. 读取 tokenless 压缩汇总 ─────────── ctx.tokenless_saved_tokens
  │
  ▼ on_request 链
  ├─ ModelRouterMiddleware.on_request()
  │   ├─ 选择 channel + mapping
  │   ├─ 写入 EXT_SELECTED_CHANNEL ── ChannelConfig { url, name, protocol, api_key, rewrite_path }
  │   └─ 写入 EXT_SELECTED_MAPPING ── SelectedMappingInfo {
  │         channel_id: "deepseek",          ← 代理渠道 ID（DB channels.id）
  │         client_name: "claude-sonnet",    ← 用户请求的模型名
  │         upstream_name: "deepseek-v4-pro",← 实际发给上游的模型名 ✨
  │         pricing: PerToken { ... },        ← 计费公式
  │         pricing_snapshot_json: "{...}",   ← 计费快照
  │       }
  │
  ├─ BridgeMiddleware.on_request()
  │   └─ 如果协议不同: 转换 body + path, 设置 EXT_BRIDGE_REVERSE
  │
  ▼ 转发到上游
  ├─ forward_to_upstream() ── 使用 ChannelConfig.url + api_key
  │
  ▼ 响应处理
  ├─ handle_streaming_response() / handle_non_streaming_response()
  │   ├─ 缓冲完整响应体
  │   ├─ on_response 链
  │   │   ├─ BridgeMiddleware.on_response() ── 逆向转换 (upstream→client 格式)
  │   │   └─ ModelRouterMiddleware.on_response() ── 健康检查 + quota 记录
  │   │
  │   └─ CostRecording ── CostMiddleware.record(ctx, response_body)
  │       ├─ extract_usage(response_body, ctx.target_protocol) ← ⚠️ BUG HERE
  │       ├─ calc_cost(usage, pricing)
  │       └─ storage.insert_cost_record(record)
  │
  ▼ 返回给客户端
```

## 二、问题 1: CostRecord 缺少上游渠道和相关模型字段

### 现状分析

`CostRecord` 当前字段与来源映射:

| CostRecord 字段 | 来源 | 缺失？ |
|---|---|---|
| `channel_id` | `SelectedMappingInfo.channel_id` (代理渠道ID, 如 "deepseek") | ⚠️ 语义模糊 |
| `project` | `ctx.project_path` | ✅ |
| `user_id` | `ctx.user_name` | ✅ |
| `agent_type` | `ctx.agent_type` | ✅ |
| `input_tokens` / `output_tokens` | 从 response body 提取 | ✅ |
| `cost` | 根据 pricing 计算 | ✅ |
| `pricing_snapshot_json` | `SelectedMappingInfo.pricing_snapshot_json` | ✅ |
| `session_id` | `ctx.session_id` | ✅ |
| *(不存在)* | `ChannelConfig.name` (渠道显示名, 如 "DeepSeek Official") | ❌ 缺失 |
| *(不存在)* | `SelectedMappingInfo.upstream_name` (上游模型名) | ❌ 缺失 |
| *(不存在)* | `SelectedMappingInfo.client_name` (用户请求的模型名) | ❌ 缺失 |

### 关键发现

`CostRecord.channel_id` 存的是**代理渠道 ID**（DB `channels.id`），而用户需要的"上游渠道"是**渠道显示名称**（DB `channels.name`）。两者不同：

```rust
// model-router/src/lib.rs:501 — SelectedMappingInfo 构建
channel_id: channel.channel_id.clone(),  // "deepseek" — 内部ID
// 但 ChannelConfig.name 才有显示名:
// channel.channel_name.clone()          // "DeepSeek Official" — 显示名
```

`upstream_name`（上游模型名）已经在 `SelectedMappingInfo` 中可用，但从未写入 `CostRecord`。例如用户请求 `claude-sonnet`，通过 tokenfleet-ai 渠道，bridge 后实际发给 DeepSeek 的是 `deepseek-v4-pro`，这个信息没有被记录。

### 改动方案

需要改动 4 个层次：

**1. `CostRecord` 添加字段** (`crates/storage/src/types.rs`):
```rust
pub struct CostRecord {
    // ... 现有字段 ...
    /// 上游渠道显示名称 (e.g. "DeepSeek Official").
    pub upstream_channel: String,
    /// 实际发送给上游的模型名 (e.g. "deepseek-v4-pro").
    pub upstream_model: String,
}
```

**2. 数据库 Schema 迁移** (`crates/storage-sqlite/migrations/001_init.sql` + 新增 002):
```sql
ALTER TABLE cost_records ADD COLUMN upstream_channel TEXT NOT NULL DEFAULT '';
ALTER TABLE cost_records ADD COLUMN upstream_model TEXT NOT NULL DEFAULT '';
```

**3. `CostMiddleware.record()` 读取新字段** (`crates/cost/src/lib.rs`):
```rust
// 从 EXT_SELECTED_CHANNEL 读取渠道名称
let upstream_channel = ctx
    .get::<ChannelConfig>(EXT_SELECTED_CHANNEL)
    .map(|ch| ch.name.clone())
    .unwrap_or_default();

// upstream_model 已经在 SelectedMappingInfo 中
let upstream_model = mapping_info
    .map(|m| m.upstream_name.clone())
    .unwrap_or_default();
```

**4. `SqliteStorage::insert_cost_record()` 写入新字段** (`crates/storage-sqlite/src/lib.rs`):
在 INSERT 语句中添加 `upstream_channel, upstream_model` 列和对应参数。

### 额外建议

如果还想要"用户请求的模型名"（`client_name`），也应该一并添加为 `client_model` 字段。这样一条计费记录就能完整展示：用户请求了什么模型 → 路由到了哪个渠道 → 实际使用了哪个上游模型。

---

## 三、问题 2: SSE 首次回消息时 usage 提取失效

### 根本原因

**`extract_usage()` 使用了错误的目标格式。** Bridge 中间件在 `on_response` 阶段已将响应体从上游格式转换为客户端格式，但 `CostMiddleware.record()` 仍使用 `ctx.target_protocol`（上游格式）来提取 usage。

### 具体调用链

```
1. 请求: POST /v1/messages (AnthropicMessages 格式)
2. Model-router: 选择 DeepSeek channel → target_protocol = OpenaiChat
3. Bridge on_request: Anthropic → OpenAI Chat (转换请求)
4. DeepSeek 返回: OpenAI Chat SSE 流
   data: {"choices":[...], "usage": {"prompt_tokens": 100, "completion_tokens": 50}}
5. Server 缓冲完整 SSE body
6. Bridge on_response_streaming: OpenAI Chat SSE → Anthropic SSE (⚠️ 转换响应)
   data: {"type":"message_delta", "usage": {"input_tokens": 100, "output_tokens": 50}}
7. extract_usage_from_sse(&body) → {"usage": {"input_tokens": 100, "output_tokens": 50}}
8. CostMiddleware.record() → extract_usage(body, ctx.target_protocol=OpenaiChat)
   → extract_openai_chat(body)
   → 查找 usage.prompt_tokens 和 usage.completion_tokens
   → ⚠️ 找不到! (因为已转换为 Anthropic 格式的 input_tokens/output_tokens)
   → 返回 Usage::default() (全零)
```

### 问题代码位置

`crates/cost/src/lib.rs:91`:
```rust
let usage = extract_usage(response_body, ctx.target_protocol);
```

以及 `crates/cost/src/lib.rs:206-213`:
```rust
pub fn extract_usage(body: &serde_json::Value, format: Option<ApiFormat>) -> Usage {
    match format {
        Some(ApiFormat::AnthropicMessages) => extract_anthropic(body),
        Some(ApiFormat::OpenaiChat) => extract_openai_chat(body),
        // ...
    }
}
```

### 为什么非流式可能也有同样问题

对于非流式响应 (`handle_non_streaming_response`)，bridge 同样在 `on_response` 中转换了响应体格式。如果转换方向是 OpenAI→Anthropic，usage 字段名也会从 `prompt_tokens`/`completion_tokens` 变为 `input_tokens`/`output_tokens`。`extract_usage(body, OpenaiChat)` 会找不到字段。

但由于 llm-bridge-core 的非流式响应转换可能保留原生 usage 字段结构，这个 bug 在非流式场景下未必触发。SSE 场景是通过 `extract_usage_from_sse` 从 bridge 转换后的 Anthropic SSE events 中提取 usage，usage 字段名一定变为 Anthropic 格式。

### 修复方案

**方案 A: 始终使用自动检测** (推荐)
```rust
// cost/src/lib.rs:91 — 改为
let usage = extract_usage(response_body, None);  // auto-detect
```

`auto_detect_usage()` 会按顺序尝试 `prompt_tokens`（OpenAI）→ `input_tokens`（Anthropic），无论 bridge 是否转换都能正确提取。

**方案 B: 使用 detected_format 替代 target_protocol**
```rust
let usage = extract_usage(response_body, ctx.detected_format);
```

因为 bridge 转换后的响应体格式 = 客户端原始格式 = `detected_format`。但此方案依赖 bridge 一定被调用且正确转换。

**推荐方案 A**，改动最小，且自动检测对非流式场景也有保护作用。

### 修复文件清单

| 文件 | 改动 |
|---|---|
| `crates/cost/src/lib.rs:91` | `ctx.target_protocol` → `None` |

---

## 四、问题 3: 计费链路追踪完整性

### 计费链路上每个环节的信息流转

| 环节 | 输入 | 输出 |
|---|---|---|
| **请求到达** | HTTP path + headers + body | `detected_format`, `agent_type`, `session_id`, `project_path` |
| **tokenless report** | `~/.tokenfleet-ai/tokenless/reports/{sid}.jsonl` | `tokenless_saved_tokens`, `tokenless_breakdown_json`, `user_name` |
| **Model-router** | `client_name` (body.model) | `ChannelConfig` + `SelectedMappingInfo` |
| **Bridge** | `detected_format`, `target_protocol` | 转换后的 body + path, `EXT_BRIDGE_REVERSE` |
| **Upstream 转发** | `ChannelConfig.url`, `api_key`, body | HTTP response |
| **Bridge 逆转换** | upstream response body | 客户端格式 body |
| **CostMiddleware** | ctx extensions + response body | `CostRecord` → SQLite |

### CostRecord 完整字段来源表

| 字段 | 类型 | 来源表达式 | 状态 |
|---|---|---|---|
| `id` | UUID v7 | `uuid::Uuid::now_v7()` | ✅ |
| `channel_id` | String | `SelectedMappingInfo.channel_id` | ⚠️ 是代理渠道ID |
| `project` | String | `ctx.project_path` | ✅ |
| `user_id` | String | `ctx.user_name` | ✅ |
| `agent_type` | String | `ctx.agent_type.to_string()` | ✅ |
| `input_tokens` | i64 | `usage.input_tokens` | ⚠️ 见问题2 |
| `output_tokens` | i64 | `usage.output_tokens` | ⚠️ 见问题2 |
| `cache_write_tokens` | i64 | `usage.cache_write_tokens` | ⚠️ 见问题2 |
| `cache_read_tokens` | i64 | `usage.cache_read_tokens` | ⚠️ 见问题2 |
| `thinking_tokens` | i64 | `usage.thinking_tokens` | ⚠️ 见问题2 |
| `cost` | f64 | `calc_cost(&usage, pricing)` | ✅ |
| `unit` | String | pricing 中提取 | ✅ |
| `schema_saved_tokens` | i64 | `CompressionStats.proxy_schema_saved()` | ✅ |
| `response_saved_tokens` | i64 | `CompressionStats.proxy_response_saved()` | ✅ |
| `rtk_saved_tokens` | i64 | `CompressionStats.rtk_saved` | ✅ |
| `pre_compress_tokens` | i64 | `CompressionStats` 汇总 | ✅ |
| `post_compress_tokens` | i64 | `CompressionStats` 汇总 | ✅ |
| `compression_tokens_saved` | i64 | 所有压缩层汇总 | ✅ |
| `pricing_snapshot_json` | String | `SelectedMappingInfo.pricing_snapshot_json` | ✅ |
| `timestamp` | String | `Utc::now().to_rfc3339()` | ✅ |
| `session_id` | Option\<String\> | `ctx.session_id` | ✅ |
| `before_tokens` | i64 | `after_tokens + total_saved` | ✅ |
| `after_tokens` | i64 | `input_tokens + output_tokens` | ✅ |
| `tokens_saved` | i64 | `tokenless_saved + compression_saved` | ✅ |
| `compression_breakdown_json` | String | `ctx.tokenless_breakdown_json` | ✅ |
| *(建议新增)* | String | `ChannelConfig.name` | ❌ |
| *(建议新增)* | String | `SelectedMappingInfo.upstream_name` | ❌ |

### 信息传递完整性评估

**✅ 完整传递的链路:**
- session_id: headers → ctx → CostRecord
- project_path: headers/report → ctx → CostRecord
- user_name: report → ctx → CostRecord
- pricing_snapshot: model-router → SelectedMappingInfo → CostRecord
- tokenless 压缩统计: report → ctx → CostRecord

**⚠️ 可能丢失信息的链路:**
1. `channel_id` 语义模糊 — 存的是代理渠道 DB ID，没有存显示名
2. `upstream_name` — 已在 `SelectedMappingInfo` 中但未写入 CostRecord
3. `client_name` — 用户请求的原始模型名，未记录

**无信息丢失（设计如此）:**
- `channel_id` 的变化是设计行为 — model-router 选择渠道后写入 ctx，CostMiddleware 读取
- bridge 转换不影响计费字段（除了问题2中的格式bug）

---

## 五、改动方案总结

### 优先级 P0: 修复 SSE usage 提取 Bug

| # | 文件 | 改动 |
|---|---|---|
| 1 | `crates/cost/src/lib.rs:91` | `ctx.target_protocol` → `None` |

### 优先级 P1: 添加 upstream_channel 和 upstream_model

| # | 文件 | 改动 |
|---|---|---|
| 1 | `crates/storage/src/types.rs` | CostRecord 添加 `upstream_channel: String`, `upstream_model: String` |
| 2 | `crates/storage-sqlite/migrations/001_init.sql` | cost_records 表添加两列 |
| 3 | `crates/storage-sqlite/src/lib.rs` | insert_cost_record 和 query_cost_records 添加新字段 |
| 4 | `crates/cost/src/lib.rs` | record() 读取 ChannelConfig.name 和 upstream_name 填充新字段 |

### 可选 P2: 添加 client_model

如果需要在计费记录中追溯"用户请求了什么模型名"，可一并添加 `client_model` 字段，来源 `SelectedMappingInfo.client_name`。

### 不需要仓储迁移兼容性说明

由于项目尚未上线（`project not yet live` 注释），可直接修改 `001_init.sql` 中的 `CREATE TABLE cost_records` 和 `insert_cost_record` SQL，无需新增 migration 文件。
