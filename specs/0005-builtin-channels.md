# 0005 — Provider & Model Registry

> **Phase 1**: 4 core providers embedded (Anthropic/OpenAI/Google/DeepSeek). No remote fetch.
> **Phase 2**: Community repository `agent-proxy-pricing`, startup fetch (3s timeout), full 10+ provider registry.

> **Provider = 厂商** (who makes the model). **Channel = 渠道** (how you reach them). See `0003-channel-model.md` for channel management.

## Core Concepts

```
Provider（厂商）                         Channel（渠道）
─────────────────────────              ───────────────────────
Anthropic                              ├── Anthropic Official API
  ├── claude-opus-4-7                  ├── OpenRouter (转售)
  ├── claude-sonnet-4-6               └── AWS Bedrock (云平台)
  └── claude-haiku-4-5
                                       Each channel has its own:
OpenAI                                 • url + api_key
  ├── gpt-5.5                          • protocol (anthropic/openai)
  ├── gpt-5.4                          • pricing (may differ from official)
  └── gpt-5-codex                      • model name mapping
```

- **Provider** defines what models exist, their official pricing, and capabilities. Changes slowly (new model release, deprecation). Maintained in this spec and in the community pricing repository.
- **Channel** defines how to reach a provider — official API, reseller, cloud deployment. Each channel has its own URL, API key, and may have different pricing. Managed by users in SQLite. See `0003-channel-model.md`.

## Providers

### Anthropic

| model_id | official_name | input | output | cache_write | cache_read | context | notes |
|----------|--------------|-------|--------|-------------|------------|---------|-------|
| claude-opus | claude-opus-4-7 | $5.00 | $25.00 | $6.25 | $0.50 | 200K | thinking: $12.00/MTok |
| claude-sonnet | claude-sonnet-4-6 | $3.00 | $15.00 | $3.75 | $0.30 | 200K | |
| claude-haiku | claude-haiku-4-5 | $1.00 | $5.00 | $1.25 | $0.10 | 200K | |

> cache_write = 1.25× input, cache_read = 0.1× input. Flat rate regardless of context length.

### OpenAI

| model_id | official_name | input | output | cache_read | context | notes |
|----------|--------------|-------|--------|------------|---------|-------|
| gpt-5.5 | gpt-5.5 | $5.00 | $30.00 | $1.25 | 400K | latest flagship (2026-04) |
| gpt-5.4 | gpt-5.4 | $2.50 | $15.00 | $0.25 | 256K | |
| gpt-5-codex | gpt-5-codex | $1.25 | $10.00 | $0.125 | 256K | code specialist |

> cache_read = 0.1× input. All models use OpenAI Responses protocol by default.

### Google DeepMind

| model_id | official_name | input | output | context | notes |
|----------|--------------|-------|--------|---------|-------|
| gemini-pro | gemini-2.5-pro | $2.50 | $10.00 | 1M | |
| gemini-flash | gemini-2.5-flash | $0.15 | $0.60 | 1M | |

### DeepSeek

| model_id | official_name | input (CNY) | output (CNY) | cache_read (CNY) | context | notes |
|----------|--------------|-------------|--------------|------------------|---------|-------|
| deepseek-chat | deepseek-v4-flash | ¥1.00 | ¥2.00 | ¥0.02 | 128K | deprecates 2026-07-24 |
| deepseek-reasoner | deepseek-v4-flash | ¥1.00 | ¥2.00 | ¥0.02 | 128K | deprecates 2026-07-24, same pricing |
| deepseek-pro | deepseek-v4-pro | ¥3.00 | ¥6.00 | ¥0.025 | 256K | price cut to 1/4 on 2026-05-31 |

> No separate cache_write fee (first cache miss = standard input). cache_read ≈ 1/50~1/120 of input.

### Zhipu GLM (智谱)

| model_id | official_name | input (CNY) | output (CNY) | cache_read (CNY) | context | notes |
|----------|--------------|-------------|--------------|------------------|---------|-------|
| glm-5.1 | glm-5.1 | ¥1.00 | ¥3.20 | ¥0.475 | 200K | latest flagship (2026-04) |
| glm-5 | glm-5 | ¥0.80 | ¥2.40 | — | 200K | |
| glm-5-code | glm-5-code | ¥1.50 | ¥5.60 | — | 128K | code specialist |

> 2026 cumulative price increase ~83%. Cache write pricing undisclosed.

### Bailian / Qwen (阿里百炼)

| model_id | official_name | input (CNY) | output (CNY) | cache_read (CNY) | context | notes |
|----------|--------------|-------------|--------------|------------------|---------|-------|
| qwen-max | qwen3.7-max | ¥12.00 | ¥36.00 | ¥1.20 | 256K | MoE, 2026-05-22, currently 50% off |
| qwen-plus | qwen3.5-plus | ¥0.80 | ¥4.80 | ¥0.08 | ≤128K | best value |
| qwen-coder | qwen-coder | ¥1.00 | ¥4.00 | ¥0.10 | 128K | code specialist |
| qwen-flash | qwen3.5-flash | ¥0.20 | ¥0.80 | — | 128K | lowest cost |

> Tiered pricing (above = ≤128K). Longer context doubles rate. Batch at 50% off.

### Kimi (月之暗面)

| model_id | official_name | input (CNY) | output (CNY) | cache_read (CNY) | context | notes |
|----------|--------------|-------------|--------------|------------------|---------|-------|
| kimi-k2 | kimi-k2 | ¥4.00 | ¥16.00 | ¥1.10 | 128K | cache_read ≈ 0.25× input |
| kimi-k2-thinking | kimi-k2-thinking | ¥4.40 | ¥18.00 | ¥1.10 | 128K | thinking mode |
| kimi-k2.6 | kimi-k2.6 | ¥6.50 | ¥27.00 | ¥1.10 | 128K | latest, ~58% price increase |

### MiniMax

| model_id | official_name | input (CNY) | output (CNY) | cache_read (CNY) | context | notes |
|----------|--------------|-------------|--------------|------------------|---------|-------|
| minimax-m2.7 | minimax-m2.7 | ¥2.18 | ¥8.70 | ¥0.44 | 205K | output 1/21 of Claude Opus |

> Also offers Coding Plan subscription ¥99/month.

### DouBao (字节豆包)

| model_id | official_name | input (CNY) | output (CNY) | cache_read (CNY) | context | notes |
|----------|--------------|-------------|--------------|------------------|---------|-------|
| doubao-seed-1.8 | doubao-seed-1.8 | ¥0.80 | ¥8.00 | ¥0.16 | 128K | cache_read = 0.2× input |
| doubao-seed-code | doubao-seed-2.0-code | ¥3.20 | ¥16.00 | — | 128K | code specialist |
| doubao-seed-lite | doubao-seed-2.0-lite | ¥0.60 | ¥3.60 | — | 128K | lightweight |

> Tiered pricing (above = ≤32K input + >200 output). Deployed on Volcano Engine. 120T daily token volume.

### StepFun (阶跃星辰)

| model_id | official_name | input (CNY) | output (CNY) | notes |
|----------|--------------|-------------|--------------|-------|
| stepfun-step-plan | step-3 | ¥2.00 | ¥8.00 | Coding Plan bundle |

---

## Community Provider Registry

The full provider + pricing data lives in a separate community repository:

```
github.com/tokenfleet-ai/agent-proxy-pricing/
├── providers/
│   ├── anthropic.json
│   ├── openai.json
│   ├── google.json
│   ├── deepseek.json
│   ├── glm.json
│   ├── qwen.json
│   ├── kimi.json
│   ├── minimax.json
│   ├── doubao.json
│   └── stepfun.json
├── index.json              # provider list + version + sha256
├── schema.json             # JSON Schema for validation
└── CHANGELOG.md
```

The proxy fetches this on startup (with 3s timeout, falls back to builtin data).

### Provider JSON Format

```json
{
  "provider": {
    "id": "anthropic",
    "name": "Anthropic",
    "website": "https://www.anthropic.com",
    "pricing_page": "https://www.anthropic.com/pricing",
    "pricing_last_updated": "2026-05-26",
    "default_protocol": "anthropic_messages"
  },
  "models": [
    {
      "model_id": "claude-sonnet",
      "official_name": "claude-sonnet-4-6",
      "context_window": 200000,
      "pricing": {
        "mode": "per_token",
        "currency": "USD",
        "input_per_mtok": 3.0,
        "output_per_mtok": 15.0,
        "cache_write_per_mtok": 3.75,
        "cache_read_per_mtok": 0.30
      },
      "deprecated_after": null
    }
  ]
}
```

### Subscription Providers

Some providers offer subscription plans instead of per-token pricing:

```json
{
  "provider": {
    "id": "github-copilot",
    "name": "GitHub Copilot",
    "pricing_model": "subscription",
    "monthly_price_usd": 10.00,
    "quota": {
      "type": "speed_limited",
      "high_speed_tokens_per_month": 50000000
    },
    "on_exhausted": "fallback_to_metered"
  },
  "models": [
    { "model_id": "claude-sonnet", "official_name": "claude-sonnet-4-6" },
    { "model_id": "claude-haiku", "official_name": "claude-haiku-4-5" },
    { "model_id": "gpt-5-codex", "official_name": "gpt-5-codex" }
  ]
}
```

## Related Specs

- Channel management, seeding, and user workflow → `0003-channel-model.md`
- Cost calculation using provider pricing → `0004-cost-tracking.md`
- Community repository update mechanism → `0010-configuration.md`
