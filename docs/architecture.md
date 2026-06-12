# 架构总览

本文描述 agent-proxy-rust 的整体架构，包含 workspace 组织、crate 依赖关系和请求生命周期。

## Workspace 结构

项目采用 Cargo workspace 组织，分为 `crates/`（library）和 `apps/`（binary）两层：

| Crate | 类型 | 职责 |
|---|---|---|
| `core` | library | `ProxyMiddleware` trait、axum server 引擎、上游转发、auth、extensions |
| `model-router` | library | 通道选择、模型名映射、健康追踪、配额管理、ArcSwap 热更新 |
| `bridge` | library | 协议翻译（Anthropic ↔ OpenAI Chat / Responses），委托 `llm-bridge-core` |
| `compress` | library | Token 压缩（请求工具定义 + 响应体），委托 `tokenless-schema` |
| `cost` | library | 计费追踪、用量提取、多种定价模型 |
| `storage` | library | 存储 trait 定义（backend-agnostic），数据模型 |
| `storage-sqlite` | library | SQLite 后端实现（WAL 模式），seed 数据管理 |
| `resilience` | library | 限流（token bucket）、重试（指数退避）、熔断（三态） |
| `server` | binary | 主应用入口，组装所有中间件，启动 axum + admin API |

## Crate 依赖图

```
┌─────────────────────────────────────────────────────────────────┐
│  server (binary)                                                │
│  ├── core                                                       │
│  ├── storage                                                    │
│  ├── storage-sqlite ──→ storage                                 │
│  ├── model-router ──→ core, storage                             │
│  ├── bridge ──→ core                                            │
│  ├── compress ──→ core                                          │
│  └── cost ──→ core, storage, model-router                       │
└─────────────────────────────────────────────────────────────────┘

外部依赖：
  core ──→ axum, reqwest (rustls), tower, tower-http, llm-bridge-core
  bridge ──→ llm-bridge-core (协议翻译核心)
  compress ──→ tokenless-schema (压缩算法)
  storage-sqlite ──→ rusqlite (bundled), r2d2, sha2, secrecy
  resilience ──→ tokio (sync, time)
```

关键设计决策：
- **core 无上游依赖**——不依赖其他 workspace crate，仅依赖外部基础库
- **storage 与实现分离**——middleware 依赖 `Box<dyn Storage>`，不知道后端是 SQLite 还是其他
- **resilience 独立**——当前 server 未直接注册 resilience 中间件，但 crate 已就绪
- **cost 依赖 model-router**——需要读取 `Pricing` 类型和 `SelectedMappingInfo`

## 请求生命周期

```
Client (Claude Code / OpenAI SDK / Codex / Gemini CLI)
   │
   │  POST /v1/messages  |  /v1/chat/completions  |  /v1/responses
   ▼
┌─────────────────────────────────────────────────────────┐
│  axum Router (server)                                   │
│  ├── Auth (proxy_api_key / proxy_token 验证)            │
│  ├── Body limit (16MB)                                  │
│  ├── Content-type validation                            │
│  └── Session correlation (X-Claude-Code-Session-Id)     │
└─────────────────────────────────────────────────────────┘
   │
   │  detect_api_format() → ctx.detected_format
   │  detect_agent_type() → ctx.agent_type
   ▼
┌─────────────────────────────────────────────────────────┐
│  on_request Chain (registration order)                  │
│                                                         │
│  ① CompressMiddleware.on_request()                      │
│     └── 压缩 tools[] 定义，节省 60-70% input tokens     │
│                                                         │
│  ② ModelAliasMiddleware.on_request()                    │
│     └── 解析模型别名（如 "smart" → "claude-sonnet-4-6"）│
│                                                         │
│  ③ ModelRouterMiddleware.on_request()                   │
│     ├── 提取 body.model → client_name                   │
│     ├── find_candidates() → 所有匹配 (channel, mapping) │
│     ├── select_channel() → Phase 1 策略选择             │
│     ├── 替换 body.model → upstream_name                 │
│     ├── 解析 target_protocol (3-step resolution)        │
│     └── 写入 ctx.extensions: ChannelConfig, Mapping     │
│                                                         │
│  ④ BridgeMiddleware.on_request()                        │
│     ├── detected_format == target_protocol → passthrough│
│     └── 不同 → 调用 llm-bridge-core 转换请求体 + path   │
└─────────────────────────────────────────────────────────┘
   │
   ▼
┌─────────────────────────────────────────────────────────┐
│  Upstream Forwarding (core::server)                     │
│  ├── reqwest::Client (HTTP/1.1, rustls)                 │
│  ├── URL = channel.base_url + rewrite_path              │
│  ├── Header: x-api-key / Authorization: Bearer          │
│  ├── Non-streaming → buffer full response               │
│  └── Streaming → SSE byte stream → client               │
└─────────────────────────────────────────────────────────┘
   │
   ▼
LLM Provider API (Anthropic / OpenAI / DeepSeek / DashScope / ...)
   │
   │  Response
   ▼
┌─────────────────────────────────────────────────────────┐
│  on_response Chain (REVERSE registration order)         │
│                                                         │
│  ④ BridgeMiddleware.on_response()                       │
│     └── 逆向转换响应体（OpenAI → Anthropic 等）         │
│                                                         │
│  ③ ModelRouterMiddleware.on_response()                  │
│     ├── 2xx → mark_healthy                              │
│     ├── 5xx/401 → mark_unhealthy_immediate              │
│     ├── 429 → record_failure                            │
│     └── FlatFee → record_usage (quota tracking)         │
│                                                         │
│  ② ModelAliasMiddleware.on_response()                   │
│     └── (passthrough)                                   │
│                                                         │
│  ① CompressMiddleware.on_response()                     │
│     └── 非流式 → ResponseCompressor 压缩响应体          │
└─────────────────────────────────────────────────────────┘
   │
   ▼
┌─────────────────────────────────────────────────────────┐
│  CostRecorder (after on_response chain)                 │
│  ├── extract_usage() — 从响应提取 token 用量            │
│  ├── calc_cost() — 根据 Pricing 计算成本                │
│  └── storage.insert_cost_record() → SQLite              │
└─────────────────────────────────────────────────────────┘
   │
   ▼
Response → Client
```

## 存储架构

```
┌────────────────────────────────────────────┐
│  Storage trait (storage crate)             │
│  ├── Provider CRUD                         │
│  ├── Model CRUD                            │
│  ├── Channel CRUD + health tracking        │
│  ├── ModelMapping CRUD                     │
│  ├── ModelAlias CRUD                       │
│  ├── CostRecord insert/query/aggregate     │
│  ├── SwitchLog                             │
│  ├── SubscriptionFee                       │
│  └── SeedManager trait                     │
│      ├── seed_init() — 本地 JSON fallback  │
│      └── seed_refresh() — 远程拉取更新     │
└────────────────────────────────────────────┘
          │
          │  implements
          ▼
┌────────────────────────────────────────────┐
│  SqliteStorage (storage-sqlite crate)      │
│  ├── rusqlite + WAL mode                   │
│  ├── r2d2 connection pool                  │
│  ├── API key 存储: SHA-256 hash + secrecy  │
│  ├── Seed JSON 嵌入编译时 + 远程刷新       │
│  └── 自动 migration (migrate on startup)   │
└────────────────────────────────────────────┘
```

存储抽象使中间件完全与后端解耦。测试中可用 in-memory 实现替代 SQLite。

## 配置系统

配置发现路径（优先级从高到低）：

```
1. CLI 参数     --db-path <PATH>
       │
2. 环境变量     AGENT_PROXY_DB_PATH
                AGENT_PROXY_API_KEY
                AGENT_PROXY_TOKEN
                AGENT_PROXY_SEED_URL
       │
3. 默认值       DB: ~/.tokenfleet-ai/token-fleet-switch/agent-proxy.db
                Listen: 127.0.0.1:11837
```

运行时配置通过 Admin API 热更新（通道优先级、API key、压缩开关等），无需重启。

> 详见 [specs/0010-configuration](../specs/0010-configuration.md)

## 安全边界

```
┌────────────────────────────────────────────────────────────┐
│  Auth Layer                                                │
│  ├── Proxy API Key (AGENT_PROXY_API_KEY)                   │
│  │   └── Header: x-api-key 或 Authorization: Bearer        │
│  ├── Proxy Token (AGENT_PROXY_TOKEN)                       │
│  │   └── Header: x-agent-token                             │
│  └── Admin API Key (ADMIN_API_KEY)                         │
│      └── 独立认证，保护管理接口                             │
├────────────────────────────────────────────────────────────┤
│  Secret Management                                         │
│  ├── secrecy::SecretString — 所有 API key 包装             │
│  ├── Debug 输出自动 redact                                  │
│  ├── API key 不落日志、不进入 URL、不出现在错误消息中       │
│  └── SQLite 存储使用 SHA-256 hash                          │
├────────────────────────────────────────────────────────────┤
│  TLS                                                       │
│  └── reqwest + rustls (aws-lc-rs backend)                  │
│      └── 所有上游连接使用 HTTPS                             │
├────────────────────────────────────────────────────────────┤
│  Input Validation                                          │
│  ├── Body size limit (16MB tower-http)                     │
│  ├── JSON depth limit (64 layers, bridge crate)            │
│  ├── Content-type validation (server layer)                │
│  └── Protocol fingerprint validation                       │
└────────────────────────────────────────────────────────────┘
```

> 详见 [specs/0008-security](../specs/0008-security.md)

## Admin API

Admin API 与 proxy 共享同一端口，独立路由，提供管理功能：

- 通道 CRUD（创建、查询、更新、删除）
- 模型映射管理
- API key 热更新（写入 `channel_api_keys` DashMap，router 即时生效）
- 压缩开关（`AtomicBool` 共享给 `CompressMiddleware`）
- 通道热重载（`ArcSwap` 原子替换通道列表）
- 成本查询与聚合
- Seed 数据状态查询与远程刷新
- 健康状态查询

> 详见 [specs/0016-admin-api-extension](../specs/0016-admin-api-extension.md)

---

Owner: baoyx · 版本：v1.0 · 生效日期：2026-06-12 · 最后更新：2026-06-12
