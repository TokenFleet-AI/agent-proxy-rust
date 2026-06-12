# 文档搜索索引

GitHub 仓库浏览器不会执行原 HTML 搜索页中的前端 JavaScript。为了让内容在 GitHub 上直接可读，本页改为 Markdown 检索索引。

## 全部文档

| 文档 | 摘要 | 关键词 |
|---|---|---|
| [index](./index.md) | 文档总索引，按角色和主题分类导航全部 docs | index, navigation, 入门, 目录, 阅读顺序 |
| [user-guide](./user-guide.md) | 用户说明书，覆盖安装、配置、通道管理、内置模型、Ruflo 集群模式、计费追踪、故障排查 | user, install, config, channel, model, billing, troubleshooting |
| [data-flow](./data-flow.md) | agent-proxy-rust 完整数据流，覆盖启动、请求链、压缩、路由、桥接、计费、故障转移、安全边界 | data-flow, request, compress, route, bridge, billing, failover, security |
| [release-guide](./release-guide.md) | 版本发布流程、工具链配置与操作规范 | release, cargo-release, make, CD, versioning |
| [channel-redesign](../specs/channel-redesign.md) | 通道设计重构方案：删除 base_url、rewrite_path 语义、force_protocol 强制协议转换 | channel, redesign, base_url, rewrite_path, force_protocol, migration |
| [ruflo-usage](./ruflo-usage.md) | Ruflo agent 工作流编排使用指南，含 15-agent swarm 参考、提示词模板 | ruflo, swarm, orchestrator, agent, workflow, 15-agent |
| [pre-commit-usage](./pre-commit-usage.md) | pre-commit hooks 安装、配置与日常使用指南 | pre-commit, hooks, fmt, clippy, deny, typos, nextest |
| [sparc-usage-guideline](./sparc-usage-guideline.md) | SPARC 单 Agent 与多智能体工作流入口选择规范 | SPARC, entry, expert, orchestrator, swarm-coordinator, TDD, 决策树 |
| [prompt-template-library](./prompt-template-library.md) | 常用 SPARC 任务提示词模板，可直接复制使用 | prompt, template, coder, tester, reviewer, orchestrator, TDD |
| [tdd-guideline](./tdd-guideline.md) | TDD 工作流规则、推荐顺序与阶段门禁 | TDD, tester, coder, reviewer, RED_CONFIRMED, TEST_PLAN_READY |
| [high-risk-task-guideline](./high-risk-task-guideline.md) | 高风险改动的协作、测试与审查要求 | 高风险, refactor, security, compatibility, reviewer, orchestrator |
| [codegraph-usage](./codegraph-usage.md) | 代码图谱/关系分析工具安装、初始化与 Claude Code 接入教程 | codegraph, MCP, 调用链, 依赖分析, 影响面, 代码图谱 |
| [search](./search.md) | 文档搜索索引，GitHub 可渲染的轻量检索入口 | search, index, 检索, 关键词 |
| [concepts](./concepts.md) | 核心概念：通道、模型、映射、协议桥接、压缩、中间件、故障转移 | concepts, channel, model, mapping, bridge, compress, middleware, failover |
| [architecture](./architecture.md) | 架构总览：Workspace 结构、Crate 依赖图、请求生命周期、存储、安全边界 | architecture, workspace, crate, dependency, request-lifecycle, middleware |
| [admin-api](./admin-api.md) | Admin API 参考：33 个管理端点的请求/响应格式 | admin, API, health, channels, providers, models, mappings, cost, seed |
| [deployment](./deployment.md) | 部署运维指南：安装、systemd/launchd 服务、健康检查、日志、备份、故障排查 | deployment, systemd, launchd, install, health, log, backup, troubleshooting |

## 研究文档

| 文档 | 摘要 | 关键词 |
|---|---|---|
| [billing-improvement-analysis](./research/billing-improvement-analysis.md) | 计费字段完整性改进方案，覆盖上游通道追踪、模型记录、SSE usage 提取 | billing, cost, improvement, SSE, upstream-channel |
| [cost-context-completeness](./research/cost-context-completeness.md) | CostRecord 25 字段逐字段来源追踪与语义分析 | cost, CostRecord, context, completeness, field-tracking |
| [sse-usage-analysis](./research/sse-usage-analysis.md) | SSE 流式场景下 Usage 提取失效分析与修复方案 | SSE, streaming, usage, token, billing, first-message |
| [upstream-channel-tracking](./research/upstream-channel-tracking.md) | 上游通道追踪机制设计与数据流全景分析 | upstream, channel, tracking, data-flow, routing |

## 推荐检索方式

- 在 GitHub 页面按 `t` 可以快速搜索仓库文件名。
- 在当前 Markdown 页面使用浏览器搜索，输入关键词如 `TDD`、`review`、`swarm`、`高风险`。
- 在本地仓库中使用 `rg "关键词" docs` 检索全部文档内容。

## 导航

- 返回：[Documentation Index](./index.md)

Owner: baoyx · 版本：v1.1 · 生效日期：2026-05-21 · 最后更新：2026-06-12
