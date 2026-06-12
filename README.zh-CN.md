# agent-proxy-rust

AI 编码代理（Claude Code、Codex、Gemini CLI）与上游 API 提供商之间的可组合中间件代理。

**Phase 1 ✅** | 151+ tests | 9 个预置通道 | 36 个模型 | 57 条映射

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
│  • 选择最优通道                        │
│  • 协议转换                           │
│  • 按项目追踪成本                      │
└──────────────────────────────────────┘
        │
        ▼
上游 API (Anthropic / OpenAI / DeepSeek / ...)
```

## 为什么需要

AI 编码代理（Claude Code、Codex、Gemini CLI）硬编码了 API 端点。当你需要：
- 通过包月计划（Copilot、TokenFleet）路由而非按量付费
- 切换上游厂商而不重新配置所有 Agent
- 跨多个 LLM 服务追踪按项目的 token 成本
- 在不兼容的 API 协议间转换（Anthropic ↔ OpenAI）

你需要一个本地代理透明处理这些问题。agent-proxy-rust 坐在你的 Agent 和上游 API 之间，压缩上下文、选择最优通道、转换协议、追踪成本——对 Agent 侧零改动。

## 功能

- **智能路由** — 包月优先、按量兜底。支持 Copilot Coding Plan 自动回落。
- **协议转换** — Anthropic Messages ↔ OpenAI Chat ↔ OpenAI Responses。完整 6 方向转换（基于 [llm-bridge-core]）。
- **Token 压缩** — 基于 [tokenless-schema] 的 schema 与响应透明压缩。
- **成本追踪** — 按项目、按模型的费用记录，含压缩节省量。本地 SQLite 存储。
- **厂商注册** — 内置 Anthropic、OpenAI、Google、DeepSeek 四大厂商。社区仓库支持更多厂商。

## 快速开始

```bash
# 前置条件：Rust 工具链 (rustup)、至少一个 LLM provider API Key

# 1. 安装
cargo install --path apps/server
# 或从 GitHub Releases 下载预编译二进制

# 2. 生成加密密钥（必填）
export PROXY_SECRET=$(openssl rand -hex 32)

# 3. 启动代理服务
agent-proxy serve &

# 4. 设置上游通道的 API Key
agent-proxy channel set-key deepseek --api-key "sk-xxx"

# 5. 验证启动
curl http://127.0.0.1:8787/health

# 6. 将 AI 编码工具指向代理
export ANTHROPIC_BASE_URL="http://127.0.0.1:8787"
```

📖 完整配置指南见 [用户手册](docs/user-guide.md)

## 架构

| Crate | 职责 |
|-------|------|
| `core` | 中间件 trait、axum 服务器、上游转发、认证 |
| `model-router` | 通道选择、模型名映射、故障转移 |
| `bridge` | 协议转换（Anthropic ↔ OpenAI）基于 llm-bridge-core |
| `compress` | 基于 tokenless-schema 的 Token 压缩 |
| `cost` | 按项目成本追踪（SQLite） |
| `storage` | 后端无关的存储 trait |
| `storage-sqlite` | SQLite 实现，含种子数据 |
| `resilience` | 限流、重试、熔断 |
| `server` | 主二进制、CLI、Admin API |

请求流：`compress → route → bridge → 转发 → bridge ← route ← compress → cost`

详见 [架构总览](docs/architecture.md) 和 [specs/](specs/)。

## 开发

```bash
make build       # 编译
make test        # 运行测试
make lint        # fmt + clippy 检查
make clippy      # 仅 clippy
make release     # Tag + CHANGELOG + push（触发 GitHub CD）
```

详见 [贡献者指南](CONTRIBUTING.md) 和 [发布指南](docs/release-guide.md)。

## 分阶段路线

| 阶段 | 范围 | 状态 |
|------|------|------|
| **Phase 1** | 本地桌面 MVP — 单用户、通道选择、SQLite 成本追踪、协议桥接 | ✅ 已完成 |
| **Phase 2** | 云端就绪 — 健康探针、Docker、配置层级、多实例 | 计划中 |
| **扩展** | 限流、积分/按字符计费 — 独立 crate | 计划中 |

## 相关项目

- [tokenless-schema](https://github.com/TokenFleet-AI/tokenless) — JSON schema 与响应压缩
- [llm-bridge-core](https://github.com/TokenFleet-AI/llm-bridge-rust/tree/master/crates/core) — Anthropic ↔ OpenAI 协议转换

## 许可证

[Apache-2.0](LICENSE)

Copyright 2025 TokenFleet-AI
