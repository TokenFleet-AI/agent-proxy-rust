# 渠道设计重构方案

> **状态**: 设计文档，待实施
> **日期**: 2026-06-04
> **关联**: token-fleet-switch `docs/migration-http-only.md`

---

## 概述

重构渠道（Channel）数据模型和路由逻辑，实现三个目标：

1. **删除渠道级 `base_url`**，改为每个协议条目自带 `base_url`
2. **`path` → `rewrite_path`**：路径重写语义，填写则覆盖请求 path，不填则透传
3. **新增 `force_protocol`**：强制协议转换开关（测试桥接用）

---

## 当前设计回顾

### Channel 结构体 (`storage/src/types.rs`)

```rust
pub struct Channel {
    pub id: String,
    pub name: String,
    pub base_url: String,          // ← 渠道级别，要删除
    pub api_key: SecretString,
    pub protocol: String,          // 默认协议
    pub protocols: String,         // JSON: [{"protocol":"...","path":"..."}]
    pub is_builtin: bool,
    pub enabled: bool,
    // ...健康检查、计费等字段
}
```

### 当前 URL 解析流程

```
Channel.base_url → "https://api.deepseek.com"
protocols JSON   → [{"protocol":"openai_chat","path":"/v1/chat/completions"}]

resolve_upstream_url():
  base_url + path_prefix = "https://api.deepseek.com/v1/chat/completions"

forward_to_upstream():
  upstream_url + proxy_req.path = "https://api.deepseek.com/v1/chat/completions/v1/chat/completions"
  ↑ 这里 path 重复了！因为 bridge 已经改了 proxy_req.path
```

### 问题

- `base_url` 在渠道级别，一个渠道只能有一个上游地址
- `path` 语义模糊——是前缀还是重写？当前实现会导致路径重复
- 无法支持同一渠道不同协议走不同上游
- 测试协议转换需要手动构造场景

---

## 目标设计

### Channel 结构体（新）

```rust
pub struct Channel {
    pub id: String,
    pub name: String,
    // ❌ 删除: pub base_url: String,
    pub api_key: SecretString,
    pub protocol: String,                    // 默认/主上游协议
    pub protocols: String,                   // JSON 格式见下
    pub force_protocol: Option<String>,      // ➕ 新增：强制协议转换
    pub is_builtin: bool,
    pub enabled: bool,
    // ...其余不变
}
```

### `protocols` JSON 新格式

```json
[
  {
    "protocol": "anthropic_messages",
    "base_url": "https://api.anthropic.com",
    "rewrite_path": "/anthropic/v1/messages"
  },
  {
    "protocol": "openai_chat",
    "base_url": "https://api.deepseek.com",
    "rewrite_path": "/chat/completions"
  },
  {
    "protocol": "openai_responses",
    "base_url": "https://api.deepseek.com"
  }
]
```

字段说明：
- `protocol`: 上游协议标识（`anthropic_messages` / `openai_chat` / `openai_responses`）
- `base_url`: 该协议的上游地址
- `rewrite_path`: **可选**。填写则**覆盖**客户端请求 path；不填则**透传**原始 path

### `rewrite_path` 语义

```
rewrite_path = "/chat/completions"   → final URL = "https://api.deepseek.com/chat/completions"
rewrite_path = "" 或不存在            → final URL = "https://api.deepseek.com/v1/chat/completions"
                                        (透传 bridge 改写后的 proxy_req.path)
```

### `force_protocol` 语义

```
force_protocol = None         → 正常逻辑：target_protocol = channel.protocol
force_protocol = "openai_chat" → 强制：target_protocol = "openai_chat"
                                 所有请求都转换为 OpenAI Chat 格式
                                 用途：测试协议桥接功能
```

---

## URL 解析新流程

```
客户端 POST /v1/messages (Anthropic body)

model-router.on_request():
  │
  ├─ 确定 target_protocol:
  │    force_protocol == Some(p) → p
  │    force_protocol == None    → channel.protocol
  │
  ├─ 从 protocols JSON 查找 target_protocol 对应条目:
  │    entry = protocols.find(p => p.protocol == target_protocol)
  │    base_url = entry.base_url
  │    rewrite_path = entry.rewrite_path
  │
  ├─ 设置 ctx.target_protocol = target_protocol
  ├─ 设置 ChannelConfig { url: base_url, rewrite_path, protocol, api_key }
  │
bridge.on_request():
  │  ConversionDirection::resolve(detected_format, target_protocol)
  │  相同 → Passthrough
  │  不同 → 协议转换，修改 proxy_req.path
  │
forward_to_upstream():
  │  path = rewrite_path ?? proxy_req.path
  │  url  = "{base_url}{path}"
  │  例: "https://api.deepseek.com" + "/chat/completions"
  │      "https://api.deepseek.com" + "/v1/chat/completions"  (透传)
```

### 具体场景

#### 场景 1: force_protocol 未设置，同协议直通

```
force_protocol = None
channel.protocol = "openai_chat"
detected_format = OpenaiChat

→ target = OpenaiChat
→ bridge: Passthrough（同协议）
→ entry.rewrite_path = "/chat/completions"
→ 最终: POST https://api.deepseek.com/chat/completions
```

#### 场景 2: force_protocol 未设置，需要桥接

```
force_protocol = None
channel.protocol = "openai_chat"
detected_format = AnthropicMessages

→ target = OpenaiChat
→ bridge: AnthropicMessages → OpenaiChat（协议转换）
→ proxy_req.path 被 bridge 改为 /v1/chat/completions
→ entry.rewrite_path = None（不填）
→ 最终: POST https://api.deepseek.com/v1/chat/completions（透传 bridge 结果）
```

#### 场景 3: force_protocol 强制桥接（测试）

```
force_protocol = "openai_chat"
channel.protocol = "anthropic_messages"
detected_format = AnthropicMessages

→ target = OpenaiChat（force_protocol 覆盖）
→ bridge: AnthropicMessages → OpenaiChat（强制转换）
→ entry.rewrite_path = "/chat/completions"
→ 最终: POST https://api.deepseek.com/chat/completions
```

---

## 代码变更清单

### `crates/storage/src/types.rs`

```diff
pub struct Channel {
    pub id: String,
    pub name: String,
-   pub base_url: String,
    pub api_key: SecretString,
    pub protocol: String,
    pub protocols: String,
+   pub force_protocol: Option<String>,
    // ...
}

+ // 新增强类型
+ #[derive(Debug, Clone, Serialize, Deserialize)]
+ pub struct ProtocolEntry {
+     pub protocol: String,
+     pub base_url: String,
+     #[serde(default, skip_serializing_if = "Option::is_none")]
+     pub rewrite_path: Option<String>,
+ }
```

### `crates/core/src/types.rs`

```diff
pub struct ChannelConfig {
    pub url: String,
    pub api_key: SecretString,
    pub protocol: ApiFormat,
    pub name: String,
+   pub rewrite_path: Option<String>,
}
```

### `crates/model-router/src/lib.rs`

| 变更 | 说明 |
|------|------|
| `ResolvedChannel` 删 `url` 字段 | 不再从 `ch.base_url` 复制 |
| `ResolvedChannel` 加 `protocols: Vec<ProtocolEntry>` | 替代 `protocols_json: String` |
| `resolve_upstream_url()` 重写 | 签名: `(protocol, &[ProtocolEntry]) → Result<(String, Option<String>), Error>` 返回 `(base_url, rewrite_path)` |
| `on_request()` 支持 `force_protocol` | `target = force_protocol.unwrap_or(channel.protocol)` |
| `ChannelConfig` 构造填入 `rewrite_path` | |

### `crates/core/src/server.rs`

```diff
// forward_to_upstream 中:
- let url = format!("{}{}", upstream_url.trim_end_matches('/'), proxy_req.path);
+ let path = channel.rewrite_path.as_deref().unwrap_or(&proxy_req.path);
+ let url = format!("{}{}", channel.url.trim_end_matches('/'), path);
```

### `crates/storage-sqlite/src/lib.rs`

| 变更 |
|------|
| `row_to_channel()`: 不再读 `base_url`（col 2），加读 `force_protocol` |
| `upsert_channel()`: 不再写 `url` 列 |
| `CHANNEL_COLS`: 删 `url`，加 `force_protocol` |
| `migrate()`: 加 v7 迁移 |

### `crates/storage-sqlite/migrations/007_protocol_base_url.sql`（新建）

```sql
-- 将 channel 级 url 注入到每个 protocols 条目
UPDATE channels SET protocols = (
    SELECT json_group_array(
        json_object(
            'protocol', json_extract(je.value, '$.protocol'),
            'base_url', channels.url,
            'rewrite_path', json_extract(je.value, '$.path')
        )
    )
    FROM json_each(channels.protocols) AS je
)
WHERE protocols != '[]' AND protocols IS NOT NULL;

-- 处理 protocols 为空的渠道
UPDATE channels SET protocols = json_array(
    json_object('protocol', protocol, 'base_url', url, 'rewrite_path', '')
)
WHERE (protocols = '[]' OR protocols IS NULL)
  AND protocol IS NOT NULL AND protocol != '';

-- 加 force_protocol 列
ALTER TABLE channels ADD COLUMN force_protocol TEXT;

-- 删 url 列
ALTER TABLE channels DROP COLUMN url;
```

### `crates/storage-sqlite/migrations/001_init.sql`

种子数据中 `url` 改为空字符串，`protocols` JSON 按新格式。

### `apps/server/src/admin.rs`

```diff
struct UpdateChannelBody {
    name: Option<String>,
    enabled: Option<bool>,
    priority: Option<u32>,
    monthly_quota: Option<u64>,
    quota_policy: Option<String>,
+   protocols: Option<String>,
+   force_protocol: Option<String>,
}
```

### `apps/cli/`（删除）

整个目录删除，无用的 CLI 骨架。

---

## 测试更新

| 文件 | 变更 |
|------|------|
| `model-router/lib.rs` tests | `make_channel()` 加 `protocols_json` 参数 |
| `model-router/lib.rs` tests | 新增 `resolve_upstream_url` 直接测试 |
| `storage-sqlite/tests/storage_contract.rs` | `base_url` 断言改为检查 protocols JSON |

---

## 数据迁移

### v7 迁移策略

1. **向上迁移**：现有 `url` 列值注入 `protocols` JSON 每个条目 → 删 `url` 列 → 加 `force_protocol` 列
2. **过渡兼容**：model-router 解析时若条目缺少 `base_url`，fallback 到 channel 级（过渡期）
3. **回滚**：从第一个 protocols 条目的 `base_url` 恢复 `url` 列

### 种子数据示例

```sql
-- DeepSeek 渠道
INSERT INTO channels (id, name, url, api_key, protocol, protocols, ...) VALUES
('deepseek', 'DeepSeek', '', 'sk-deepseek',
 'openai_chat',
 '[{"protocol":"openai_chat","base_url":"https://api.deepseek.com","rewrite_path":"/chat/completions"},
   {"protocol":"anthropic_messages","base_url":"https://api.deepseek.com","rewrite_path":"/anthropic/v1/messages"}]',
 ...);
```

---

## 不做的

- ❌ `api_key` 不移入 protocols JSON（保持渠道级别，后续迭代考虑）
- ❌ 不修改 bridge 协议转换逻辑（`ctx.target_protocol` 仍然由 model-router 设置）
- ❌ 不修改 `forward_to_upstream` 的核心转发逻辑（只改 path 选择）

---

## 影响范围

| 组件 | 影响 |
|------|------|
| `resolve_upstream_url` | 🔴 高 - 签名和实现全改 |
| `ResolvedChannel` | 🟡 中 - 删 `url` 字段 |
| `Channel` struct | 🟡 中 - 删 `base_url`，加 `force_protocol` |
| `ChannelConfig` | 🟡 中 - 加 `rewrite_path` |
| `forward_to_upstream` | 🟢 低 - 只改 path 选择逻辑 |
| Bridge | 🟢 无影响 |
| Cost | 🟢 无影响 |
| Admin API GET response | 🔴 高 - 不再返回顶层 `baseUrl`（token-fleet-switch 需同步改） |

---

## 实施步骤

1. 新增 `ProtocolEntry` 类型 + 改 `Channel` 结构体
2. 新增 v7 迁移 SQL
3. 更新 `SqliteStorage`（row_to_channel / upsert_channel / migrate）
4. 重写 `ResolvedChannel` + `resolve_upstream_url`
5. `on_request` 加 `force_protocol` 逻辑
6. `forward_to_upstream` 支持 `rewrite_path`
7. 更新 Admin API `UpdateChannelBody`
8. 更新种子数据 + 测试
9. 删除 `apps/cli/`
10. `make test` + `make clippy` + `make lint`
