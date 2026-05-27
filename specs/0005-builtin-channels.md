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

### DeepSeek

DeepSeek thinks in deep (思考模式), providing reasoning capabilities. Both models support JSON output, tool calls, prefix continuation (Beta), and FIM completion (Beta, non-thinking mode only).

BASE URLs: OpenAI `api.deepseek.com` / Anthropic `api.deepseek.com/anthropic`.

| model_id | input (cache miss) | input (cache hit) | output | context | notes |
|----------|--------------------|--------------------|--------|---------|-------|
| deepseek-v4-flash | ¥1.00 | ¥0.02 | ¥2.00 | 1M | max output 384K, concurrency 2500 |
| deepseek-v4-pro | ¥3.00 | ¥0.025 | ¥6.00 | 1M | max output 384K, concurrency 500; 2.5折 (原价 ¥12 / ¥0.1 / ¥24) |

> No separate cache_write fee. Both support thinking and non-thinking modes.

### Zhipu GLM (智谱)

All prices in CNY/MTok. Cache write: free (限时免费). Input length tiers: `[0, 32K)` / `[32K+)`.

| model_id | input | output | cache_read | context | notes |
|----------|-------|--------|------------|---------|-------|
| glm-5.1 | ¥6 / ¥8 | ¥24 / ¥28 | ¥1.3 / ¥2 | 200K | latest flagship |
| glm-5-turbo | ¥5 / ¥7 | ¥22 / ¥26 | ¥1.2 / ¥1.8 | 200K | |
| glm-5 | ¥4 / ¥6 | ¥18 / ¥22 | ¥1 / ¥1.5 | 200K | |
| glm-4.7 | ¥2–4 tiered | ¥8–16 tiered | ¥0.4–0.8 tiered | 200K | 3-tier by input+output length |
| glm-4.5-air | ¥0.8–1.2 tiered | ¥2–8 tiered | ¥0.16–0.24 tiered | 128K | 3-tier by input+output length |
| glm-4.7-flashx | ¥0.5 | ¥3 | ¥0.1 | 200K | budget with cache |
| glm-4.7-flash | free | free | free | 200K | free tier |

> glm-4.7 tiers: [0,32K)+[0,0.2K)out ¥2/¥8 → [0,32K)+[0.2K+)out ¥3/¥14 → [32K,200K) ¥4/¥16.
> glm-4.5-air tiers: [0,32K)+[0,0.2K)out ¥0.8/¥2 → [0,32K)+[0.2K+)out ¥0.8/¥6 → [32K,128K) ¥1.2/¥8.

### Bailian / Qwen (阿里百炼)

| model_id | official_name | input (CNY) | output (CNY) | cache_write (CNY) | cache_read (CNY) | context | notes |
|----------|--------------|-------------|--------------|-------------------|------------------|---------|-------|
| qwen3.7-max | qwen3.7-max | ¥6.00 | ¥18.00 | ¥7.50 | ¥1.20 | 256K | latest flagship, 50% off (原价 input ¥12 / output ¥36) |
| qwen3.6-max | qwen3.6-max | ¥9.00 | ¥54.00 | ¥11.25 | ¥0.90 | 256K | preview, vibe coding specialist, max input 240K |
| qwen3.6-plus | qwen3.6-plus | ¥2.00 | ¥12.00 | ¥2.50 | ¥0.20 | 1M | vision, thinking, function calling; snapshot `qwen3.6-plus-2026-04-02` |
| qwen3.6-flash | qwen3.6-flash | ¥1.20 | ¥7.20 | ¥1.50 | ¥0.12 | 1M | vision, thinking, agentic coding, math reasoning |
| qwen3.5-plus | qwen3.5-plus | ¥0.80 | ¥4.80 | ¥1.00 | ¥0.08 | 1M | vision, thinking; snapshot `qwen3.5-plus-2026-02-15` |
| qwen3.5-flash | qwen3.5-flash | ¥0.20 | ¥2.00 | ¥0.25 | ¥0.02 | 1M | vision, thinking, fastest; Batch Chat 50% off |

> qwen3.7-max: Batch File input ¥6 / output ¥18 per MTok. Tool calls: code_interpreter (free), web_search (¥4/千次), web_extractor (free).
> qwen3.6-plus: Batch File input ¥1 / output ¥6. Batch Chat 50% off. Max input 991K (thinking 983K), max output 64K, thinking 80K. RPM 30000, TPM 5000000. Tool calls: web_search (¥4/千次), code_interpreter (free), web_extractor (free), i2i_search (¥48/千次), t2i_search (¥24/千次).

### Kimi (月之暗面)

| model_id | official_name | input (CNY) | output (CNY) | cache_read (CNY) | context | notes |
|----------|--------------|-------------|--------------|------------------|---------|-------|
| kimi-k2.6 | kimi-k2.6 | ¥6.50 | ¥27.00 | ¥1.10 | 256K | latest, ~58% price increase |
| kimi-k2.5 | kimi-k2.5 | ¥4.00 | ¥21.00 | ¥0.70 | 256K | |

### MiniMax

| model_id | input | output | cache_read | cache_write | notes |
|----------|-------|--------|------------|-------------|-------|
| minimax-m2.7 | ¥2.10 | ¥8.40 | ¥0.42 | ¥2.625 | base |
| minimax-m2.7-highspeed | ¥4.20 | ¥16.80 | ¥0.42 | ¥2.625 | 2× price for faster response |
| minimax-m2.5 | ¥2.10 | ¥8.40 | ¥0.21 | ¥2.625 | previous gen, same base price |
| minimax-m2.5-highspeed | ¥4.20 | ¥16.80 | ¥0.21 | ¥2.625 | |

> All prices CNY/MTok. Also offers Coding Plan subscription ¥99/month.


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
