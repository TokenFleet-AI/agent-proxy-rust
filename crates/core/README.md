# agent-proxy-rust-core

Core middleware trait, axum server engine, upstream forwarding, and auth for agent-proxy.

## 功能

- 定义 [`ProxyMiddleware`] trait — 整个代理系统的核心扩展点
- 基于 axum 的 HTTP 代理引擎，支持上游转发和流式响应
- 双模式认证层：简单密钥/Token 和基于角色的密钥映射
- 中间件链：`on_request` 按注册顺序执行，`on_response` 按逆序执行
- 请求体大小限制、上游连接/读取超时、连接上下文类型映射
- 多协议自动检测：Anthropic Messages、`OpenAI` Chat、`OpenAI` Responses

## 架构

```text
Client → Auth Layer → Router → handle_proxy_request
                                    ├── on_request chain (registration order)
                                    ├── forward to upstream
                                    └── on_response chain (reverse order)
```

## 关键类型

- `ProxyMiddleware` — 核心中间件 trait，定义 `on_request`、`on_response`、`on_init`、`on_shutdown` 等生命周期钩子
- `AgentProxy` / `AgentProxyBuilder` — 代理应用入口，使用 builder 模式注册中间件并启动服务
- `ProxyConfig` — 代理配置：监听地址、最大请求体、超时、认证密钥
- `ProxyRequest` / `ProxyResponse` — 代理内部请求/响应封装
- `ConnectionContext` — 每连接上下文，携带类型映射、扩展数据、压缩统计等
- `ChannelConfig` — 由 model-router 写入上下文的选定通道配置
- `ProxyError` — 统一的错误类型（`BadRequest`、`Internal`、`ChannelSelection`、`RateLimited` 等）
- `CostRecorder` — 后响应费用记录 trait，与 `ProxyMiddleware` 分离
- `CompressionStats` — 多层压缩统计
- `ModelAliasMiddleware` — 内置的模型别名映射中间件
- `AuthState` / `AgentRole` — 认证状态和角色注入

## 使用示例

```rust
use std::net::SocketAddr;
use std::sync::Arc;

use agent_proxy_rust_core::{AgentProxy, ProxyConfig, ProxyMiddleware};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = ProxyConfig::new("127.0.0.1:8787".parse()?);

    let proxy = AgentProxy::builder()
        .config(config)
        // .middleware(Box::new(my_compress_middleware))
        // .middleware(Box::new(my_router_middleware))
        // .middleware(Box::new(my_bridge_middleware))
        .build();

    proxy.serve().await?;
    Ok(())
}
```

## 模块

| 模块 | 说明 |
|---|---|
| `middleware` | `ProxyMiddleware` trait 和中间件链执行函数 |
| `server` | axum 代理引擎、路由、上游转发 |
| `auth` | 认证中间件（简单模式 + 角色映射） |
| `config` | `ProxyConfig` 配置类型 |
| `types` | `ProxyRequest`、`ProxyResponse`、`ConnectionContext`、`AgentType`、`ApiFormat` |
| `extensions` | 上下文扩展键常量（`EXT_SELECTED_CHANNEL` 等） |
| `error` | `ProxyError` 错误枚举 |
| `compression` | `CompressionStats` 多层压缩统计 |
| `report` | Tokenless 报告文件消费器 |
| `testing` | 共享测试辅助函数 |

## 依赖

本 crate 依赖：
- `axum` — HTTP 路由和中间件
- `reqwest` — 上游 HTTP 客户端（rustls-tls + stream）
- `tower` / `tower-http` — 请求体限制
- `tokio` — 异步运行时
- `llm-bridge-core` — API 格式检测（`ApiFormat` 枚举来源）
- `secrecy` — 密钥安全包装
- `serde` / `serde_json` — 序列化
- `bytes` — 零拷贝字节缓冲区
- `dashmap` — 并发映射

## 相关文档

- [数据流文档](../../docs/data-flow.md)
- [中间件引擎设计](../../specs/0002-middleware-engine.md)
- [用户指南](../../docs/user-guide.md)

---

Part of the [agent-proxy-rust](https://github.com/TokenFleet-AI/agent-proxy-rust) workspace.
