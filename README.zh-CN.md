# agent-proxy-rust

AI 编码代理（Claude Code、Codex、Gemini CLI）与上游 API 提供商之间的可组合中间件代理。

```
客户端 (Claude Code / Codex / Gemini CLI)
        │
        ▼
┌──────────────────────────────────────┐
│           agent-proxy-rust            │
│                                      │
│  compress → route → bridge → cost    │
│                                      │
│  • 压缩工具定义                        │
│  • 选择最优渠道                        │
│  • 协议转换                           │
│  • 按项目追踪成本                      │
└──────────────────────────────────────┘
        │
        ▼
上游 API (Anthropic / OpenAI / DeepSeek / ...)
```

## 功能

- **智能路由** — 包月优先、按量兜底。支持 Copilot Coding Plan 自动回落。
- **协议转换** — Anthropic Messages ↔ OpenAI Chat ↔ OpenAI Responses。完整 6 方向转换（基于 [llm-bridge-core]）。
- **Token 压缩** — 基于 [tokenless-schema] 的 schema 与响应透明压缩。
- **成本追踪** — 按项目、按模型的费用记录，含压缩节省量。本地 SQLite 存储。
- **厂商注册** — 内置 Anthropic、OpenAI、Google、DeepSeek 四大厂商。社区仓库支持更多厂商。

## 快速开始

```bash
# 安装
cargo install agent-proxy

# 启动（自动创建 4 个默认渠道，需手动填写 API Key）
agent-proxy serve

# 设置渠道的 API Key
agent-proxy channel set-key anthropic-official --api-key "sk-ant-xxx"

# 添加自定义渠道
agent-proxy channel add openrouter \
  --url "https://openrouter.ai/api/v1" \
  --protocol anthropic_messages \
  --api-key "sk-or-xxx"
```

将 AI 编码工具配置为使用 `http://127.0.0.1:8787` 作为 API 端点。

## 架构

| Crate | 职责 |
|-------|------|
| `core` | 中间件 trait、axum 服务器、上游转发 |
| `model-router` | 渠道选择、模型名映射 |
| `compress` | 基于 tokenless-schema 的 Token 压缩 |
| `bridge` | 基于 llm-bridge-core 的协议转换 |
| `cost` | 按项目成本追踪（SQLite） |

请求流：`compress → route → bridge → 转发 → bridge ← route ← compress → cost`

详见 [specs/](specs/)。

## 分阶段路线

| 阶段 | 范围 |
|------|------|
| **Phase 1** | 本地桌面 MVP — 单用户、简单渠道选择、SQLite 成本追踪 |
| **Phase 2** | 云端就绪 — 健康探针、Docker、配置层级、多实例 |
| **扩展** | 限流、积分/按字符计费 — 独立 crate |

## 相关项目

- [tokenless-schema](https://github.com/TokenFleet-AI/tokenless) — JSON schema 与响应压缩
- [llm-bridge-core](https://github.com/TokenFleet-AI/llm-bridge-rust/tree/master/crates/core) — Anthropic ↔ OpenAI 协议转换

## 许可证

[Apache-2.0](LICENSE)

Copyright 2025 TokenFleet-AI
