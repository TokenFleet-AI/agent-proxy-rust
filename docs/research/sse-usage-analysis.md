# SSE 流式场景下 Usage 提取失效分析

## 问题概述

用户反馈"首次回消息的计费失效"——即当上游以 SSE 流式返回响应时，第一条（也是唯一一条）消息的 token usage 没有被正确提取和计费，导致计费记录中 usage 字段全部为零。

---

## 1. `CostMiddleware.record()` 的完整调用链

### 非流式场景（正常运行路径）

```
handle_proxy_request()
  → forward_to_upstream()                       // 转发请求到上游
  → handle_non_streaming_response()             // 处理响应
    → run_on_response_chain()                    // 响应中间件链（反向注册顺序）
    → cr.record(ctx, &response_body_json)        // 调用 CostRecorder::record()
      → CostMiddleware::record()
        → extract_usage(response_body, ctx.target_protocol)  // 按协议提取 usage
        → calc_cost(&usage, &pricing)            // 计算费用
        → storage.insert_cost_record(&record)    // 写入存储
```

### 流式场景（当前问题路径）

```
handle_proxy_request()
  → forward_to_upstream()
  → handle_streaming_response()
    → upstream_resp.bytes().await                 // 缓冲完整 SSE 响应体
    → run_on_response_chain()                      // ！！响应中间件链
      → BridgeMiddleware::on_response()            // 转换 SSE 为客户端格式（如适用）
    → extract_usage_from_sse(&proxy_resp.body)     // 从转换后的 SSE 文本中提取 usage JSON
    → cr.record(ctx, &usage_json)                  // 调用 CostRecorder::record()
      → CostMiddleware::record()
        → extract_usage(response_body, ctx.target_protocol)  // 按 target_protocol 解析
        → calc_cost(&usage, &pricing)
        → storage.insert_cost_record(&record)
```

**关键时序**：`record()` 在整个 SSE 流**完全缓冲**后才被调用（逐帧转发是 Phase 2，见 server.rs:495-496 注释）。"首次回消息"指的是整个流式响应结束后的**首次（也是唯一一次）计费调用**。

---

## 2. Usage 返回零值的所有代码路径

### 路径 A：桥接场景中 usage 完全归零（主要 Bug）

这是最严重的 bug，直接影响所有通过协议桥接的 SSE 请求。

**触发条件**：客户端协议与上游协议不同（例如客户端 Anthropic Messages -> 上游 OpenAI Chat）。

**根因**：`extract_usage_from_sse()` 与 `extract_usage(response_body, target_protocol)` 之间的协议格式不匹配。

**详细步骤**：

1. 上游返回 OpenAI Chat SSE 流（包含 `usage.prompt_tokens`、`usage.completion_tokens`）
2. `run_on_response_chain()` 调用 `BridgeMiddleware::on_response()`，将 body 内全部 SSE 从 OpenAI Chat 格式转换为 Anthropic 格式
3. `extract_usage_from_sse()` 在 Anthropic SSE 中寻找 `message_delta`/`message_start` 的 `usage` 子对象
4. 提取到类似 `{"output_tokens": 150, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0}` 的 JSON（Anthropic Usage 格式，只含 output_tokens）
5. 包装为 `{"usage": {"output_tokens": 150, ...}}` 传入 `record()`
6. `record()` 调用 `extract_usage(body, ctx.target_protocol)`
7. `ctx.target_protocol = Some(ApiFormat::OpenaiChat)`（因为上游是 OpenAI Chat）
8. 进入 `extract_openai_chat(body)`（cost/lib.rs:254），查找 `body["usage"]["prompt_tokens"]`
9. **找不到 `prompt_tokens`**（实际字段是 `output_tokens`），全部返回 0
10. 结果：`Usage { input_tokens: 0, output_tokens: 0, ... }`

**代码位置**：
- server.rs:537 — `extract_usage_from_sse(&proxy_resp.body)` 传入的是桥接后的 body
- server.rs:538 — `cr.record(ctx, &body_json)` record() 用 `target_protocol` 解析
- cost/lib.rs:91 — `let usage = extract_usage(response_body, ctx.target_protocol)` 接收的是提取后的 usage 子对象

### 路径 B：Anthropic 原生流式中 `input_tokens` 丢失

**触发条件**：直接使用 Anthropic Messages 协议（无桥接），上游返回 SSE 流。

**根因**：`message_start` 的 `input_tokens` 被 `message_delta` 覆盖。

**详细步骤**：

1. Anthropic SSE 流中，`message_start` 事件包含 `usage: {input_tokens: 1500, output_tokens: 0}`
2. `message_delta` 事件包含 `usage: {output_tokens: 300, cache_read_input_tokens: 200}`（无 `input_tokens` 字段）
3. `extract_usage_from_sse()` 的逻辑是"最后出现的 usage 事件胜出"（server.rs:712 `let mut last_usage: Option<serde_json::Value> = None`）
4. `message_delta` 覆盖后，`last_usage = {"output_tokens": 300, "cache_read_input_tokens": 200}`
5. 包装为 `{"usage": {"output_tokens": 300, "cache_read_input_tokens": 200}}`
6. `extract_usage(body, ctx.target_protocol)` 中 `extract_anthropic(body)` 找不到 `usage.input_tokens`
7. 结果：`input_tokens = 0`，`output_tokens = 300`（output 侥幸正确）

**注意**：llm-bridge-core 的 `sse_output.rs:72-76` 在序列化 `message_delta` 的 usage 时也只包含 `output_tokens`、`cache_read_input_tokens`、`cache_creation_input_tokens`，明确没有 `input_tokens`。

### 路径 C：Anthropic 桥接到 OpenAI Responses 的 `message_delta` 也丢失 `input_tokens`

与路径 B 类似，因为 `message_delta` 的 usage 对象中缺少 `input_tokens`。

### 路径 D：`extract_usage_sse()` (cost crate 版) 从未被生产代码调用

cost/lib.rs:313 定义了一个独立的 `pub fn extract_usage_sse(body: &str) -> Usage` 函数，它直接解析 SSE 文本并返回 `Usage`，具有完整的合并语义。但该函数只在单元测试（cost/lib.rs:615, 626, 637）中使用，**从未在生产代码中被调用**。实际调用的是 server.rs 中 `extract_usage_from_sse()` + `CostMiddleware.record()` 的两步走路径。

### 路径 E：`auto_detect_usage()` 无法处理流式 usage JSON

当 `ctx.target_protocol` 为 `None` 时（理论上不常见），`extract_usage()` 调用 `auto_detect_usage()`。该函数检测 `usage.prompt_tokens` 或 `usage.input_tokens` 的存在。但在流式提取的场景下，`extract_usage_from_sse()` 返回的 JSON 可能只有部分字段，导致自动检测走错分支。例如 Anthropic `message_delta` 的 usage 只有 `output_tokens`，不包含 `input_tokens` 或 `prompt_tokens`，所以 `auto_detect_usage()` 直接返回 `Usage::default()`。

### 路径 F：`Usage` 的 `Default` 实现全零

cost/lib.rs:34-46 — `Usage` 的 `#[derive(Default)]` 实现使所有字段为 0。所有 `unwrap_or(0)` 调用也导致缺失字段静默归零。

---

## 3. 修复方案

### 方案 A：合并 SSE 事件中的 usage 字段（修正路径 B）

在 `extract_usage_from_sse()`（server.rs:707-753）中，不要简单地用 `last_usage = Some(u.clone())` 覆盖，而是合并 `message_start` 和 `message_delta` 的 usage 字段。

```rust
// 修复前：直接覆盖
if event.get("type").and_then(|v| v.as_str()) == Some("message_delta")
    && let Some(u) = event.get("usage")
{
    last_usage = Some(u.clone());
}

// 修复后：合并字段
fn merge_usage(into: &mut serde_json::Value, from: &serde_json::Value) {
    if let (Some(into_map), Some(from_map)) = (into.as_object_mut(), from.as_object()) {
        for (k, v) in from_map {
            // 只合并数值字段，跳过 null/非数值
            if v.is_number() && !v.as_f64().is_some_and(|f| f == 0.0) {
                into_map.insert(k.clone(), v.clone());
            }
            // 或者更简单：总是覆盖非零值
        }
    }
}
```

### 方案 B：修复桥接场景中的 usage 提取（修正路径 A）

核心问题在于 `extract_usage_from_sse()` 提取的 usage JSON 格式与 `extract_usage()` 期待的格式不匹配。

**子方案 B1**：在 `extract_usage_from_sse()` 中直接解析并返回 `Usage` 结构体，跳过 `extract_usage()` 的二次解析。

将 `handle_streaming_response()` 改为：

```rust
if let Some(ref cr) = state.cost_recorder {
    // 直接解析 SSE，返回 Usage 结构体
    let usage = cost::extract_usage_sse(&body_text);
    // 构造一个包含 usage 的 JSON Value
    let body_json = serde_json::json!({
        "usage": {
            "input_tokens": usage.input_tokens,
            "output_tokens": usage.output_tokens,
            // ... 其他字段
        }
    });
    if let Err(e) = cr.record(ctx, &body_json).await { ... }
}
```

但这会引入 cost crate 与 server crate 之间的跨 crate 依赖。

**子方案 B2**：在 `CostMiddleware.record()` 内部进行协议感知的 SSE 解析。

当 `response_body` 是 `extract_usage_from_sse()` 返回的 JSON（即只有 `usage` 字段的顶层 JSON），直接解析其中的值，不依赖 `target_protocol`。

```rust
pub async fn record(&self, ctx: &ConnectionContext, response_body: &serde_json::Value) -> Result<(), ProxyError> {
    // 如果响应体只有 usage 字段（SSE 提取场景），直接解析 usage
    if let Some(usage) = response_body.get("usage") {
        let usage = Usage {
            input_tokens: usage.get("input_tokens").and_then(Value::as_u64).unwrap_or(0),
            output_tokens: usage.get("output_tokens").and_then(Value::as_u64).unwrap_or(0),
            cache_write_tokens: usage.get("cache_creation_input_tokens").and_then(Value::as_u64).unwrap_or(0),
            cache_read_tokens: usage.get("cache_read_input_tokens").and_then(Value::as_u64).unwrap_or(0),
            thinking_tokens: usage.get("thinking_tokens").and_then(Value::as_u64).unwrap_or(0),
        };
        // ... 继续计算费用
    } else {
        // 非流式场景：使用协议格式匹配
        let usage = extract_usage(response_body, ctx.target_protocol);
        // ...
    }
}
```

**子方案 B3**：在 `extract_usage_from_sse()` 中直接将 SSE 文本和 `target_protocol` 传入，做一次性的协议感知解析。

将 `extract_usage_from_sse()` 从 server.rs 移动到 cost crate，使其能够访问协议逻辑。

### 方案 C：最简洁的修复（推荐）

**在 `extract_usage_from_sse()` 中合并 Anthropic 的 `message_start` + `message_delta` usage，然后在桥接后的记录场景中绕过 `target_protocol` 匹配（直接用字段名匹配 JSON 结构）。**

具体改动：

**server.rs `extract_usage_from_sse()`**（修正路径 B）：
- 将 `last_usage` 从 `Option<serde_json::Value>` 改为 `Usage` 结构体（用 cost crate 的字段）
- 或改用合并策略，将 `message_start` 和 `message_delta` 的 usage 合并

**cost/lib.rs `record()`**（修正路径 A）：
- 添加一条短路逻辑：如果 `response_body` 的 `usage` 字段中包含 `output_tokens`（Anthropic 格式的 SSE usage），直接从字段名匹配提取，不用 `target_protocol` 分支

### 方案 D：终极修复——让 record() 接受原始 SSE 文本

将 `CostRecorder::record()` 的签名扩展为接受原始 SSE body，让 cost 中间件自己负责解析。但这涉及到 trait 接口变更，影响较大。

---

## 4. 修复方案对照表

| 路径 | Bug | 严重程度 | 推荐修复 |
|------|-----|----------|---------|
| A | 桥接场景 usage 全零 | **高** | 在 `record()` 中不依赖 `target_protocol` 解析流式 usage，直接用字段名匹配 |
| B | Anthropic 原生 `input_tokens` 丢失 | **中** | 合并 `message_start` 和 `message_delta` 的 usage 字段 |
| E | `auto_detect_usage` 无法处理不完整 usage | **低** | 增强 field 检测逻辑 |
| D | `extract_usage_sse()` 死代码 | **低** | 移除或改为真实调用 |

**首选方案**：组合方案 C。在 `extract_usage_from_sse()` 中实现字段合并，然后在 `CostMiddleware.record()` 中针对纯 usage 子对象做直接字段匹配，不经过协议格式分支。

---

## 5. "首次回消息"的语义确认

"首次回消息"指的是：客户端发送一个 stream=true 的请求后，上游通过 SSE 流式返回完整响应。当流结束后（整个 SSE body 被缓冲），proxy 调用 `record()` 进行计费。由于上述 bug，这个计费调用返回的 usage 全部为零，导致用户看到的"首次回消息"没有正确计费。

在 Telephony / billing 语境中，这不是"第一次 chunk"而是"第一个完整的请求-响应周期"的计费。
