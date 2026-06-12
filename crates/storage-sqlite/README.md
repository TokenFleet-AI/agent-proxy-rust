# agent-proxy-rust-storage-sqlite

SQLite backend implementation for agent-proxy-rust storage trait.

## 功能

- 实现 `Storage` trait — 使用 `rusqlite` + `r2d2` 连接池
- 实现 `SeedManager` trait — 从内嵌 JSON 初始化种子数据，支持远程刷新
- WAL 模式：每个连接启用 WAL 以支持并发读写
- Schema 迁移：`migrate()` 按 `user_version` 增量执行 SQL 迁移文件
  - V1: 基础表（providers、models、channels、model_mappings、cost_records、switch_logs、subscription_fees）
  - V2: model_aliases 表
- 文件存储（`SqliteStorage::new`）和内存存储（`SqliteStorage::new_in_memory`）两种模式
- 连接池最大 4 个连接（`max_connections()` 返回 4）
- 种子数据：内嵌编译时 JSON，支持远程 URL 热更新

## 关键类型

- `SqliteStorage` — SQLite 存储实现，包装 `r2d2::Pool<SqliteConnectionManager>`

## 配置

```rust
use std::sync::Arc;
use agent_proxy_rust_storage::{Storage, SeedManager};
use agent_proxy_rust_storage_sqlite::SqliteStorage;

// 文件数据库
let storage = SqliteStorage::new(std::path::Path::new("data/proxy.db"))?;

// 或内存数据库（测试用）
let storage = SqliteStorage::new_in_memory()?;

// 启动时运行迁移和种子初始化
storage.migrate().await?;
storage.seed_init().await?;

// 可选：从远程 URL 刷新种子数据
// storage.seed_refresh(Some("https://example.com/seed/")).await?;
```

## 数据表

| 表名 | 说明 |
|---|---|
| `providers` | 上游 AI 提供商 |
| `models` | 模型定义（外键关联 providers） |
| `channels` | 上游通道（API key、协议、健康状态、计费） |
| `model_mappings` | 模型映射（client_name → upstream_name + billing） |
| `model_aliases` | 模型别名映射 |
| `cost_records` | 费用记录（token 用量、费用、压缩节省） |
| `switch_logs` | 通道切换日志 |
| `subscription_fees` | 月度订阅费用 |

## 依赖

本 crate 依赖：
- `agent-proxy-rust-storage` — `Storage` trait、`SeedManager` trait、数据类型
- `rusqlite`（bundled）— SQLite 引擎
- `r2d2` / `r2d2_sqlite` — 连接池
- `sha2` / `hex` — 种子数据完整性校验
- `reqwest`（blocking）— 远程种子数据拉取
- `tokio` — `spawn_blocking` 包装同步数据库操作

## 相关文档

- [存储抽象设计](../../specs/0014-storage-abstraction.md)
- [远程种子数据设计](../../specs/0019-remote-seed-data.md)

---

Part of the [agent-proxy-rust](https://github.com/TokenFleet-AI/agent-proxy-rust) workspace.
