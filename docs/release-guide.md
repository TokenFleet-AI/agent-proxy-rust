# 发布指南

## 两步发布流程

为避免 CI 失败时误发到 crates.io，发布分为两步：

### Step 1: 推送代码 + 创建 tag

```bash
# 先更新版本号
make bump-version NEW_VERSION=1.2.0

# 再执行发布（VERSION 自动从 Cargo.toml 读取）
make release
```

此步骤会：
- 生成 CHANGELOG.md
- 创建 git tag
- 推送到 GitHub（自动触发 CI）

### Step 2: 等待 CI 通过后发布

```bash
# 查看 CI 状态
gh run list --limit 1

# 看到 success 后执行（发布所有 crate 到 crates.io）
make release-publish
```

## 注意事项

- **不要跳过 CI 检查**：crates.io 发布后无法撤回
- **多 crate 发布**：`release-publish` 按依赖顺序发布 8 个 crate
- **GitHub Release 自动创建**：push tag 后 GitHub Actions 会自动创建 Release 页面
