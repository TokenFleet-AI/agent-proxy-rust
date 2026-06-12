# agent-proxy-rust-model-router

Model routing and channel selection middleware for agent-proxy-rust.

## 功能

- 根据客户端请求的 `model` 字段选择最优上游通道（channel）
- Phase 1 选择策略：`FlatFee` 通道优先（配额内），`Metered` 通道作为 fallback
- 通道健康追踪：二元模型 + 60 秒冷却期，3 次连续失败标记 Unhealthy
- 多协议解析：3 步策略确定目标协议（`force_protocol` > 客户端匹配 > 第一个协议）
- 协议-模型兼容性检查：当 mapping 声明协议约束时自动切换
- 配额追踪：`FlatFee` 通道按 token 消耗量跟踪月度配额
- 热重载：通过 `ArcSwap` 原子替换通道列表，admin API 可即时生效
- API Key 运行时覆盖：admin API 更新密钥后无需重启
- 上游响应状态码处理：5xx/401 立即标记不健康，429 记录失败，4xx 忽略

## 关键类型

- `ModelRouterMiddleware` — 通道选择和模型路由中间件
- `ResolvedChannel` — 从存储层解析的通道及其模型映射
- `ResolvedMapping` — 模型映射（client_name → upstream_name + billing）
- `SelectedMappingInfo` — 写入上下文的选定映射信息（供 cost 模块使用）
- `ChannelBilling` — 计费类型枚举（`FlatFee` / `Metered`）
- `Pricing` — 定价公式（`PerToken`、`Credits`、`CharBased`、`PerUnit`、`Tiered`）
- `Quota` / `QuotaUsage` — 配额定义和消耗追踪
- `ChannelState` / `ChannelHealth` — 通道健康状态机
- `reload_channels_from_storage()` — 从存储层热重载通道列表

## 使用示例

```rust
use std::sync::Arc;
use agent_proxy_rust_core::ProxyMiddleware;
use agent_proxy_rust_model_router::ModelRouterMiddleware;
use agent_proxy_rust_storage::Storage;

async fn build_router(storage: Arc<dyn Storage>) -> Box<dyn ProxyMiddleware> {
    let router = ModelRouterMiddleware::from_storage(storage)
        .await
        .expect("failed to load channels");
    Box::new(router)
}
```

## 依赖

本 crate 依赖：
- `agent-proxy-rust-core` — `ProxyMiddleware` trait、`ProxyError`、上下文扩展键
- `agent-proxy-rust-storage` — `Storage` trait、`ProtocolEntry`
- `arc-swap` — 原子替换通道列表（热重载）
- `dashmap` — 并发健康状态映射和配额追踪
- `secrecy` — API Key 安全包装

## 相关文档

- [通道与模型设计](../../specs/0003-channel-model.md)
- [中间件引擎设计](../../specs/0002-middleware-engine.md)

---

Part of the [agent-proxy-rust](https://github.com/TokenFleet-AI/agent-proxy-rust) workspace.
