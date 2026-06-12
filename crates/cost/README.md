# agent-proxy-rust-cost

Cost tracking middleware for agent-proxy-rust.

## 功能

- 从上游 API 响应中提取 token 用量（支持 Anthropic、`OpenAI` Chat、`OpenAI` Responses 三种格式）
- 流式 SSE 响应用量提取：解析 `message_delta`、`response.completed`、最终 chunk 等事件
- 多定价公式支持：`PerToken`、`Credits`、`CharBased`、`PerUnit`、`Tiered`（阶梯定价）
- 费用记录写入存储后端（通过 `Storage` trait）
- 压缩节省量汇总：合并 proxy 层 + tokenless 层的 schema/response/rtk 节省
- `CostRecorder` trait 实现：在 `on_response` 链完成后由引擎调用
- 费用查询和聚合接口：按项目、模型、通道、小时、日等维度分组

## 关键类型

- `CostMiddleware` — 费用追踪中间件，实现 `CostRecorder` trait
- `Usage` — 统一的 token 用量结构（`input_tokens`、`output_tokens`、`cache_write_tokens`、`cache_read_tokens`、`thinking_tokens`）
- `extract_usage()` — 从 JSON 响应体提取用量（支持自动检测和指定格式）
- `extract_usage_sse()` — 从 SSE 流式响应提取用量
- `calc_cost()` — 根据定价公式计算费用，返回 `(cost, unit)`

## 使用示例

```rust
use std::sync::Arc;
use agent_proxy_rust_core::CostRecorder;
use agent_proxy_rust_cost::CostMiddleware;
use agent_proxy_rust_storage::Storage;

// 创建费用中间件
let storage: Arc<dyn Storage> = /* ... */;
let cost = CostMiddleware::new(storage, "default-user".to_string());

// 作为 CostRecorder 注册到 builder
// builder.cost_recorder(Arc::new(cost))

// 也可以直接查询费用
// let records = cost.query(filter).await?;
// let aggregates = cost.aggregate(group_by, range).await?;
```

## 依赖

本 crate 依赖：
- `agent-proxy-rust-core` — `CostRecorder` trait、`CompressionStats`、上下文扩展键
- `agent-proxy-rust-storage` — `Storage` trait、`CostRecord`、`CostFilter`、`CostGroupBy`
- `agent-proxy-rust-model-router` — `Pricing`、`SelectedMappingInfo`、定价类型
- `uuid` — 费用记录 ID（v7，含时间戳）
- `chrono` — 时间戳

## 相关文档

- [费用追踪设计](../../specs/0004-cost-tracking.md)
- [阶梯定价设计](../../specs/0018-tiered-pricing.md)

---

Part of the [agent-proxy-rust](https://github.com/TokenFleet-AI/agent-proxy-rust) workspace.
