# Admin API 参考

本文档列出 agent-proxy-rust 提供的所有管理 API 端点，基于实际代码生成。

## 基础信息

- 基础 URL：`http://127.0.0.1:11837`
- 认证：`x-admin-key` 请求头（值为启动时生成或 `AGENT_PROXY_ADMIN_KEY` 环境变量）
- 响应格式：JSON
- CORS：允许所有来源（本地开发模式）

## 错误响应格式

所有错误返回统一 JSON 结构：

```json
{
  "error": "error message"
}
```

标准 HTTP 状态码：`400`（请求错误）、`401`（未授权）、`404`（未找到）、`409`（冲突）、`500`（内部错误）。

---

## 端点列表

### 健康检查

#### `GET /health`

代理服务健康检查（无需认证）。

**响应：**
```json
{"status": "ok"}
```

#### `GET /admin/health`

管理面板健康状态（需认证）。

**响应：**
```json
{
  "healthy": true,
  "healthyChannels": 7,
  "totalChannels": 9
}
```

---

### Providers

#### `GET /admin/providers`

列出所有 provider。

**响应：** `Vec<Provider>` — provider 列表。

#### `GET /admin/providers/{id}`

获取单个 provider。

**路径参数：** `id` — provider ID。

**响应：** `Provider` 对象，未找到返回 `404`。

---

### Models

#### `GET /admin/models`

列出所有模型，可按 provider 过滤。

**查询参数：**
| 参数 | 类型 | 说明 |
|------|------|------|
| `provider_id` | string | 按 provider ID 过滤 |

**响应：** `Vec<Model>` — 模型列表。

#### `GET /admin/models/{id}`

获取单个模型。

**路径参数：** `id` — 模型 ID。

**响应：** `Model` 对象，未找到返回 `404`。

---

### Channels

#### `GET /admin/channels`

列出所有通道，可按模型过滤。

**查询参数：**
| 参数 | 类型 | 说明 |
|------|------|------|
| `model_id` | string | 按模型 ID 过滤 |

**响应：** `Vec<Channel>` — 通道列表。

#### `GET /admin/channels/{id}`

获取单个通道。

**路径参数：** `id` — 通道 ID。

**响应：** `Channel` 对象，未找到返回 `404`。

#### `PUT /admin/channels/{id}`

更新通道配置。更新后自动触发 channel 热重载。

**请求体：**
```json
{
  "name": "My Channel",
  "enabled": true,
  "priority": 10,
  "monthlyQuota": 1000000,
  "quotaPolicy": "Block",
  "protocols": "[{\"protocol\":\"openai_chat\",\"base_url\":\"https://api.openai.com\"}]",
  "forceProtocol": "openai_chat"
}
```

所有字段均为可选。`forceProtocol` 必须是 `protocols` 中已存在的协议。

**响应：** 更新后的 `Channel` 对象。

#### `DELETE /admin/channels/{id}`

删除通道。删除后自动触发 channel 热重载。

**响应：**
```json
{"deleted": true}
```

未找到返回 `404`。

#### `POST /admin/channels/{id}/healthy`

标记通道为健康状态。

**响应：** `{"status": "ok"}`

#### `POST /admin/channels/{id}/failure`

记录通道故障。

**响应：** `{"status": "ok"}`

#### `PUT /admin/channels/{id}/api-key`

设置或更新通道 API 密钥。密钥会持久化到数据库并同步到内存，立即生效。

**请求体：**
```json
{"apiKey": "sk-xxx"}
```

空字符串会清除密钥并将通道标记为不健康。

**响应：** `{"status": "ok"}`

#### `GET /admin/channels/{id}/protocols`

获取通道支持的协议列表。

**响应：**
```json
{
  "channelId": "deepseek",
  "channelName": "DeepSeek",
  "protocols": [
    {"protocol": "openai_chat", "baseUrl": "https://api.deepseek.com"}
  ]
}
```

---

### Model Mappings

#### `GET /admin/model-mappings`

列出所有模型映射。

**响应：** `Vec<ModelMapping>` — 映射列表。

#### `POST /admin/model-mappings`

创建或更新模型映射。创建后自动触发 channel 热重载。

**请求体：** `ModelMapping` 对象（包含 `id`、`channelId`、`clientName`、`upstreamName`、`billing`、`pricingJson`、`weight`、`enabled`、`protocols` 等字段）。

**响应：** 创建的 `ModelMapping` 对象。

#### `PUT /admin/model-mappings/{id}`

更新模型映射部分字段。更新后自动触发 channel 热重载。

**请求体：** JSON 对象，可包含以下任意字段：
- `upstreamName` (string)
- `clientName` (string)
- `billing` (string)
- `pricingJson` (string)
- `weight` (number)
- `enabled` (boolean)
- `protocols` (string)

**响应：** `{"status": "ok"}`，未找到返回 `404`。

#### `DELETE /admin/model-mappings/{id}`

删除模型映射。删除后自动触发 channel 热重载。

**响应：** `{"deleted": true}`，未找到返回 `404`。

---

### Model Aliases

#### `GET /admin/model-aliases`

列出所有模型别名。

**响应：** `Vec<ModelAlias>` — 别名列表。

#### `POST /admin/model-aliases`

创建或更新模型别名。

**请求体：** `ModelAliasRequest` 对象。

**响应：** 创建的 `ModelAlias` 对象。

#### `DELETE /admin/model-aliases/{id}`

删除模型别名。

**路径参数：** `id` — 别名 ID（整数）。

**响应：** `{"deleted": true}`，未找到返回 `404`。

---

### Available Channels

#### `GET /admin/available-channels`

列出已启用的通道及其绑定模型，用于 token-fleet-switch 直连模式。

**响应：** `Vec<AvailableChannelInfo>` — 可用通道信息列表。

---

### Cost Records

#### `GET /admin/cost-records`

查询费用记录。

**查询参数：**
| 参数 | 类型 | 说明 |
|------|------|------|
| `project` | string | 按项目路径过滤 |
| `model_name` | string | 按模型名称过滤 |
| `channel_name` | string | 按通道名称过滤 |
| `days` | number | 查询天数 |
| `limit` | number | 返回条数限制 |
| `offset` | number | 分页偏移 |
| `tz_offset` | number | 时区偏移（分钟，东八区为 `480`） |

**响应：** `Vec<CostRecord>` — 费用记录列表。

#### `GET /admin/cost-records/report`

按项目聚合的费用报告。

**查询参数：**
| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `project` | string | — | 按项目过滤 |
| `days` | number | `30` | 查询天数 |
| `tz_offset` | number | — | 时区偏移（分钟） |

**响应：** `Vec<CostAggregate>` — 聚合结果，包含 `groupKey`、`totalCost`、`totalTokens`、`count` 等字段。

#### `GET /admin/cost-records/savings`

压缩节省统计。

**查询参数：** 同 `cost-records/report`（`project`、`days`、`tz_offset`）。

**响应：**
```json
{
  "schemaSavedTokens": 50000,
  "responseSavedTokens": 30000,
  "rtkSavedTokens": 10000,
  "totalSavedTokens": 90000
}
```

#### `GET /admin/cost-records/trend`

按小时或按天的费用趋势。

**查询参数：**
| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `project` | string | — | 按项目过滤 |
| `days` | number | `30` | 查询天数 |
| `group_by` | string | `hourly` | 聚合粒度：`hourly` 或 `daily` |
| `tz_offset` | number | — | 时区偏移（分钟） |

**响应：** `Vec<CostAggregate>` — 趋势数据。

#### `POST /admin/cost-records/prune`

清理历史费用记录。

**请求体：**
```json
{"olderThanDays": 90}
```

默认清理 90 天前的记录。

**响应：**
```json
{"deleted": 1500}
```

---

### Projects

#### `GET /admin/projects`

列出所有有费用记录的项目路径。

**响应：** `Vec<string>` — 项目路径列表。

---

### Switch Logs

#### `GET /admin/switch-logs`

查询通道切换日志。

**查询参数：**
| 参数 | 类型 | 说明 |
|------|------|------|
| `limit` | number | 返回条数限制 |

**响应：** `Vec<SwitchLog>` — 切换日志列表。

---

### Seed Data

#### `GET /admin/seed/status`

获取种子数据状态。

**查询参数：**
| 参数 | 类型 | 说明 |
|------|------|------|
| `remote` | boolean | 设为 `true` 同时检查远程更新（不应用） |

**响应：** `SeedStatus` 对象，包含 `localVersion`、`source`、`lastError` 等字段。

#### `POST /admin/seed/refresh`

触发种子数据远程刷新。

**请求体：**
```json
{"url": "https://example.com/seed"}
```

可选，不传则使用默认远程 URL。

**响应：**
```json
{
  "success": true,
  "previousVersion": 3,
  "newVersion": 4,
  "source": "remote",
  "errors": []
}
```

---

### Compress

#### `GET /admin/compress/status`

获取压缩中间件开关状态。

**响应：**
```json
{"enabled": true}
```

#### `POST /admin/compress/toggle`

切换压缩中间件开关。

**请求体：**
```json
{"enabled": false}
```

`enabled` 字段为必填布尔值，缺失返回 `400`。

**响应：**
```json
{"enabled": false}
```

---

## 端点统计

| 类别 | 数量 |
|------|------|
| 健康检查 | 2 |
| Providers | 2 |
| Models | 2 |
| Channels | 8 |
| Model Mappings | 4 |
| Model Aliases | 3 |
| Available Channels | 1 |
| Cost Records | 5 |
| Projects | 1 |
| Switch Logs | 1 |
| Seed Data | 2 |
| Compress | 2 |
| **合计** | **33** |

---

## 元信息

- 基于 commit：`642f0c3`（`perf: downgrade verbose request logs from info to debug`）
- 源码来源：`apps/server/src/admin.rs`、`apps/server/src/admin_auth.rs`、`crates/core/src/server.rs`
- 设计规范：`specs/0016-admin-api-extension.md`
- 生成日期：2026-06-12

---

Owner: baoyx · 版本：v1.0 · 生效日期：2026-06-12 · 最后更新：2026-06-12
