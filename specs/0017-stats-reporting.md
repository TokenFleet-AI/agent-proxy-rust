# 0017 — Stats Reporting to tokenless

> tokenless hook → 文件队列 → agent-proxy rename-then-read → CostRecord（统一计费）。

---

## 依赖

- [tokenless/specs/0018](https://github.com/TokenFleet-AI/tokenless/blob/master/specs/0018-compression-stats-reporting.md) — ProxyReport 数据格式 + 文件队列协议

---

## 1. Session ID 提取

### 1.1 ConnectionContext 扩展

```rust
// crates/core/src/types.rs
pub struct ConnectionContext {
    // ... 现有字段 ...
    /// Session ID from X-Claude-Code-Session-Id header
    pub session_id: Option<String>,
    /// Accumulated tokens saved by tokenless hooks (from report file)
    pub tokenless_saved_tokens: u64,
    /// Raw JSON breakdown from tokenless reports
    pub tokenless_breakdown_json: Option<String>,
}
```

### 1.2 Header 提取（server.rs）

在 `handle_proxy_request` 中，创建 ctx 后立即提取：

```rust
// 从 proxy_req.headers 提取（大小写不敏感）
let session_id = proxy_req.headers
    .iter()
    .find(|(k, _)| k.as_str().eq_ignore_ascii_case("x-claude-code-session-id"))
    .and_then(|(_, v)| v.to_str().ok())
    .map(|s| s.to_string());
```

---

## 2. 报告文件消费

### 2.1 文件队列路径

```
~/.tokenfleet-ai/tokenless/reports/{session_id}.jsonl    ← tokenless hook 写入
~/.tokenfleet-ai/tokenless/reports/{session_id}.processing ← agent-proxy rename 后读取
```

### 2.2 consume_report()（core/src/report.rs）

```rust
pub(crate) fn consume_report(session_id: &str) -> Option<TokenlessAccumulator> {
    // 1. 原子 rename 认领文件
    fs::rename(&source, &target).ok()?;

    // 2. 逐行解析 JSONL
    let acc = parse_report_file(&target);

    // 3. 清理
    let _ = fs::remove_file(&target);

    acc
}
```

### 2.3 调用时机

在 `handle_proxy_request` 中，创建 ctx 后，中间件链运行之前：

```rust
if let Some(ref sid) = session_id {
    if let Some(acc) = crate::report::consume_report(sid) {
        ctx.session_id = Some(sid.clone());
        ctx.tokenless_saved_tokens = acc.total_saved;
        ctx.tokenless_breakdown_json = Some(acc.breakdown_json);
    }
}
```

---

## 3. CostRecorder 集成

### 3.1 CostRecorder trait（core/src/middleware.rs）

```rust
#[async_trait]
pub trait CostRecorder: Send + Sync + std::fmt::Debug {
    async fn record(
        &self,
        ctx: &ConnectionContext,
        response_body: &serde_json::Value,
    ) -> Result<(), ProxyError>;
}
```

### 3.2 impl in cost crate

```rust
#[async_trait]
impl CostRecorder for CostMiddleware {
    async fn record(&self, ctx: &ConnectionContext, body: &Value) -> Result<(), ProxyError> {
        self.record(ctx, body).await
    }
}
```

### 3.3 注册方式（main.rs）

```rust
let cost_middleware = Arc::new(CostMiddleware::new(storage.clone(), ...));
let proxy = AgentProxyBuilder::default()
    .cost_recorder(cost_middleware)
    .middleware(...)
    .build()?;
```

### 3.4 调用时机

在 `handle_non_streaming_response` / `handle_streaming_response` 中，`on_response` 链完成后、`build_axum_response` 之前：

```rust
if let Some(ref cr) = state.cost_recorder {
    if let Ok(body_json) = serde_json::from_slice(&proxy_resp.body) {
        if let Err(e) = cr.record(ctx, &body_json).await {
            tracing::warn!(error = %e, "cost recording failed");
        }
    }
}
```

---

## 4. CostRecord 字段（扩展后）

```
CostRecord {
    // ── 关联 ──
    session_id: Option<String>,           // 🆕 X-Claude-Code-Session-Id
    // ── 消耗 ──
    input_tokens, output_tokens, cache_write_tokens, cache_read_tokens, thinking_tokens,
    cost, unit,
    // ── 压缩 ──
    schema_saved_tokens, response_saved_tokens, rtk_saved_tokens,
    pre_compress_tokens, post_compress_tokens, compression_tokens_saved,
    // ── 计费 ──
    before_tokens: i64,                   // 🆕 压缩前估算
    after_tokens: i64,                    // 🆕 实际消耗
    tokens_saved: i64,                    // 🆕 总节省
    compression_breakdown_json: String,   // 🆕 明细
    // ── 审计 ──
    pricing_snapshot_json, timestamp,
}
```

---

## 5. 数据流总览

```
Claude Code
  │  X-Claude-Code-Session-Id: sess_abc
  ▼
tokenless hook（压缩前）
  ├─ CompressSchema (ToonHrv): saved=800
  ├─ RewriteCommand (RtkStandard): saved=150
  └─ append → ~/.tokenfleet-ai/tokenless/reports/sess_abc.jsonl

Claude Code HTTP 请求到达 agent-proxy
  ├─ handle_proxy_request: 提取 X-Claude-Code-Session-Id
  ├─ consume_report("sess_abc")
  │    → rename sess_abc.jsonl → sess_abc.processing (原子)
  │    → 解析: total_saved=950, breakdown=[{op:CompressSchema,...},{op:RewriteCommand,...}]
  │    → delete sess_abc.processing
  ├─ ctx: session_id="sess_abc", tokenless_saved_tokens=950
  ├─ CompressMiddleware: proxy_schema_saved=700
  ├─ ModelRouter: channel + pricing
  ├─ Forward → upstream
  └─ CostRecorder.record():
       ├─ tokenless_saved=950 + proxy_saved=700 = total_saved=1650
       ├─ before_tokens ≈ after_tokens + total_saved
       └─ INSERT INTO cost_records (session_id, before_tokens, after_tokens,
                                     tokens_saved, compression_breakdown_json, ...)
```

---

## 6. 实施完成状态

| 步骤 | 文件 | 状态 |
|------|------|------|
| 1 | `core/src/types.rs` | ✅ ConnectionContext 加 session_id, tokenless_saved_tokens, tokenless_breakdown_json |
| 2 | `core/src/report.rs` | ✅ 新增模块：consume_report() + rename-then-read |
| 3 | `core/src/server.rs` | ✅ 提取 X-Claude-Code-Session-Id + 消费报告 + 调用 CostRecorder |
| 4 | `core/src/middleware.rs` | ✅ CostRecorder trait 定义 |
| 5 | `cost/src/lib.rs` | ✅ CostRecorder impl + CostRecord 扩展字段 |
| 6 | `storage/src/types.rs` | ✅ CostRecord 加 session_id, before_tokens, after_tokens, tokens_saved, compression_breakdown_json |
| 7 | `storage-sqlite/migrations/006_billing_correlation.sql` | ✅ 新增列 + 索引 |
| 8 | `apps/server/src/main.rs` | ✅ CostMiddleware 注册 |
| 9 | `tokenless-cli/src/shared.rs` | ✅ append_report_to_file() |
| 10 | `tokenless-cli/src/main.rs` | ✅ tracing_subscriber 双输出 |

---

> Owner: baoyx · 版本：v2.0 · 更新：2026-06-03（反映文件队列 + CostRecorder + rename-then-read 实际实现）
