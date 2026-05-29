# 0016 - Admin API Extension

## 1. Existing Endpoints (Baseline)

agent-proxy-rust already provides these admin endpoints:

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/admin/providers` | List providers |
| `GET` | `/admin/providers/{id}` | Get single provider |
| `GET` | `/admin/models` | List models (filterable by provider_id) |
| `GET` | `/admin/models/{id}` | Get single model |
| `GET` | `/admin/channels` | List channels (filterable by model_id) |
| `GET` | `/admin/channels/{id}` | Get single channel |
| `PUT` | `/admin/channels/{id}` | Update channel config |
| `DELETE` | `/admin/channels/{id}` | Delete channel |
| `POST` | `/admin/channels/{id}/healthy` | Mark channel healthy |
| `POST` | `/admin/channels/{id}/failure` | Record channel failure |
| `GET` | `/admin/cost-records` | Query cost records (filterable) |
| `GET` | `/admin/health` | Global health status |

## 2. Gaps Identified (for token-fleet-switch integration)

| Gap | Priority | Reason |
|-----|----------|--------|
| Auth key management | P0 | Tauri needs to register/rotate/revoke auth tokens |
| Switch logs | P1 | Dashboard shows real-time switch events |
| Cost daily aggregation | P1 | Dashboard pre-aggregated views |
| Config reload | P1 | Hot-reload after channel edits |
| Provider/Model CRUD (POST/PUT/DELETE) | P2 | Tauri manages these in its own DB already |
| Channel creation | P2 | Tauri creates channels in its DB, pushes to proxy |

## 3. New Endpoints

### 3.1 Auth Keys

```
GET /admin/auth/keys
  Response: [
    {"key_hash_prefix": "abc123...", "role": "coder", "agent_type": "ClaudeCode", "created_at": ...},
    ...
  ]
  Note: Only first 8 chars of hash returned for display.

POST /admin/auth/register
  Request:  {"key_hash": "<SHA256>", "role": "coder", "agent_type": "ClaudeCode"}
  Response: 201 Created
  Error:    409 Conflict (duplicate hash)

PUT /admin/auth/rotate
  Request:  {"old_key_hash": "<SHA256>", "new_key_hash": "<SHA256>"}
  Response: 200 OK (role and agent_type preserved from old key)
  Error:    404 Not Found (old key not found)

DELETE /admin/auth/{key_hash}
  Response: 204 No Content
  Error:    404 Not Found
```

### 3.2 Switch Logs

```
GET /admin/switch-logs?limit=20&from_channel=X&to_channel=Y
  Response: [
    {
      "id": "...",
      "from_channel_id": "...",
      "to_channel_id": "...",
      "reason": "429_rate_limit",
      "cost_record_id": 12345,
      "created_at": 1716912000
    },
    ...
  ]

GET /admin/switch-logs/stats?from=<unix_ts>&to=<unix_ts>
  Response: {
    "total_switches": 42,
    "by_reason": {"429_rate_limit": 30, "5xx_error": 5, "quota_exhausted": 7},
    "by_channel": {"channel_a": 25, "channel_b": 17}
  }
```

### 3.3 Cost Daily

```
GET /admin/cost-daily?date=2026-05-29&channel=X
  Response: [
    {"date": "2026-05-29", "channel_id": "...", "total_cost": 1.23, "total_tokens": 50000, "request_count": 150},
    ...
  ]

GET /admin/cost-records/aggregate?group_by=project&from=<ts>&to=<ts>
  Response: [
    {"dimension": "token-fleet-switch", "total_cost": 12.50, "total_tokens": 500000, "count": 1500},
    ...
  ]

GET /admin/cost-records/savings?from=<ts>&to=<ts>
  Response: {
    "schema_saved_tokens": 50000,
    "response_saved_tokens": 30000,
    "rtk_saved_tokens": 10000,
    "total_saved_tokens": 90000,
    "estimated_cost_saved": 1.35
  }
```

### 3.4 Config Reload

```
POST /admin/config/reload
  Request: {
    "channels": [
      {"id": "...", "name": "Anthropic", "url": "https://...", "api_key": "sk-...",
       "protocol": "anthropic_messages", "billing_type": "metered", "priority": 1,
       "enabled": true, "health_status": "Healthy"}
    ],
    "model_mappings": [
      {"id": "...", "channel_id": "...", "client_name": "claude-sonnet",
       "upstream_name": "claude-sonnet-4-7", "billing": "metered", "pricing_json": "{...}"}
    ]
  }
  Response: {"status": "ok", "channels_loaded": 14, "mappings_loaded": 64}
  Error:    400 Bad Request (invalid payload)
```

### 3.5 Channel Creation

```
POST /admin/channels
  Request: {
    "id": "my-deepseek",
    "name": "My DeepSeek Channel",
    "url": "https://api.deepseek.com",
    "api_key": "sk-xxx",
    "protocol": "openai_chat",
    "billing_type": "metered",
    "priority": 10,
    "enabled": true
  }
  Response: 201 Created
  Error:    409 Conflict (duplicate id)
```

## 4. Authentication for Admin Endpoints

All `/admin/*` endpoints are protected by `x-admin-key` header:

```rust
async fn admin_auth_middleware(
    State(admin_key): State<SecretString>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let provided = req.headers()
        .get("x-admin-key")
        .and_then(|v| v.to_str().ok());
    if provided == Some(admin_key.expose_secret()) {
        Ok(next.run(req).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}
```

The `admin_key` is:
- Generated once at first Tauri startup: `generate_auth_token()`
- Stored in macOS Keychain (via keychain crate)
- Passed to agent-proxy-rust as `--admin-key` CLI argument at startup
- Never written to `settings.json` or any other config file

## 5. CORS Configuration

Tauri webview runs on a custom protocol (`tauri://localhost`). The Admin API
must allow cross-origin requests from the Tauri frontend:

```rust
use tower_http::cors::{Any, CorsLayer};

let cors = CorsLayer::new()
    .allow_origin(Any)          // Safe: admin API is localhost-only
    .allow_methods(Any)
    .allow_headers(Any);

let admin = Router::new()
    .route("/admin/...", ...)
    .layer(cors);
```

In production (when agent-proxy-rust runs on a remote server), CORS must be
restricted to the specific Tauri origin.

## 6. Error Response Format

All errors use consistent JSON format:

```json
{
  "error": {
    "code": "NOT_FOUND",
    "message": "channel with id 'xxx' not found"
  }
}
```

Standard HTTP status codes: 400 (Bad Request), 401 (Unauthorized), 404 (Not Found),
409 (Conflict), 500 (Internal).

## 7. Pagination

List endpoints support pagination:

```
GET /admin/cost-records?page=1&per_page=50
```

Response includes pagination metadata:

```json
{
  "data": [...],
  "pagination": {
    "page": 1,
    "per_page": 50,
    "total": 1250,
    "total_pages": 25
  }
}
```

## 8. Implementation Priority

| Priority | Endpoints | Rationale |
|----------|-----------|-----------|
| P0 | Auth key management (CRUD) | Token-fleet-switch auth flow depends on this |
| P0 | Config reload | Channel edits must be hot-reloaded |
| P1 | Switch logs | Dashboard health panel |
| P1 | Cost daily / aggregate / savings | Cost dashboard |
| P2 | Channel creation | Tauri can push new channels |
| P3 | Pagination | Performance for large datasets |
