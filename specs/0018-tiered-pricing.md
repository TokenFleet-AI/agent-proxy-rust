# 梯度定价 & 多维度计价

## 概述

扩展定价模型，支持：
- **梯度定价**：按累积用量分区间，不同区间不同单价
- **多维度计价**：Token（LLM）、时长（视频）、数量（图片）
- **无梯度单价**：直接按单位计费

---

## 类型定义

### BillingDimension — 计费维度

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BillingDimension {
    /// Token 计费（LLM 文本 API）
    Tokens,
    /// 时长计费（视频/音频 API），resolution 可选
    Duration {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resolution: Option<String>,
    },
    /// 图片数量计费（图片生成 API），quality 可选
    Images {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        quality: Option<String>,
    },
}
```

### TierPrice — 梯度区间单价

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TierPrice {
    /// Token 类单价
    Token {
        input_per_mtok: f64,
        output_per_mtok: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache_write_per_mtok: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache_read_per_mtok: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thinking_per_mtok: Option<f64>,
    },
    /// 通用单价（时长/图片/次数）
    Unit {
        per_unit: f64,
    },
}
```

### PricingTier — 梯度区间

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingTier {
    /// 该区间上限（含），None = 无上限
    /// - Tokens: token 数量
    /// - Duration: 秒数
    /// - Images: 图片张数
    pub up_to: Option<u64>,
    /// 该区间单价
    pub price: TierPrice,
}
```

### Pricing — 扩展

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Pricing {
    // 现有变体
    PerToken { .. },
    Credits { .. },
    CharBased { .. },

    // 新增：按单位计费（无梯度）
    PerUnit {
        metric: BillingDimension,
        per_unit: f64,
        currency: String,
    },

    // 新增：梯度计费
    Tiered {
        dimension: BillingDimension,
        tiers: Vec<PricingTier>,
        #[serde(default = "default_currency")]
        currency: String,
    },
}
```

---

## JSON 示例

### Token 梯度（DeepSeek 风格）

```json
{
  "type": "tiered",
  "dimension": { "type": "tokens" },
  "currency": "CNY",
  "tiers": [
    {
      "up_to": 1000000000,
      "price": {
        "type": "token",
        "input_per_mtok": 1.0,
        "output_per_mtok": 2.0,
        "cache_read_per_mtok": 0.02
      }
    },
    {
      "up_to": 5000000000,
      "price": {
        "type": "token",
        "input_per_mtok": 0.8,
        "output_per_mtok": 1.6,
        "cache_read_per_mtok": 0.015
      }
    },
    {
      "up_to": null,
      "price": {
        "type": "token",
        "input_per_mtok": 0.5,
        "output_per_mtok": 1.0,
        "cache_read_per_mtok": 0.01
      }
    }
  ]
}
```

### 视频时长 — 无梯度（Sora 风格）

```json
{
  "type": "per_unit",
  "metric": { "type": "duration", "resolution": "1080p" },
  "per_unit": 0.50,
  "currency": "USD"
}
```

### 视频时长 — 梯度

```json
{
  "type": "tiered",
  "dimension": { "type": "duration", "resolution": "1080p" },
  "currency": "USD",
  "tiers": [
    { "up_to": 3600,  "price": { "type": "unit", "per_unit": 0.50 } },
    { "up_to": 36000, "price": { "type": "unit", "per_unit": 0.35 } },
    { "up_to": null,  "price": { "type": "unit", "per_unit": 0.20 } }
  ]
}
```

### 图片 — 无梯度

```json
{
  "type": "per_unit",
  "metric": { "type": "images", "quality": "hd" },
  "per_unit": 0.04,
  "currency": "USD"
}
```

### 图片 — 梯度

```json
{
  "type": "tiered",
  "dimension": { "type": "images", "quality": "standard" },
  "currency": "USD",
  "tiers": [
    { "up_to": 100,  "price": { "type": "unit", "per_unit": 0.04 } },
    { "up_to": 1000, "price": { "type": "unit", "per_unit": 0.03 } },
    { "up_to": null, "price": { "type": "unit", "per_unit": 0.02 } }
  ]
}
```

---

## 计费汇总

| 模式 | Pricing 变体 | 梯度 | 适用场景 |
|------|-------------|:--:|------|
| Token 固定单价 | `PerToken` | ✗ | LLM API，固定费率 |
| Token 梯度 | `Tiered { dimension: Tokens }` | ✓ | DeepSeek 等按量梯度 |
| 积分 | `Credits` | ✗ | 内部积分系统 |
| 字符 | `CharBased` | ✗ | 中文按字计费 |
| 时长（无梯度） | `PerUnit { metric: Duration }` | ✗ | 视频 API 按秒 |
| 时长（梯度） | `Tiered { dimension: Duration }` | ✓ | 视频批量折扣 |
| 图片（无梯度） | `PerUnit { metric: Images }` | ✗ | 图片 API 按张 |
| 图片（梯度） | `Tiered { dimension: Images }` | ✓ | 图片批量折扣 |

---

## 实施计划

### Phase 1（本次实施）— Token 梯度

- [ ] `crates/model-router/src/types.rs`：新增 `Pricing::Tiered`、`BillingDimension`、`TierPrice`、`PricingTier`、`Pricing::PerUnit`
- [ ] `crates/cost/src/lib.rs`：`calc_cost()` 新增 `Tiered { dimension: Tokens }` 分支
- [ ] `ChannelBilling::from_storage()`：支持解析 `"tiered"` billing 类型

### Phase 2（后续）— 视频/图片维度

- [ ] `calc_cost()` 支持 `Duration` / `Images` 维度
- [ ] `extract_usage()` 支持从响应中提取时长/图片数量

### Phase 3（后续）— 管理端

- [ ] Admin API 校验 tiered pricing JSON 有效性
- [ ] Dashboard 展示梯度价格配置
