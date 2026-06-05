# 上游渠道追踪分析

## 1. 数据流全景图

```
┌─────────────────────────────────────────────────────────────────────────┐
│  请求到达 (ProxyRequest)                                                │
│    body: {"model":"claude-sonnet-4-6", ...}                             │
│     │                                                                   │
│     ▼                                                                   │
│  ModelRouterMiddleware::on_request()                                    │
│     │                                                                   │
│     ├─1. 提取 client_name = "claude-sonnet-4-6"                        │
│     ├─2. find_candidates() → 匹配 ModelMappings                          │
│     ├─3. select_channel() → 选中 (ResolvedChannel, ResolvedMapping)     │
│     ├─4. 替换 body.model = mapping.upstream_name                        │
│     ├─5. 写入 ctx.extensions:                                           │
│     │    ┌─────────────────────────────────────────────────────┐        │
│     │    │ EXT_SELECTED_CHANNEL → ChannelConfig {              │        │
│     │    │   url, api_key, protocol,                           │        │
│     │    │   name: channel.channel_name,  ← "DeepSeek Official"│        │
│     │    │   rewrite_path                                      │        │
│     │    │ }                                                   │        │
│     │    │                                                     │        │
│     │    │ EXT_SELECTED_MAPPING → SelectedMappingInfo {        │        │
│     │    │   channel_id:     channel.channel_id,  ← "deepseek" │        │
│     │    │   mapping_id:     mapping.mapping_id                │        │
│     │    │   client_name:    mapping.client_name               │        │
│     │    │   upstream_name:  mapping.upstream_name ← "deepseek-│        │
│     │    │                                v4-flash"            │        │
│     │    │   is_flat_fee, pricing, pricing_snapshot_json       │        │
│     │    │ }                                                   │        │
│     │    └─────────────────────────────────────────────────────┘        │
│     │                                                                   │
│     ▼                                                                   │
│  上游 API 请求 → 响应                                                     │
│     │                                                                   │
│     ▼                                                                   │
│  CostMiddleware::record()                                               │
│     ├─ ctx.get::<SelectedMappingInfo>(EXT_SELECTED_MAPPING)              │
│     │   → 拿到 channel_id, upstream_name, pricing 等                    │
│     ├─ ctx.get::<ChannelConfig>(EXT_SELECTED_CHANNEL)                    │
│     │   → 拿到 channel_name (目前没被使用!)                               │
│     ├─ extract_usage(response_body, target_protocol) → Usage            │
│     ├─ calc_cost(&usage, &pricing) → (cost, unit)                       │
│     └─ 构造 CostRecord → 写入 storage                                   │
│     │                                                                   │
│     ▼                                                                   │
│  Storage (SQLite)                                                       │
│    INSERT INTO cost_records (...)                                        │
└─────────────────────────────────────────────────────────────────────────┘
```

## 2. 关键数据结构字段对应关系

### 2.1 ResolvedMapping (model-router 内部结构, 不可序列化)

| 字段            | 含义                                     |
|-----------------|------------------------------------------|
| `mapping_id`    | 模型映射唯一ID                           |
| `client_name`   | 客户端侧的模型名 (如 "claude-sonnet")    |
| `upstream_name` | 上游 API 模型名 (如 "claude-sonnet-4-7") |
| `billing`       | ChannelBilling 计费模式                  |
| `allowed_protocols` | 协议约束, 空=全部                   |

### 2.2 SelectedMappingInfo (存储在 ctx.extensions)

| 字段                    | 来源                | 含义                                 |
|-------------------------|---------------------|--------------------------------------|
| `channel_id`            | channel.channel_id  | 渠道ID (如 "deepseek")              |
| `mapping_id`            | mapping.mapping_id  | 映射ID                               |
| `client_name`           | mapping.client_name | 客户端模型名                         |
| `upstream_name`         | mapping.upstream_name | **上游模型名 (需要的 upstream_model)** |
| `is_flat_fee`           | mapping.billing     | 是否包月                             |
| `pricing`               | mapping.billing     | 计费定价快照                         |
| `pricing_snapshot_json` | 序列化的pricing     | 审计用                                |

### 2.3 ChannelConfig (存储在 ctx.extensions)

| 字段            | 来源                     | 含义                        |
|-----------------|--------------------------|-----------------------------|
| `url`           | protocol.base_url        | 上游 URL                    |
| `api_key`       | channel.api_key          | API 密钥                    |
| `protocol`      | target_protocol          | 协议格式                    |
| `name`          | channel.channel_name     | **渠道名称 (可作 upstream_channel)** |
| `rewrite_path`  | protocol.rewrite_path    | 路径重写                     |

### 2.4 CostRecord (storage 层, 序列化/持久化)

| 字段 (当前)              | 来源                                    | SQLite 列              |
|--------------------------|-----------------------------------------|------------------------|
| `id`                     | uuid::Uuid::now_v7()                    | id TEXT PK             |
| `channel_id`             | mapping_info.channel_id                 | channel_id TEXT        |
| `project`                | ctx.project_path                        | project TEXT           |
| `user_id`                | ctx.user_name                           | user_id TEXT           |
| `agent_type`             | ctx.agent_type                          | agent_type TEXT        |
| `input_tokens`           | usage.input_tokens                      | input_tokens INTEGER   |
| `output_tokens`          | usage.output_tokens                     | output_tokens INTEGER  |
| `cache_write_tokens`    | usage.cache_write_tokens               | cache_write_tokens INTEGER |
| `cache_read_tokens`     | usage.cache_read_tokens                | cache_read_tokens INTEGER |
| `thinking_tokens`        | usage.thinking_tokens                   | thinking_tokens INTEGER|
| `cost`                   | calc_cost()                             | cost REAL              |
| `schema_saved_tokens`    | CompressionStats                        | schema_saved_tokens INTEGER |
| `response_saved_tokens`  | CompressionStats                        | response_saved_tokens INTEGER |
| `rtk_saved_tokens`       | CompressionStats                        | rtk_saved_tokens INTEGER |
| `pre_compress_tokens`    | CompressionStats                        | pre_compress_tokens INTEGER |
| `post_compress_tokens`   | CompressionStats                        | post_compress_tokens INTEGER |
| `compression_tokens_saved` | CompressionStats                      | compression_tokens_saved INTEGER |
| `unit`                   | pricing.currency                        | unit TEXT               |
| `pricing_snapshot_json`  | mapping_info.pricing_snapshot_json      | pricing_snapshot_json TEXT |
| `timestamp`              | Utc::now()                              | timestamp TEXT          |
| `session_id`             | ctx.session_id                          | session_id TEXT         |
| `before_tokens`          | computed                                | before_tokens INTEGER   |
| `after_tokens`           | computed                                | after_tokens INTEGER    |
| `tokens_saved`           | computed                                | tokens_saved INTEGER    |
| `compression_breakdown_json` | ctx.tokenless_breakdown_json      | compression_breakdown_json TEXT |

### 2.5 当前缺失的数据字段

| 需要添加的字段          | 可用数据源                                               | 说明                     |
|-------------------------|----------------------------------------------------------|--------------------------|
| `upstream_channel`      | `ctx.get::<ChannelConfig>(EXT_SELECTED_CHANNEL).name`    | 渠道名称 (友好可读)      |
| `upstream_model`        | `mapping_info.upstream_name` (来自 SelectedMappingInfo)  | 上游 API 模型名          |

## 3. 分析与方案

### 3.1 现状

当前 CostRecord 的 `channel_id` 字段存储的是 `mapping_info.channel_id` (如 "deepseek")，这是数据库中 channels 表的 ID 字段，是一个技术标识符，不是人类可读的渠道名称。同时，**完全没有存储上游模型名**。

但是两个字段所需的数据已经在运行时上下文中可用：
- **上游渠道名称**: `ChannelConfig.name` (通过 `EXT_SELECTED_CHANNEL` 扩展键获取)
- **上游模型名称**: `SelectedMappingInfo.upstream_name` (通过 `EXT_SELECTED_MAPPING` 扩展键获取)

### 3.2 改动方案

#### 步骤 A: 在 CostRecord 结构体中添加字段

**文件**: `crates/storage/src/types.rs`

在 `CostRecord` 结构体中添加两个新字段：

```rust
/// Upstream channel display name (e.g. "DeepSeek Official").
#[serde(default)]
pub upstream_channel: String,
/// Upstream model name sent to the API (e.g. "claude-sonnet-4-7").
#[serde(default)]
pub upstream_model: String,
```

这两个字段都使用 `#[serde(default)]` 确保反序列化时向后兼容（存量记录没有这些字段也能正常读取）。

#### 步骤 B: 在 CostRecord 构建处填充字段

**文件**: `crates/cost/src/lib.rs` 中的 `record()` 方法

在 `let mapping_info = ctx.get::<SelectedMappingInfo>(EXT_SELECTED_MAPPING);` 之后，添加：

```rust
let upstream_model = mapping_info.map_or(String::new(), |m| m.upstream_name.clone());
```

然后在构造 `CostRecord` 时，从 `ChannelConfig` 获取渠道名称：

```rust
// 已有的 channel_id 保持不变
let channel_id = mapping_info.map_or(String::new(), |m| m.channel_id.clone());
// 新增: 从 ChannelConfig 获取渠道显示名称
let channel_config = ctx.get::<ChannelConfig>(EXT_SELECTED_CHANNEL);
let upstream_channel = channel_config.map_or(String::new(), |c| c.name.clone());
```

在 `CostRecord` 字面量中添加：

```rust
upstream_channel,
upstream_model,
```

#### 步骤 C: 新增 SQLite 迁移

**文件**: 新建 `crates/storage-sqlite/migrations/002_upstream_channel_model.sql`

```sql
-- V2: Add upstream_channel and upstream_model to cost_records
ALTER TABLE cost_records ADD COLUMN upstream_channel TEXT NOT NULL DEFAULT '';
ALTER TABLE cost_records ADD COLUMN upstream_model TEXT NOT NULL DEFAULT '';
```

#### 步骤 D: 更新迁移逻辑

**文件**: `crates/storage-sqlite/src/lib.rs` 中的 `migrate()` 方法

```rust
// 在现有的 if version < 1 { ... } 块之后添加:
if version < 2 {
    conn.execute_batch(MIGRATION_V2)
        .map_err(|e| StorageError::Migration(e.to_string()))?;
}

// 更新 version 为 2
conn.pragma_update(None, "user_version", 2)
```

同时需要新增常量：

```rust
const MIGRATION_V1: &str = include_str!("../migrations/001_init.sql");
const MIGRATION_V2: &str = include_str!("../migrations/002_upstream_channel_model.sql");
```

#### 步骤 E: 更新 INSERT/UPDATE/SELECT SQL

**文件**: `crates/storage-sqlite/src/lib.rs`

1. **insert_cost_record** 方法: INSERT SQL 增加 `upstream_channel` 和 `upstream_model` 两列，VALUES 增加两个参数占位符和绑定。

2. **query_cost_records** 方法: SELECT SQL 增加这两列，`CostRecord` 构造增加两个 `row.get()` 调用。

### 3.3 变更文件清单

| 文件 | 改动类型 | 说明 |
|------|---------|------|
| `crates/storage/src/types.rs` | 字段添加 | CostRecord 增加 `upstream_channel` 和 `upstream_model` |
| `crates/cost/src/lib.rs` | 逻辑添加 | record() 方法中读取并填充这两个字段 |
| `crates/storage-sqlite/migrations/002_upstream_channel_model.sql` | 新增文件 | V2 迁移: ALTER TABLE cost_records ADD COLUMN |
| `crates/storage-sqlite/src/lib.rs` | 多处修改 | 新增 `MIGRATION_V2` 常量、更新 migrate() 逻辑、更新 INSERT/SELECT SQL |

### 3.4 兼容性说明

- 所有新字段使用 `#[serde(default)]` + 数据库列 `NOT NULL DEFAULT ''`，确保已有记录和旧代码兼容。
- `CostFilter` 无需改动 — 用户可以通过已有的 `channel_name` 或新的数据查询接口筛选。
- `CostGroupBy` 可以后续添加 `UpstreamChannel` 和 `UpstreamModel` 维度，但不是本阶段必需。
