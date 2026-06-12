# 发布指南

本文档描述 agent-proxy-rust 的版本发布流程、工具链配置与操作规范。

## 概述

本项目采用**单版本 workspace 发布模型**：所有 crate 共享同一版本号，发布时生成单一的 `v{version}` tag，触发 GitHub Actions 构建 4 平台二进制并创建 GitHub Release。

**不发布到 crates.io**，仅通过 GitHub Release 分发预编译二进制。

## 工具链

| 工具 | 版本 | 用途 |
|---|---|---|
| `cargo-release` | ≥ 0.25 | 版本管理、tag 创建 |
| `git-cliff` | ≥ 2.13 | 自动生成 CHANGELOG |
| `gh` CLI | latest | GitHub Release / Actions 查询 |

安装：

```bash
cargo install cargo-release git-cliff
```

## 关键配置文件

### `release.toml`（workspace 根目录）

```toml
# 整个 workspace 共享同一版本
shared-version = true

# 单一 tag 格式，匹配 CD workflow 的 v* 模式
tag-name = "v{{version}}"
tag-message = "Release v{{version}}"

# 不发布到 crates.io
publish = false

# 只允许从 master 分支发布
allow-branch = ["master"]

# 版本 bump 时的 commit message
pre-release-commit-message = "chore: release v{{version}}"
```

### `.github/workflows/cd.yml`

监听 `v*` tag push，触发 `.github/workflows/release.yml` 构建并发布。

### `.github/workflows/release.yml`

构建 4 个目标平台：

| 平台 | Target | 归档格式 |
|---|---|---|
| macOS ARM | `aarch64-apple-darwin` | `.tar.gz` |
| macOS Intel | `x86_64-apple-darwin` | `.tar.gz` |
| Linux musl | `x86_64-unknown-linux-musl` | `.tar.gz` |
| Windows | `x86_64-pc-windows-msvc` | `.zip` |

## 发布流程

### 前置检查

```bash
# 1. 确保在 master 分支且已同步远程
git checkout master
git pull origin master

# 2. 运行完整验证
make lint          # fmt + clippy
make test          # cargo nextest
cargo audit        # 依赖安全检查

# 3. 确保工作树干净
git status
```

### 执行发布

#### 方式 A：`make release`（推荐，手动控制版本）

1. **手动修改 `Cargo.toml` 中的版本号**：

```toml
[workspace.package]
version = "1.0.1"   # ← 修改这里
```

2. **执行发布**：

```bash
make release
```

`make release` 的完整步骤：

```makefile
VERSION := $(shell grep -m1 '^version' Cargo.toml | cut -d'"' -f2)

release:
    @git cliff --tag v$(VERSION) -o CHANGELOG.md   # 1. 生成带版本号的 CHANGELOG
    @git commit -a -n -m "docs: update CHANGELOG for v$(VERSION)" || true
    @cargo release tag --execute --no-confirm      # 2. 创建 v{version} tag
    @git push origin master                        # 3. 推送 master
    @git push origin v$(VERSION)                   # 4. 推送 tag，触发 CD
```

3. **验证**：

```bash
# 查看 Actions 状态
gh run list --limit 3

# 监控 CD workflow
gh run watch <run-id>

# 查看 Release
gh release view v1.0.1
```

#### 方式 B：`cargo release patch --execute`（全自动 bump）

```bash
# patch: 1.0.0 → 1.0.1
cargo release patch --execute --no-confirm

# 或 minor / major
cargo release minor --execute --no-confirm
cargo release major --execute --no-confirm
```

自动完成：bump 版本 → 更新 crate 间依赖 → commit → tag → push。

> ⚠️ 注意：此方式下 CHANGELOG 不会自动更新。如需 CHANGELOG，发布后手动执行 `git cliff --tag vX.Y.Z -o CHANGELOG.md && git commit -a -m "docs: CHANGELOG"`。

### 版本级别选择

| 级别 | 示例 | 适用场景 |
|---|---|---|
| `patch` | 1.0.0 → 1.0.1 | Bug 修复、安全补丁 |
| `minor` | 1.0.0 → 1.1.0 | 新增功能（向后兼容） |
| `major` | 1.0.0 → 2.0.0 | 破坏性变更 |

遵循 [Semantic Versioning](https://semver.org/)。

## `cargo-release` 命令参考

### 两种模式

| 用法 | 说明 |
|---|---|
| `cargo release <LEVEL>` | 完整发布：bump + commit + tag + push |
| `cargo release <STEP>` | 只跑单个步骤（tag / push / publish 等） |

### 常用标志

| 标志 | 作用 |
|---|---|
| `--execute` / `-x` | 真执行（默认 dry-run） |
| `--no-confirm` / `-n` | 跳过交互确认 |
| `--workspace` | 对整个 workspace 操作 |
| `--config <PATH>` | 指定配置文件 |

### 单步骤命令

```bash
cargo release tag --execute --no-confirm     # 只创建 tag
cargo release push --execute --no-confirm    # 只推送
cargo release publish --execute              # 只发布到 crates.io（本项目禁用）
```

### 演练模式（不加 `--execute`）

```bash
cargo release patch --workspace              # 查看将发生什么，不实际执行
```

## 删除 Release

如需删除已发布的版本（如发布错误）：

```bash
# 删除 GitHub Release + 远程 tag（一步完成）
gh release delete v1.0.0 --cleanup-tag --yes

# 删除本地 tag（如仍存在）
git tag -d v1.0.0

# 验证
gh release list --limit 5
git tag -l "v1.0.0"
git ls-remote --tags origin | grep v1.0.0
```

## 常见问题

### Q：为什么 `cargo release tag` 报 "tag doesn't exist"？

**原因**：`cargo release tag` 默认在非 TTY 环境下等待交互确认，默认拒绝，导致没创建 tag。

**解决**：加 `--no-confirm` 标志。

### Q：为什么 CD workflow 没触发？

**原因**：tag 格式不匹配。CD 监听 `v*`（如 `v1.0.0`），但默认 `cargo-release` 在 workspace 下创建 `{crate}-v{version}` 格式。

**解决**：在 `release.toml` 中设置 `tag-name = "v{{version}}"`（已配置）。

### Q：CHANGELOG 显示 `[unreleased]` 而不是版本号？

**原因**：`git cliff` 在没有 tag 时不知道用什么版本号。

**解决**：用 `--tag` 显式指定：`git cliff --tag v1.0.0 -o CHANGELOG.md`。

### Q：可以回滚发布吗？

GitHub Release 可删除（见上方"删除 Release"章节），但 git tag 一旦传播难以收回。建议：
- 发现错误立即删除 GitHub Release
- 不要重新使用同一版本号（应发布修正版本，如 1.0.1 → 1.0.2）

## 发布清单

发布前对照：

- [ ] 在 `master` 分支
- [ ] 工作树干净（`git status`）
- [ ] `make lint` 通过
- [ ] `make test` 通过
- [ ] `cargo audit` 无高危漏洞
- [ ] 所有待发布变更已合并到 master
- [ ] 已更新 `Cargo.toml` 版本号（方式 A）或决定 bump 级别（方式 B）

---

Owner: baoyx · 版本：v1.0 · 生效日期：2026-06-12 · 最后更新：2026-06-12
