# 0015 - Health State Machine

## 1. 概述

agent-proxy-rust 的通道健康检测采用三层机制：

1. **被动检测** — 每次请求转发时根据上游 HTTP 状态码自动更新健康状态
2. **主动探测** — 通过 admin API 触发或设置 key 时自动触发，发送最小成本请求验证连通性
3. **限流恢复** — 429 限流使用 30 秒短 cooldown，快速自动重试

## 2. 健康状态定义

| 状态 | 含义 | 触发条件 | 路由行为 |
|------|------|----------|----------|
| `Healthy` | 通道正常 | 请求成功 2xx / 探测成功 | 正常选择 |
| `Degraded` | 有故障但可重试 | 探测失败 (401/503 等) | 排除在选择之外 |
| `Cooldown` | 多次失败进入冷却 | ≥3 次连续故障 | 排除，冷却结束后自动恢复 |
| `Unavailable` | 无 API key | key 为空 | 永久排除，无 key 不使用 |
| `Unhealthy` | 运行时标记不健康 | 5xx 错误 / 立即标记 | 排除，等 is_tryable 恢复 |

> 注：`health_status` 存储在 DB 为字符串（Degraded/Cooldown），运行时 `ChannelHealth` 为 Healthy/Unhealthy 二元枚举。

## 3. 状态转换

```text
                        ┌─────────┐
            ┌──────────►│ Healthy │◄──────────┐
            │ success   │ (正常)  │           │
            │           └────┬─────┘           │
            │                │                  │
            │        probe 失败 / 探测不可达    │
            │           ┌────▼─────┐    ───────┘
            │  success  │ Degraded │   手动 / 主动探测成功
            │           │ (非致命) │
            │           └────┬─────┘
            │                │
            │           ≥3 次连续故障
            │           ┌────▼──────┐
            │  success  │ Cooldown  │   ────────────
            │ (手动恢复) │ (冷却期) │   probe 成功自恢复
            │           └──────────┘
            │
            │           5xx / 401
            │           ┌────▼──────┐
            └───────────│Unavailable │
             success    │ (立即下线) │
                        └──────────┘
```

## 4. 故障分类

| HTTP Status | 效果 | 冷却策略 |
|-------------|------|----------|
| **5xx (500-599)** | 立即 `Unhealthy` | 指数退避 (60s/300s/900s) |
| **401** (Unauthorized) | 立即 `Unhealthy` | 指数退避 (60s/300s/900s) |
| **429** (Rate Limited) | `mark_rate_limited` | **30 秒固定冷却** |
| **4xx** (非 429/401) | 不视为故障 | 无影响 |

**关键变化**: 429 使用独立的 `mark_rate_limited` 而非 `record_failure`，冷却时间固定 30 秒，不累积故障次数。这意味着即使连续 429，每次冷却后都只等 30 秒即可重试，直到恢复。

## 5. 真实健康探测

### 5.1 探测机制

`apps/server/src/health_probe.rs` — `probe_channel()`

1. 从 storage 获取 channel 的所有 enabled mapping
2. 查询所有模型的 `price_input + price_output`，选择**价格最低**的模型
3. 根据 channel 第一个 protocol 构造最小请求:
   - `openai_chat` → `POST {base_url}/v1/chat/completions` `{"model":"...", "max_tokens":1, "messages":[{"role":"user","content":"hi"}]}`
   - `anthropic_messages` → `POST {base_url}/v1/messages` (同上 + `x-api-key` / `anthropic-version` header)
   - `openai_responses` → `POST {base_url}/v1/responses` `{"model":"...", "input":"hi"}`
4. 使用独立 reqwest client (connect 5s, read 10s)
5. 根据响应判断:
   - `200-299` → `Healthy`
   - `401/403` → `InvalidKey` (可达但 key 无效)
   - `429` → `RateLimited` (key 有效但被限流)
   - 超时/连接失败 → `Unreachable`
   - 其他 → `Unknown`

### 5.2 触发时机

| 触发方式 | 说明 |
|----------|------|
| **设置 API Key** | `PUT /admin/channels/{id}/api-key` 设置非空 key 后自动探测 |
| **手动触发** | `POST /admin/channels/{id}/probe` 管理员手动触发 |
| **被动检测** | 每次上游请求的响应自动更新健康状态 |

### 5.3 探测结果对健康状态的影响

| 探测结果 | 内存 (health_map) | 数据库 (health_status) |
|----------|-------------------|----------------------|
| `Healthy` | 移除 unhealthy 标记 | 设为 "Healthy" |
| `RateLimited` | 移除 unhealthy 标记 | 设为 "Healthy" (key 有效) |
| `InvalidKey` | 标记 unhealthy | "Degraded" (通过 record_failure) |
| `Unreachable` | 标记 unhealthy | "Degraded" |
| `Unknown` | 标记 unhealthy | "Degraded" |
| `NoModels` | 不修改 | 不修改 |
| `NoProtocols` | 不修改 | 不修改 |

`RateLimited` 视为健康: key 是有效的，上游只是限制了速率，正常业务请求能通过。

## 6. 路由集成

```rust
// 正常选择: 排除 unhealthy 通道
if self.is_healthy(&ch.channel_id) {
    return Ok((ch, m));
}

// 全部 unhealthy 时的兜底: 检查 cooldown 是否已过
// 429 限流: 1 次失败 → 30s,  非 429: 指数退避 60s/300s/900s
if self.is_tryable_past_cooldown(&ch.channel_id) {
    return Ok((ch, m));
}
```

`is_tryable_past_cooldown()` 逻辑:
- 连续失败 ≤ 1 次 → 30 秒冷却 (针对限流)
- 连续失败 ≥ 2 次 → 指数退避 (60s / 300s / 900s)

## 7. Admin API 端点

| 端点 | 说明 |
|------|------|
| `GET /admin/channels/{id}` | 返回通道当前健康状态 |
| `POST /admin/channels/{id}/healthy` | 手动标记为健康 |
| `POST /admin/channels/{id}/failure` | 记录一次故障 (测试用) |
| `POST /admin/channels/{id}/probe` | **新增** — 手动触发真实健康探测 |
| `PUT /admin/channels/{id}/api-key` | 设置 key + **自动触发探测** |
| `GET /admin/health` | 聚合: healthyChannels / totalChannels |

## 8. 测试场景

| 场景 | 预期 |
|------|------|
| 5xx → 立即 Unhealthy | 指数退避冷却 |
| 401 → 立即 Unhealthy | 指数退避冷却 |
| 429 → mark_rate_limited | 30s 冷却 |
| 连续 429 → 每次 30s | 冷却连续 30s 不变长 |
| 设置 key → 自动探测 | 返回探测结果 |
| 手动 probe → 返回结果 | 更新健康状态 |
| probe invalid_key → Degraded | 内存标记 unhealthy |
| 冷却过期 → 自动恢复 | is_tryable 返回 true |
| 非 429 4xx → 无影响 | 不计数故障 |

## 9. 代码位置

| 模块 | 文件 |
|------|------|
| 健康探测核心 | `apps/server/src/health_probe.rs` |
| Admin 探测集成 | `apps/server/src/admin.rs` |
| 运行时健康状态 | `crates/model-router/src/types.rs` (ChannelState, ChannelHealth) |
| 路由健康检查 | `crates/model-router/src/lib.rs` (is_healthy, mark_rate_limited) |
| DB 健康字段 | `crates/storage/src/types.rs` (ChannelHealthStatus) |
