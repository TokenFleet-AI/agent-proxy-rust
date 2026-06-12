# agent-proxy-rust-storage

Backend-agnostic storage trait and data types for agent-proxy-rust.

## 功能

- 定义 `Storage` trait — 所有存储后端的统一接口
- 定义 `SeedManager` trait — 种子数据（providers、models、channels、mappings）初始化和远程刷新
- 所有数据类型的规范定义：`Provider`、`Model`、`Channel`、`ModelMapping`、`CostRecord` 等
- 统一错误类型 `StorageError`：`Backend`、`NotFound`、`Duplicate`、`Connection`、`Migration`
- 中间件 crate 依赖 `Box<dyn Storage>` 注入，无需知道具体后端实现

## 关键类型

### Trait

- `Storage` — 后端无关的存储 trait，所有方法 async 且返回 `Result<T, StorageError>`
- `SeedManager` — 种子数据管理 trait：`seed_init`、`seed_refresh`、`seed_status`、`seed_check_remote`

### 数据模型

- `Provider` — 上游 AI 提供商（`id`、`name`、`created_at`）
- `Model` — 模型定义（`client_name`、定价、`context_window`、`channel_count`）
- `Channel` — 上游通道（API key、协议、健康状态、计费类型、优先级）
- `ModelMapping` — 模型映射（`client_name` → `upstream_name` + billing + pricing_json）
- `ModelAlias` / `ModelAliasRequest` — 模型别名映射

### 费用与聚合

- `CostRecord` — 单次请求的费用记录（token 用量、费用、压缩节省、定价快照）
- `CostFilter` — 查询过滤器（project、model、channel、时间范围、limit/offset）
- `CostGroupBy` — 聚合维度（Project、Model、Channel、ProjectModelMonth、ProjectModelHour、Hourly、Daily）
- `CostAggregate` — 聚合结果
- `TimeRange` — 时间范围

### 其他

- `SwitchLog` — 通道切换日志
- `SubscriptionFee` — 月度订阅费用
- `AvailableChannelInfo` / `AvailableModelInfo` — 可用通道信息（供 admin API）
- `SeedStatus` / `SeedManifest` — 种子数据状态和清单
- `ProtocolEntry` — 通道协议条目（`protocol`、`base_url`、`rewrite_path`）

## `Storage` trait 方法概览

| 领域 | 方法 |
|---|---|
| Provider | `list_providers`、`get_provider` |
| Model | `list_models`、`get_model` |
| Channel | `list_channels`、`get_channel`、`upsert_channel`、`set_channel_enabled`、`set_channel_api_key`、`update_channel`、`delete_channel`、`mark_channel_healthy`、`record_channel_failure` |
| Mapping | `list_mappings`、`upsert_mapping`、`set_mapping_enabled`、`delete_mapping`、`list_all_mappings` |
| Model Alias | `list_model_aliases`、`get_model_alias_target`、`upsert_model_alias`、`delete_model_alias`、`set_model_alias_enabled` |
| Cost | `insert_cost_record`、`query_cost_records`、`aggregate_costs`、`prune_cost_records`、`list_projects` |
| Switch Log | `insert_switch_log`、`query_switch_logs` |
| Subscription | `insert_subscription_fee`、`query_subscription_fees` |
| Available | `list_available_channels` |
| Lifecycle | `migrate`、`health_check`、`max_connections` |

## 使用示例

```rust
use std::sync::Arc;
use agent_proxy_rust_storage::{Storage, StorageError, Provider};

// 中间件通过 Arc<dyn Storage> 注入
async fn list_all_providers(storage: &Arc<dyn Storage>) -> Result<Vec<Provider>, StorageError> {
    storage.list_providers().await
}
```

## 依赖

本 crate 依赖：
- `async-trait` — 异步 trait 支持
- `chrono` — 时间类型（带 serde 支持）
- `secrecy` — API Key 安全包装
- `serde` / `serde_json` — 序列化
- `thiserror` — 错误类型派生

## 相关文档

- [存储抽象设计](../../specs/0014-storage-abstraction.md)
- [远程种子数据设计](../../specs/0019-remote-seed-data.md)

---

Part of the [agent-proxy-rust](https://github.com/TokenFleet-AI/agent-proxy-rust) workspace.
