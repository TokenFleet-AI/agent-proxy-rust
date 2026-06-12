# 核心概念

本文解释 agent-proxy-rust 的核心概念，帮助你理解系统的工作方式。

## 通道 (Channel)

通道是**上游 LLM 服务接入点的抽象**。每个通道代表一个可调用的 API 端点，包含：

- **通道 ID**（如 `deepseek`、`tokenfleet-ai`）——唯一标识符
- **Provider**（如 `deepseek`、`anthropic`、`alibaba-bailian`）——上游服务商
- **优先级**——数值越大优先级越高，决定故障转移顺序（如 `tokenfleet-ai` priority=1，`deepseek` priority=10）
- **协议配置**——每个通道支持一种或多种协议（`anthropic_messages`、`openai_chat`、`openai_responses`），各自有独立的 `baseUrl` 和可选的 `rewritePath`
- **API Key**——加密存储，运行时可通过 Admin API 热更新

当前系统有 **9 个 seed 通道**：`tokenfleet-ai`、`tokenfleet-cn`、`dashscope-coding`、`dashscope-payg`、`dashscope-token`、`deepseek`、`glm-official`、`kimi-official`、`minimax-official`。

通道支持两种计费模式：**FlatFee**（包月/预付，优先选择）和 **Metered**（按量计费，作为回退）。

> 详见 [specs/0003-channel-model](../specs/0003-channel-model.md)

## 模型 (Model)

模型定义是 LLM 模型的**元数据记录**，描述每个模型的能力和价格：

- **模型 ID**（如 `deepseek:deepseek-v4-pro`、`anthropic:claude-opus-4-8`）——格式为 `provider:model_name`
- **输入/输出价格**——每百万 token 的费率（`input_per_mtok`、`output_per_mtok`）
- **上下文窗口大小**——模型支持的最大 token 数
- **支持的特性**——`vision`（图像理解）、`reasoning`（推理）、`cache`（缓存）等

当前系统有 **36 个模型定义**，覆盖 Anthropic Claude、OpenAI GPT、DeepSeek、阿里通义千问、智谱 GLM、Kimi、MiniMax 等主流供应商。

模型定义独立于通道——同一模型可以在多个通道上提供，价格和可用性由通道的模型映射决定。

> 详见 [specs/0003-channel-model](../specs/0003-channel-model.md)

## 模型映射 (Model Mapping)

模型映射将**通用模型名路由到特定通道的实际模型**，是实现"一个 API key 访问多个 provider"的关键。

例如：用户请求 `claude-sonnet-4-6` → 通过映射 → 在 `tokenfleet-ai` 通道调用真实的 `claude-sonnet-4-6`。

每条映射包含：
- **client_name**——客户端请求中使用的模型名（如 `claude-sonnet-4-6`）
- **upstream_name**——实际发送给上游 API 的模型名（可能与 client_name 不同）
- **channel_id**——绑定的通道
- **billing**——计费方式（FlatFee 或 Metered）及定价细节
- **protocols**——可选约束，指定此映射仅在特定协议下生效

当前有 **57 条映射规则**，分布在 9 个通道上。路由中间件按通道优先级和计费类型自动选择最优通道。

> 详见 [specs/0003-channel-model](../specs/0003-channel-model.md)

## 协议桥接 (Protocol Bridge)

协议桥接在不同 LLM API 协议间进行**透明转换**。客户端和上游可能使用不同的 API 格式，桥接中间件自动处理差异。

当前支持 **6 个方向**的转换：

| 客户端协议 | 上游协议 | 转换方向 |
|---|---|---|
| Anthropic Messages | OpenAI Chat | `anthropic_to_openai` |
| Anthropic Messages | OpenAI Responses | `anthropic_to_openai_responses` |
| OpenAI Chat | Anthropic Messages | `openai_to_anthropic` |
| OpenAI Responses | Anthropic Messages | `responses_to_anthropic` |
| 相同协议 | 相同协议 | Passthrough（无转换） |

桥接支持流式（SSE）和非流式两种模式。底层转换逻辑由 `llm-bridge-core` 库实现，bridge crate 仅负责中间件集成。

例如：Claude Code 发送 Anthropic 格式请求 → 路由选择 `deepseek` 通道（OpenAI Chat 协议）→ 桥接自动将请求转为 OpenAI 格式 → 上游响应再逆向转回 Anthropic 格式。

> 详见 [specs/0007-bridge-crate](../specs/0007-bridge-crate.md)

## Token 压缩 (Compression)

压缩中间件在请求转发前**压缩上下文**以节省 token，降低成本。

- **请求压缩**：使用 `tokenless-schema` 的 `SchemaCompressor` 压缩工具定义（tool schemas），典型节省 60-70%。策略包括：截断函数描述（max 256 字符）、截断参数描述（max 160 字符）、删除 title 和 example、限制 enum 项数（max 100）
- **响应压缩**：使用 `ResponseCompressor` 精简非流式响应体
- **运行时可控**：通过 Admin API 可随时开关压缩功能

压缩统计（pre_tokens、post_tokens、saved）会写入 `ConnectionContext`，最终由 cost 中间件记录到数据库。

> 详见 [specs/0006-compress-crate](../specs/0006-compress-crate.md)

## 中间件链 (Middleware Chain)

中间件架构是系统的**核心扩展点**。所有功能模块实现统一的 `ProxyMiddleware` trait，按注册顺序组成处理链。

```rust
pub trait ProxyMiddleware: Send + Sync {
    async fn on_request(&self, req: &mut ProxyRequest, ctx: &mut ConnectionContext) -> Result<(), ProxyError>;
    async fn on_response(&self, res: &mut ProxyResponse, ctx: &ConnectionContext) -> Result<(), ProxyError>;
    fn name(&self) -> &'static str;
}
```

当前注册顺序（on_request 正向，on_response 反向）：

1. **compress** — 工具定义压缩
2. **model_alias** — 模型别名解析
3. **model-router** — 通道选择 + 模型名映射
4. **bridge** — 协议翻译

注册通过 `AgentProxyBuilder` 完成，每个中间件通过 `ConnectionContext` 的 extensions 机制传递数据（如选中的通道信息、压缩统计）。

> 详见 [specs/0002-middleware-engine](../specs/0002-middleware-engine.md)

## 故障转移 (Failover)

故障转移是**当首选通道失败时自动切换**到备用通道的机制，对用户完全透明。

选择策略（Phase 1）：
1. **FlatFee 通道优先**——按 priority 降序排列，优先选择有配额且健康的包月通道
2. **Metered 通道回退**——当 FlatFee 配额耗尽或不可用时，按优先级尝试按量通道
3. **Cooldown 重试**——当所有通道不健康时，超过 60 秒冷却期后重试

健康检测基于响应状态码：
- **2xx** — 标记健康
- **5xx / 401** — 立即标记不健康
- **429** — 记录失败，连续 3 次后标记不健康
- **其他 4xx** — 不计入通道失败（客户端问题）

通道列表通过 `ArcSwap` 原子热更新，Admin API 修改通道后无需重启即可生效。

> 详见 [specs/0003-channel-model](../specs/0003-channel-model.md)、[specs/015-health-state-machine](../specs/0015-health-state-machine.md)

## 计费追踪 (Cost Tracking)

计费中间件在每次请求完成后**自动记录 token 使用和成本**。

支持多种定价模型：
- **PerToken** — 按百万 token 计费（input/output/cache_write/cache_read/thinking 分别定价）
- **Tiered** — 分层定价（按 token 总量落入不同价格区间）
- **Credits** — 积分制
- **CharBased** — 按字符数计费（部分国内供应商）
- **PerUnit** — 按单位计费（如视频生成按次）

记录内容包括：通道、模型、token 用量、计算成本、压缩节省、请求耗时、项目路径等。支持按项目、通道、模型、时间维度聚合查询。

> 详见 [specs/0004-cost-tracking](../specs/0004-cost-tracking.md)、[specs/0018-tiered-pricing](../specs/0018-tiered-pricing.md)

---

Owner: baoyx · 版本：v1.0 · 生效日期：2026-06-12 · 最后更新：2026-06-12
