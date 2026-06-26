# Documentation Index

This directory contains reusable project documentation for the template repository.

> 📌 **语言策略**：本项目文档以中文为主。工具类文档（ruflo-usage、pre-commit-usage）保留中英双版本。所有文档均可在 GitHub 上直接阅读。

## 🚀 新用户入门

面向第一次接触本项目的人类用户，按角色推荐阅读顺序：

### 使用者（部署与使用代理）
1. [README](../README.md) — 项目概览与快速启动
2. [核心概念](./concepts.md) — 通道、模型、映射、协议桥接等核心概念
3. [用户手册](./user-guide.md) — 完整配置指南
4. [部署运维指南](./deployment.md) — 安装、systemd/launchd 服务、健康检查、备份与故障排查
5. [数据流](./data-flow.md) — 请求链路全解析
6. [发布指南](./release-guide.md) — 版本发布流程

### 贡献者（参与开发）
1. [README](../README.md) — 项目概览
2. [贡献者指南](../CONTRIBUTING.md) — 开发流程与规范
3. [架构总览](./architecture.md) — Workspace 结构、Crate 依赖、请求生命周期
4. [CLAUDE.md](../CLAUDE.md) — AI Agent 工作规范（也适合人类参考）
5. [架构设计](../specs/0001-architecture.md) — 系统架构

## Agent workflow

- [Ruflo Usage](./ruflo-usage.md) — how this template uses Ruflo for agent workflow and orchestration.
- [CodeGraph Usage](./codegraph-usage.md) — 通用代码图谱/关系分析教程，用于快速理解仓库结构、调用链和影响面。
- [核心概念](./concepts.md) — 通道、模型、映射、协议桥接等核心概念。
- [架构总览](./architecture.md) — Workspace 结构、Crate 依赖、请求生命周期。
- [Data Flow](./data-flow.md) — agent-proxy-rust 完整数据流，覆盖启动、请求链、压缩、路由、桥接、计费、故障转移、安全边界。
- [User Guide](./user-guide.md) — 用户说明书，覆盖安装、配置、通道管理、内置模型、Ruflo 集群模式、计费追踪、故障排查。
- [Admin API 参考](./admin-api.md) — 管理 API 端点完整参考，覆盖 33 个端点的请求/响应格式。
- [Channel Redesign](../specs/channel-redesign.md) — 通道设计重构方案：删除 base_url、rewrite_path 语义、force_protocol 强制协议转换。

## Development workflow

- [Pre-commit Usage](./pre-commit-usage.md) — how to install and run repository pre-commit hooks.
- [发布指南](./release-guide.md) — 版本发布流程、工具链配置、`make release` / `cargo-release` 操作规范、CD 自动构建、常见问题。

## 设计文档（Specs）

完整的设计规范位于 [`specs/`](../specs/) 目录，关键文档：

- [0001-architecture](../specs/0001-architecture.md) — 系统架构
- [0002-middleware-engine](../specs/0002-middleware-engine.md) — 中间件引擎
- [0004-cost-tracking](../specs/0004-cost-tracking.md) — 计费追踪
- [0008-security](../specs/0008-security.md) — 安全模型
- [0009-deployment](../specs/0009-deployment.md) — 部署架构
- [0014-storage-abstraction](../specs/0014-storage-abstraction.md) — 存储抽象
- [0016-admin-api-extension](../specs/0016-admin-api-extension.md) — Admin API

完整索引见 [`specs/index.md`](../specs/index.md)。

## SPARC 文档中心

小任务找专家，大任务找协调器；`TDD` 是规则，不是入口；高风险任务不得单 Agent 一把梭。

- [SPARC 使用规范](./sparc-usage-guideline.md) — 内部使用规范，用于统一单 Agent 与多智能体工作流的入口选择。
- [提示词模板库](./prompt-template-library.md) — 常用 SPARC 任务提示词模板，可直接复制使用。
- [TDD 规范](./tdd-guideline.md) — TDD 工作流规则、推荐顺序与阶段门禁。
- [高风险任务处理规范](./high-risk-task-guideline.md) — 高风险改动的协作、测试与审查要求。
- [文档搜索索引](./search.md) — GitHub 可渲染的轻量检索入口。

## 研究分析

[`research/`](./research/) 目录下的深度调研文档：

- [Billing 改进分析](./research/billing-improvement-analysis.md) — 计费字段完整性改进方案
- [Cost Context 完整性](./research/cost-context-completeness.md) — 计费上下文完整性分析
- [SSE Usage 分析](./research/sse-usage-analysis.md) — SSE 流式使用统计方案
- [Upstream Channel 追踪](./research/upstream-channel-tracking.md) — 上游通道追踪机制

## AI Agent 工作流阅读顺序

> 以下推荐顺序面向使用 SPARC / Ruflo 工作流的 AI Agent。人类用户请参考上方"新用户入门"。

1. 先读 [SPARC 使用规范](./sparc-usage-guideline.md)，建立整体判断框架。
2. 再看 [提示词模板库](./prompt-template-library.md)，拿到可直接复制的任务模板。
3. 涉及测试先行时，补充阅读 [TDD 规范](./tdd-guideline.md)。
4. 涉及重构、安全、兼容性等高风险改动时，补充阅读 [高风险任务处理规范](./high-risk-task-guideline.md)。
5. 需要快速定位主题时，使用 [文档搜索索引](./search.md)。

Owner: baoyx · 版本：v1.1 · 生效日期：2026-05-21 · 最后更新：2026-06-12
6. 发布流程参考 [发布指南](./release-guide.md)。
