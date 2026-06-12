# agent-proxy-rust-compress

Token compression middleware wrapping tokenless-schema.

## 功能

- 请求压缩：使用 `SchemaCompressor` 压缩 `tools` 数组中的工具定义
  - 截断 function description（最大 256 字符）和 parameter description（最大 160 字符）
  - 移除 title、example、markdown 格式
  - 限制 enum 项数（最大 100）
- 响应压缩：使用 `ResponseCompressor` 压缩非流式 JSON 响应体
- Token 计数追踪：压缩前后 token 数估算，写入 `ctx.extensions`
- 运行时开关：通过 `enabled_flag()` 返回的 `Arc<AtomicBool>` 在 admin API 中动态启停
- Debug 日志：将压缩前后的工具摘要写入 `~/.tokenfleet-ai/agent-proxy/schema-compressdebug.log`（JSONL，200KB 截断）
- 流式响应跳过：`is_streaming` 为 `true` 时不执行响应压缩

## 关键类型

- `CompressMiddleware` — 压缩中间件，支持 `new()`（启用）和 `disabled()`（禁用）构造
- `ResponseStats` — 响应压缩统计（`pre_tokens` / `post_tokens`）

## 使用示例

```rust
use agent_proxy_rust_compress::CompressMiddleware;
use agent_proxy_rust_core::ProxyMiddleware;

// 压缩中间件必须注册在第一位（在 model-router 和 bridge 之前）
let compress = CompressMiddleware::new();

// 运行时切换
let enabled_flag = compress.enabled_flag();
// enabled_flag.store(false, std::sync::atomic::Ordering::Relaxed);

// builder.middleware(Box::new(compress))  // 第一个注册
```

## 依赖

本 crate 依赖：
- `agent-proxy-rust-core` — `ProxyMiddleware` trait、`CompressionStats`、扩展键
- `tokenless-schema` — `SchemaCompressor` 和 `ResponseCompressor` 核心压缩引擎
- `dirs` — 定位 `~/.tokenfleet-ai/` 目录写入 debug 日志

## 相关文档

- [Compress Crate 设计](../../specs/0006-compress-crate.md)

---

Part of the [agent-proxy-rust](https://github.com/TokenFleet-AI/agent-proxy-rust) workspace.
