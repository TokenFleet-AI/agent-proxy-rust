# 远程 Seed Data 更新机制

## 概述

将数据库 seed data（providers、models、channels、model_mappings 及其定价）从硬编码的 SQL migration 中解耦，改为通过 Git Tag 版本化的远程 JSON 文件分发，支持热更新模型定价和 channel 配置，无需重新编译部署。

**核心痛点**：

- 当前 `001_init.sql` 混合了 DDL（270 行）和 seed data（大量 INSERT）
- 模型定价、新模型、channel 配置变更需要修改源码 → 编译 → 部署
- 无版本追踪，无法感知 seed data 是否过期
- 无离线回退，首次启动完全依赖编译时嵌入的数据

---

## 设计目标

| 目标 | 说明 |
|------|------|
| **DDL/Seed 分离** | migration 只负责建表，seed data 独立管理 |
| **远程热更新** | Git Tag 版本化 JSON 文件，HTTP fetch 后 upsert |
| **离线可用** | 嵌入式 fallback JSON，断网时首次启动也能正常工作 |
| **完整性校验** | SHA-256 checksum 验证每个文件 |
| **版本追踪** | 本地 `seed_metadata` 表记录版本号、更新时间、hash |
| **可观测性** | Admin API 查看状态 + 手动/自动触发刷新 |

---

## 架构设计

```
┌────────────────────────────────────────────────────────────┐
│                      main.rs 启动流程                       │
│                                                            │
│  migrate()  ──→  seed_init()  ──→  (可选) seed_refresh()   │
│  DDL only       嵌入式回退        远程拉取 + upsert          │
└────────────────────────────────────────────────────────────┘
       │               │                    │
       ▼               ▼                    ▼
┌─────────────┐ ┌─────────────┐  ┌──────────────────────┐
│ 001_init.sql│ │ seed/*.json │  │ GitHub Raw (CDN)      │
│ (DDL only)  │ │ include_str!│  │ refs/tags/seed-v{N}   │
│             │ │ 编译时嵌入   │  │ + SHA-256 checksum    │
└─────────────┘ └─────────────┘  └──────────────────────┘
```

### 数据流

```
seed_init()
  │
  ├─ 检查 seed_metadata 表是否存在
  │   └─ 不存在 → 使用嵌入式 fallback JSON 初始化
  │
  └─ 检查本地 version
      └─ version == 0 → 使用嵌入式 fallback（首次启动）
      └─ version > 0  → 已有数据，跳过（等待 refresh）

seed_refresh(url?)
  │
  ├─ GET {base_url}/seed-manifest.json
  │   └─ 失败 → 返回错误（已有数据不受影响）
  │
  ├─ 比对 local_version vs remote_version
  │   └─ 相同 → 无需更新
  │
  ├─ 逐个下载 JSON 文件，验证 SHA-256
  │   └─ 校验失败 → 中止，报错
  │
  ├─ 反序列化 + 逐条 UPSERT 到 SQLite
  │
  └─ 更新 seed_metadata
```

---

## 文件结构

```
crates/storage-sqlite/
├── migrations/
│   └── 001_init.sql              # DDL only + seed_metadata 表
├── seed/                          # NEW: 嵌入式 fallback
│   ├── seed-manifest.json
│   ├── providers.json
│   ├── models.json
│   ├── channels.json
│   └── model_mappings.json
├── src/
│   ├── lib.rs                    # SqliteStorage impl
│   └── seed.rs                   # NEW: SeedManager impl
```

---

## Git Tag 版本策略

### Tag 命名

```
seed-v1  → 初版
seed-v2  → 更新模型定价
seed-v3  → 新增 provider + models
...
```

### GitHub Raw URL 模板

```
https://raw.githubusercontent.com/TokenFleet-AI/agent-proxy-rust/refs/tags/{TAG}/crates/storage-sqlite/seed/
```

实例：
```
https://raw.githubusercontent.com/TokenFleet-AI/agent-proxy-rust/refs/tags/seed-v3/crates/storage-sqlite/seed/seed-manifest.json
```

### 更新流程

1. 开发者修改 `seed/*.json`
2. 运行 `make seed-manifest` → 重新计算各文件 SHA-256，更新 `seed-manifest.json`
3. 提交 PR → merge master
4. `git tag seed-v{N+1} && git push --tags`
5. 实例通过 Admin API 或定时任务拉取新版本

---

## Seed Manifest 格式

`seed-manifest.json`:

```json
{
  "version": 3,
  "min_schema_version": 1,
  "updated_at": "2026-06-04T00:00:00Z",
  "entries": {
    "providers": {
      "file": "providers.json",
      "sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    },
    "models": {
      "file": "models.json",
      "sha256": "a7ffc6f8bf1ed76651c14756a061d662f580ff4de43b49fa82d80a4b80f8434a"
    },
    "channels": {
      "file": "channels.json",
      "sha256": "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
    },
    "model_mappings": {
      "file": "model_mappings.json",
      "sha256": "6b23c0d5f35d1b11f9b683f0b0a617355deb11277d91ae091d399c655b5f2c1d"
    }
  }
}
```

---

## Seed JSON 文件格式

### providers.json

```json
[
  {
    "id": "anthropic",
    "name": "Anthropic",
    "created_at": 0
  }
]
```

### models.json

```json
[
  {
    "id": "deepseek:deepseek-v4-flash",
    "provider_id": "deepseek",
    "client_name": "deepseek-v4-flash",
    "price_input": 1.0,
    "price_output": 2.0,
    "currency": "CNY",
    "context_window": 1000000,
    "created_at": 0
  }
]
```

### channels.json

```json
[
  {
    "id": "deepseek",
    "name": "DeepSeek Official",
    "api_key": "",
    "protocol": "anthropic_messages",
    "protocols": "[{\"protocol\":\"openai_chat\",\"baseUrl\":\"https://api.deepseek.com\",\"rewritePath\":\"/chat/completions\"}]",
    "is_builtin": true,
    "enabled": true,
    "billing_type": "metered",
    "monthly_quota": null,
    "quota_policy": "fallback",
    "priority": 0
  }
]
```

### model_mappings.json

```json
[
  {
    "id": "deepseek:deepseek-v4-flash",
    "channel_id": "deepseek",
    "client_name": "deepseek-v4-flash",
    "upstream_name": "deepseek-v4-flash",
    "billing": "metered",
    "pricing_json": "{\"type\":\"per_token\",\"currency\":\"CNY\",\"input_per_mtok\":1.0,\"output_per_mtok\":2.0}",
    "weight": 100,
    "enabled": true
  }
]
```

---

## 新增数据库表

```sql
-- seed_metadata: 追踪本地 seed data 状态
CREATE TABLE IF NOT EXISTS seed_metadata (
    key TEXT PRIMARY KEY,           -- "version", "last_refresh", "remote_url", "providers:sha256" 等
    value TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

-- 初始化默认值
INSERT OR IGNORE INTO seed_metadata (key, value, updated_at) VALUES
    ('version', '0', strftime('%s', 'now')),
    ('source', 'embedded', strftime('%s', 'now'));
```

---

## SeedManager Trait

```rust
/// Seed data manager — initializes and refreshes reference data from remote.
#[async_trait]
pub trait SeedManager: Send + Sync {
    /// 使用嵌入式 fallback 初始化 seed data（幂等）。
    async fn seed_init(&self) -> Result<SeedStatus, StorageError>;

    /// 从远程拉取并 upsert seed data。
    /// url 为 None 时使用默认 SEED_REMOTE_URL 或上次记录的 url。
    async fn seed_refresh(&self, url: Option<&str>) -> Result<SeedStatus, StorageError>;

    /// 查询当前 seed 状态。
    async fn seed_status(&self) -> Result<SeedStatus, StorageError>;
}
```

### SeedStatus 类型

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeedStatus {
    pub local_version: u32,
    pub remote_version: Option<u32>,
    pub update_available: bool,
    pub source: String,                  // "embedded" | "remote" | "cache"
    pub entries: Vec<SeedEntryStatus>,
    pub last_refresh_at: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeedEntryStatus {
    pub name: String,
    pub local_sha256: Option<String>,
    pub remote_sha256: Option<String>,
    pub changed: bool,
}
```

---

## 嵌入式回退

编译时通过 `include_str!` 嵌入：

```rust
const SEED_PROVIDERS: &str = include_str!("../seed/providers.json");
const SEED_MODELS: &str = include_str!("../seed/models.json");
const SEED_CHANNELS: &str = include_str!("../seed/channels.json");
const SEED_MODEL_MAPPINGS: &str = include_str!("../seed/model_mappings.json");
```

- 当本地 `seed_metadata.version == 0` 时（首次启动），自动使用嵌入式数据初始化
- 远程拉取失败时不回退嵌入式数据（已有数据不受影响），仅报告错误
- 数据库文件被删除后重建时，再次使用嵌入式数据

---

## 安全设计

| 威胁 | 缓解 |
|------|------|
| 中间人篡改 | HTTPS (GitHub Raw) |
| 文件内容篡改 | SHA-256 checksum 校验，每个文件单独验证 |
| 回滚攻击 | `version` 单调递增，本地记录 `last_version`，拒绝低版本 |
| 恶意 JSON 注入 | serde 强类型反序列化 + 字段级校验 |
| 远程不可用 | 已有数据保持不变，仅报告错误；嵌入式回退仅用于首次初始化 |

---

## Admin API

### `GET /admin/seed/status`

查询当前状态。支持 `?remote=true` 参数同时检查远程最新版本。

返回示例：
```json
{
  "localVersion": 2,
  "remoteVersion": 3,
  "updateAvailable": true,
  "source": "remote",
  "entries": [
    { "name": "providers", "localSha256": "abc...", "remoteSha256": "abc...", "changed": false },
    { "name": "models", "localSha256": "def...", "remoteSha256": "xxx...", "changed": true }
  ],
  "lastRefreshAt": "2026-06-03T12:00:00Z",
  "lastError": null
}
```

### `POST /admin/seed/refresh`

触发远程拉取。支持可选 body：

```json
{
  "url": "https://raw.githubusercontent.com/TokenFleet-AI/agent-proxy-rust/refs/tags/seed-v3/crates/storage-sqlite/seed/"
}
```

返回示例：
```json
{
  "success": true,
  "previousVersion": 2,
  "newVersion": 3,
  "updatedEntries": ["models", "model_mappings"],
  "errors": []
}
```

---

## 配置

| 环境变量 | 默认值 | 说明 |
|----------|--------|------|
| `AGENT_PROXY_SEED_URL` | (空) | 远程 seed data 基础 URL；为空时仅使用嵌入式数据 |
| `AGENT_PROXY_SEED_AUTO_REFRESH` | `false` | 启动时是否自动拉取远程更新 |
| `AGENT_PROXY_SEED_TAG` | (空) | 指定 Git Tag 版本，如 `seed-v3`；优先级高于 manifest 中的默认版本 |

---

## 实施计划

### Phase 1 — 基础分离（本次）

- [ ] 从 `001_init.sql` 提取 seed data → `seed/*.json`
- [ ] `001_init.sql` 精简为 DDL only + 新增 `seed_metadata` 表
- [ ] 新增 `SeedStatus` / `SeedEntryStatus` 类型到 `crates/storage/src/types.rs`
- [ ] 新增 `SeedManager` trait 到 `crates/storage/src/lib.rs`
- [ ] 实现 `seed.rs` — `seed_init()` + 嵌入式回退
- [ ] `main.rs` 启动流程加入 `seed_init()`
- [ ] 单元测试

### Phase 2 — 远程拉取

- [ ] `Cargo.toml` 添加 `reqwest`、`sha2`、`hex` 依赖
- [ ] 实现 `seed_refresh()` — HTTP fetch + SHA-256 校验 + upsert
- [ ] Admin API: `GET /admin/seed/status` + `POST /admin/seed/refresh`
- [ ] 版本回滚防护
- [ ] 集成测试

### Phase 3 — Makefile 自动化

- [ ] `make seed-manifest` — 自动计算 SHA-256 → 更新 manifest
- [ ] `make seed-tag` — 自动 git tag + push

---

## 依赖

```toml
# crates/storage-sqlite/Cargo.toml 新增
reqwest = { version = "0.12", features = ["rustls-tls", "json"], default-features = false }
sha2 = "0.10"
hex = "0.4"
```

`reqwest` 已在 workspace 的 `Cargo.lock` 中存在（被其他 crate 间接依赖），无需额外引入。
