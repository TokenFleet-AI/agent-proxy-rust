# 计费上下文完整性分析 (CostRecord Context Completeness)

## 概述

本文档对 `agent-proxy-rust` 项目中 `CostRecord` 的所有 25 个字段进行逐字段来源追踪分析，标注语义模糊或信息缺失的问题，并给出改进建议。

---

## 一、完整字段-来源映射表

### 1.1 来自 `SelectedMappingInfo`（由 model-router 中间件在 `on_request` 阶段写入 context extensions）

| CostRecord 字段 | Source | 完整度 | 备注 |
|---|---|---|---|
| `channel_id` | `SelectedMappingInfo.channel_id` | ⚠️ | 见下方语义分析 |
| `pricing_snapshot_json` | `SelectedMappingInfo.pricing_snapshot_json` | ✅ | 序列化的定价快照 |
| `cost`（计算使用） | `SelectedMappingInfo.pricing` → `calc_cost()` | ✅ | FlatFee 渠道 pricing 为 None，cost=0 |
| `unit`（计算使用） | `SelectedMappingInfo.pricing` → `calc_cost()` | ✅ | "USD"/"CNY"/"credits" |

**数据流**: `storage-sqlite` → `ResolvedChannel.mappings[].billing` → `ChannelBilling::Metered { pricing }` → 序列化为 `SelectedMappingInfo.pricing` + `pricing_snapshot_json` → 存入 extension `EXT_SELECTED_MAPPING` → `cost::record()` 读取。

### 1.2 来自 `ConnectionContext` 直接字段

| CostRecord 字段 | Source | 完整度 | 备注 |
|---|---|---|---|
| `project` | `ctx.project_path`（来自 `X-Claude-Code-Project-Path` header） | ✅ | Option→"" |
| `user_id` | `ctx.user_name` 或 `CostMiddleware.user_name`（fallback） | ✅ | 两层 fallback |
| `agent_type` | `ctx.agent_type`（来自请求 header 检测） | ✅ | 枚举→字符串 |
| `session_id` | `ctx.session_id`（来自 `X-Claude-Code-Session-Id` header） | ✅ | 用于 billing 关联 |
| `timestamp` | `Utc::now().to_rfc3339()` | ✅ | 写入时间点 |

### 1.3 来自上游响应 body（`extract_usage()` 解析）

| CostRecord 字段 | Source | 完整度 | 备注 |
|---|---|---|---|
| `input_tokens` | 响应 body `usage` 中的 input/prompt tokens | ✅ | 三种格式兼容 |
| `output_tokens` | 响应 body `usage` 中的 output/completion tokens | ✅ | |
| `cache_write_tokens` | Anthropic: `cache_creation_input_tokens`; OAI: 0 | ✅ | OAI 不提供此字段 |
| `cache_read_tokens` | Anthropic: `cache_read_input_tokens`; OAI Chat: `prompt_tokens_details.cached_tokens`; OAI Responses: `input_tokens_details.cached_tokens` | ✅ | |
| `thinking_tokens` | 当前始终为 0（未从上游解析） | ⚠️ | Anthropic 支持，但当前未提取 |

### 1.4 来自 `CompressionStats`（通过 context extensions `EXT_COMPRESSION_STATS`）

| CostRecord 字段 | Source | 完整度 | 备注 |
|---|---|---|---|
| `schema_saved_tokens` | `CompressionStats.proxy_schema_saved` | ✅ | |
| `response_saved_tokens` | `CompressionStats.proxy_response_saved` | ✅ | |
| `rtk_saved_tokens` | `CompressionStats.rtk_saved` | ✅ | |
| `pre_compress_tokens` | `CompressionStats.tokenless_pre` + `proxy_req_pre` | ✅ | 合并值 |
| `post_compress_tokens` | `CompressionStats.tokenless_post` + `proxy_req_post` | ✅ | |
| `compression_tokens_saved` | 五层压缩节约之和 | ✅ | 计算值 |

### 1.5 计算字段

| CostRecord 字段 | 计算公式 | 完整度 | 备注 |
|---|---|---|---|
| `after_tokens` | `usage.input_tokens + usage.output_tokens` | ✅ | 上游实际消耗 |
| `tokens_saved` | `ctx.tokenless_saved_tokens + compression_saved` | ✅ | 全部层节约 |
| `before_tokens` | `after_tokens + total_saved` | ✅ | 压缩前估算 |
| `id` | `uuid::Uuid::now_v7()` | ✅ | UUID v7 PK |
| `compression_breakdown_json` | `ctx.tokenless_breakdown_json` | ✅ | tokenless 层明细 |

---

## 二、语义模糊 / 信息缺失字段

### 2.1 `channel_id` — 语义模糊（严重）

**现状**: `CostRecord.channel_id` = `SelectedMappingInfo.channel_id` = `ResolvedChannel.channel_id`（代理内部渠道 ID，如 `"tokenfleet-ai"`, `"deepseek"`, `"dashscope-payg"`）。

**问题**: `channel_id` 记录的是"这个 proxy 通过哪个渠道发出的请求"，而非"请求到了哪家上游提供商"。CostRecord 无法回答"哪个上游提供商（Anthropic/DeepSeek/GLM）被调用"、"上游用的什么模型名"这类问题。

**示例场景混淆**:
- channel_id=`"tokenfleet-ai"` + client_name=`"deepseek-v4-flash"` → 实际上游是 DeepSeek
- channel_id=`"tokenfleet-ai"` + client_name=`"claude-sonnet-4-6"` → 实际上游是 Anthropic
- channel_id=`"dashscope-payg"` + client_name=`"qwen3.7-max"` → 实际上游是 Alibaba

没有 `provider_name` 字段区分这些场景。

### 2.2 缺少 `client_name` — 客户端请求的模型名（严重缺失）

`SelectedMappingInfo` 包含 `client_name`（用户请求的模型名）和 `upstream_name`（发送给上游的实际模型名），但 **CostRecord 完全未存储这两个字段**。

这意味着：
- CostRecord 无法回答"用户请求了哪个模型"
- CostRecord 无法回答"上游实际使用了哪个模型"
- `CostFilter.model_name` 在 SQL 层面被错误地映射为 `channel_id = ?`（见下方 bug）
- `CostGroupBy::Model` 在 SQL 聚合时直接映射为 `channel_id` — **完全错误**

### 2.3 `thinking_tokens` — 未从 Anthropic 响应中提取

虽然 `Usage` 结构体有 `thinking_tokens` 字段，且 `Pricing::PerToken` 支持 `thinking_per_mtok`，但 `extract_anthropic()` 和 `extract_anthropic_from_usage()` 都硬编码 `thinking_tokens: 0`。Anthropic 的 `usage` 响应中实际包含 `"thinking_tokens"` 和 `"thinking_tokens_details"` 字段，但当前未解析。

---

## 三、SQL 级别的错误（bug）

### 3.1 `CostFilter.model_name` 映射错误

在 `storage-sqlite/src/lib.rs:952-954`:

```rust
if let Some(model_name) = filter.model_name {
    sql.push_str(" AND channel_id = ?");
    param_values.push(Box::new(model_name));
}
```

变量名为 `model_name`，但 WHERE 条件匹配的是 `channel_id` 列。这是因为 `cost_records` 表中根本没有模型名列。Admin API `query_cost_records` handler 将 `query.model_name` 原样传入 `CostFilter.model_name`，但这个过滤条件实际上是无效/错误的。

### 3.2 `CostGroupBy::Model` 与 `CostGroupBy::Channel` 完全等价

在 `storage-sqlite/src/lib.rs:1036-1043`:

```rust
CostGroupBy::Model | CostGroupBy::Channel => ("channel_id", "channel_id"),
```

`Model` 和 `Channel` 两种分组维度产生了相同的 SQL，因为表中没有模型名列。

---

## 四、缺少建议补充字段

建议在 `CostRecord` 中新增以下字段以修复上述问题：

| 建议字段 | 类型 | 来源 | 用途 |
|---|---|---|---|
| `client_name` | `String` | `SelectedMappingInfo.client_name` | 用户请求的模型名 |
| `upstream_name` | `String` | `SelectedMappingInfo.upstream_name` | 上游实际使用的模型名 |
| `provider_name` | `String` | 通过 `channel_id` 反查 `channels.name` | 上游提供商名称（用于计费归因） |
| `target_protocol` | `String` | `ctx.target_protocol`（`ApiFormat` 的字符串表示） | 实际使用的协议格式 |

### 建议字段对查询/聚合的影响

- 新增 `client_name` 后，`CostFilter.model_name` 可以正确过滤，`CostGroupBy::Model` 可以按用户请求的模型分组
- 新增 `provider_name` 后，可以按实际上游提供商聚合成本
- 新增 `target_protocol` 后，可以区分协议桥接场景下的流量分析

---

## 五、数据流图

```
Client Request
  │
  ├─ headers: X-Claude-Code-Project-Path → ctx.project_path
  ├─ headers: X-Claude-Code-Session-Id  → ctx.session_id
  ├─ headers: user-agent / x-agent-type  → ctx.agent_type
  └─ body: {"model": "claude-sonnet-4-6"} → model-router
       │
       ▼
ModelRouterMiddleware.on_request()
  │
  ├─ find_candidates("claude-sonnet-4-6") → [(channel, mapping)]
  ├─ select_channel() → (ResolvedChannel, ResolvedMapping)
  │
  ├─ ChannelConfig (存入 EXT_SELECTED_CHANNEL)
  │   ├─ url = resolved base_url
  │   ├─ api_key = channel API key
  │   ├─ protocol = target protocol
  │   └─ name = channel.channel_name
  │
  └─ SelectedMappingInfo (存入 EXT_SELECTED_MAPPING) ←── 来源
      ├─ channel_id          ──────────────────────────────→ CostRecord.channel_id
      ├─ client_name         ✗ 未写入 CostRecord
      ├─ upstream_name       ✗ 未写入 CostRecord
      ├─ pricing             ──────→ calc_cost() → cost + unit
      └─ pricing_snapshot_json ───→ CostRecord.pricing_snapshot_json
           │
           ▼
Upstream Response
  │
  ├─ response body usage → extract_usage() → Usage → CostRecord.{input,output,...}_tokens
  └─ CompressionStats (from EXT_COMPRESSION_STATS) → CostRecord.{schema,response,...}_saved
       │
       ▼
CostMiddleware.record()
  │
  └─ CostRecord (25 fields) → storage.insert_cost_record()
```

---

## 六、结论

### 当前可用的计费上下文信息等级

| 类别 | 级别 | 说明 |
|---|---|---|
| 谁用的 (user_id) | ✅ 完整 | 多层 fallback |
| 什么项目 (project) | ✅ 完整 | 来自请求 header |
| 什么代理类型 (agent_type) | ✅ 完整 | 来自 header 检测 |
| 库存信息 (session_id) | ✅ 完整 | 用于 billing 关联 |
| 用什么渠道发出的 (channel_id) | ⚠️ 有歧义 | 代理渠道 ID，非上游真实提供商 |
| 请求了什么模型 (client_name) | ❌ 缺失 | 需要新增 |
| 上游用了什么模型 (upstream_name) | ❌ 缺失 | 需要新增 |
| 上游是谁 (provider_name) | ❌ 缺失 | 需要新增或反查 |
| 用了什么协议 (target_protocol) | ❌ 缺失 | 可选补充 |
| costing 定价信息 | ✅ 完整 | pricing_snapshot_json 可审计 |
| 压缩节约明细 | ✅ 完整 | compression_breakdown_json |

### 最关键的三个问题

1. **缺少模型名字段**: `CostRecord` 没有 `client_name` 和 `upstream_name`，导致查询/聚合只能按 channel 分，无法按模型分。
2. **SQL 层 filter/group 错误**: `CostFilter.model_name` 实际过滤 `channel_id`，`CostGroupBy::Model` 实际上按 `channel_id` 分组，语义完全错误。
3. **`channel_id` 语义模糊**: 它记录的是 proxy 内部渠道 ID，不直接反映上游提供商，需要外部关联才能理解。
