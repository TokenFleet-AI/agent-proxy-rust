# agent-proxy-rust-bridge

Protocol translation bridge middleware for agent-proxy-rust.

## 功能

- 在 Anthropic Messages、`OpenAI` Chat Completions、`OpenAI` Responses 三种 API 格式之间转换
- 请求转换：根据 `ctx.detected_format` 和 `ctx.target_protocol` 自动决定转换方向
- 响应转换：反向转换上游响应为客户端期望的格式
- 流式 SSE 转换：批量解析 SSE 帧并转换为目标协议的 SSE 流
- 输入验证：JSON 深度检查防止栈溢出
- 上游错误响应透传：不转换包含 `{"error": ...}` 的上游错误
- Passthrough 优化：相同协议时跳过转换

## 关键类型

- `BridgeMiddleware` — 协议桥中间件（无状态，`Default` 可构造）
- `bridge_stream()` — 流式 SSE 协议转换适配器，将上游字节流包装为协议转换后的流

## 转换方向

| 客户端 | 上游 | 方向 |
|---|---|---|
| Anthropic | OpenAI Chat | `AnthropicToOpenai` |
| Anthropic | OpenAI Responses | `AnthropicToResponses` |
| OpenAI Chat | Anthropic | `OpenaiToAnthropic` |
| OpenAI Responses | Anthropic | `ResponsesToAnthropic` |
| 相同 | 相同 | `Passthrough` |

## 使用示例

```rust
use agent_proxy_rust_bridge::BridgeMiddleware;
use agent_proxy_rust_core::ProxyMiddleware;

// Bridge 是无状态的，直接构造即可
let bridge = BridgeMiddleware::new();

// 注册时必须在 model-router 之后（需要 ctx.target_protocol 已设置）
// builder
//   .middleware(Box::new(router))   // model-router 先设置 target_protocol
//   .middleware(Box::new(bridge))   // bridge 在 model-router 之后
```

## 依赖

本 crate 依赖：
- `agent-proxy-rust-core` — `ProxyMiddleware` trait、`ProxyRequest`/`ProxyResponse`、`ApiFormat`
- `llm-bridge-core` — 核心协议转换引擎（`transform::*`、`stream::*`）
- `futures` / `tokio-stream` — 流式 SSE 转换
- `serde_json` — JSON 解析和序列化

## 相关文档

- [Bridge Crate 设计](../../specs/0007-bridge-crate.md)
- [中间件引擎设计](../../specs/0002-middleware-engine.md)

---

Part of the [agent-proxy-rust](https://github.com/TokenFleet-AI/agent-proxy-rust) workspace.
