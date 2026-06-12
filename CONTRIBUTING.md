# 贡献者指南

感谢你考虑为 agent-proxy-rust 贡献力量！

## 开发环境

### 前置条件
- Rust 工具链（通过 [rustup](https://rustup.rs/) 安装，版本见 `rust-toolchain.toml`）
- Cargo 2024 edition 支持
- make（可选，但推荐）

### 初始化
```bash
git clone https://github.com/TokenFleet-AI/agent-proxy-rust.git
cd agent-proxy-rust
make build
```

## 开发流程

### 分支策略
- `master`：主分支，保持可发布状态
- 功能分支：`feat/xxx`、`fix/xxx`、`docs/xxx`

### 常用命令
```bash
make build       # 编译
make test        # 运行测试
make lint        # fmt + clippy 检查
make clippy      # 仅 clippy
```

### 提交规范
遵循 [Conventional Commits](https://www.conventionalcommits.org/)：
- `feat:` 新功能
- `fix:` 修复
- `docs:` 文档
- `chore:` 工具/依赖
- `refactor:` 重构
- `perf:` 性能

### PR 流程
1. Fork 仓库
2. 创建功能分支
3. 确保 `make lint` 和 `make test` 通过
4. 提交 PR 到 `master`
5. 等待 review

## 代码规范

详细规范见 [CLAUDE.md](./CLAUDE.md)（AI Agent 工作规范，也适用于人类开发者）。核心要点：

- Rust 2024 edition
- `#![forbid(unsafe_code)]`
- 禁止 `unwrap()` / `expect()`
- 所有公共项必须有文档
- `cargo clippy --pedantic` 零警告

## 文档规范

- 文档放在 `docs/` 目录
- 末尾添加元信息：`Owner: xxx · 版本：vX.Y · 生效日期：YYYY-MM-DD · 最后更新：YYYY-MM-DD`
- 中文为主，术语保留英文

## 发布流程

见 [发布指南](./docs/release-guide.md)。

---

Owner: baoyx · 版本：v1.0 · 生效日期：2026-06-12 · 最后更新：2026-06-12
