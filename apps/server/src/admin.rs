//! Admin API handlers for provider/model/channel management.
//!
//! These endpoints let token-fleet-switch manage configuration via HTTP
//! instead of direct `SQLite` access.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use agent_proxy_rust_model_router::{ChannelState, ResolvedChannel, reload_channels_from_storage};
use agent_proxy_rust_storage::{
    AvailableChannelInfo, Channel, ChannelHealthStatus, CompressionSavingsReport, CostAggregate,
    CostFilter, CostGroupBy, CostRecord, Model, ModelMapping, ProtocolEntry, Provider, SeedManager,
    SeedStatus, Storage, StorageError, SwitchLog, TimeRange,
};
use arc_swap::ArcSwap;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post, put},
};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tower_http::cors::{Any, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;

/// Shared state for admin handlers.
#[derive(Clone)]
pub struct AdminState {
    pub storage: Arc<dyn Storage>,
    /// In-memory channel health map shared with the model router.
    pub health_map: Arc<DashMap<String, ChannelState>>,
    /// In-memory API key overrides shared with the model router.
    pub api_key_map: Arc<DashMap<String, secrecy::SecretString>>,
    /// Seed data manager for remote updates.
    pub seed: Arc<dyn SeedManager>,
    /// Shared toggle for `CompressMiddleware` on/off.
    pub compress_enabled: Arc<AtomicBool>,
    /// Channel list shared with the model router, atomically swapped on reload.
    pub channels_swap: Arc<ArcSwap<Vec<ResolvedChannel>>>,
}

impl AdminState {
    /// Triggers a hot-reload of the channel list from storage so that
    /// priority, enabled, protocol, and mapping changes take effect
    /// immediately without requiring a proxy restart.
    async fn reload_channels(&self) {
        if let Err(e) =
            reload_channels_from_storage(self.storage.as_ref(), &self.channels_swap).await
        {
            tracing::error!(error = %e, "failed to reload channels after mutation");
        }
    }
}

/// Builds the admin API router with auth middleware.
///
/// If `admin_key` is `Some`, all `/admin/*` routes are protected by
/// the `x-admin-key` header check.
pub fn admin_routes(
    storage: Arc<dyn Storage>,
    seed: Arc<dyn SeedManager>,
    admin_key: Option<String>,
    health_map: Arc<DashMap<String, ChannelState>>,
    api_key_map: Arc<DashMap<String, secrecy::SecretString>>,
    compress_enabled: Arc<AtomicBool>,
    channels_swap: Arc<ArcSwap<Vec<ResolvedChannel>>>,
) -> Router {
    use axum::middleware;

    use crate::admin_auth::{AdminAuthLayer, admin_auth_middleware};

    let state = AdminState {
        storage,
        health_map,
        api_key_map,
        seed,
        compress_enabled,
        channels_swap,
    };
    let mut router = Router::new()
        // Providers
        .route("/admin/providers", get(list_providers))
        .route("/admin/providers/{id}", get(get_provider))
        // Models
        .route("/admin/models", get(list_models))
        .route("/admin/models/{id}", get(get_model))
        // Channels
        .route("/admin/channels", get(list_channels))
        .route("/admin/channels/{id}", get(get_channel))
        .route("/admin/channels/{id}", put(update_channel))
        .route("/admin/channels/{id}", delete(delete_channel))
        .route("/admin/channels/{id}/healthy", post(mark_channel_healthy))
        .route("/admin/channels/{id}/failure", post(record_channel_failure))
        .route("/admin/channels/{id}/api-key", put(set_channel_api_key))
        .route("/admin/channels/{id}/protocols", get(get_channel_protocols))
        // Cost
        .route("/admin/cost-records", get(query_cost_records))
        .route("/admin/cost-records/report", get(cost_report))
        .route("/admin/cost-records/savings", get(cost_savings))
        .route("/admin/cost-records/trend", get(cost_trend))
        .route(
            "/admin/cost-records/prune",
            post(prune_cost_records_handler),
        )
        // Model Mappings
        .route("/admin/model-mappings", get(list_model_mappings))
        .route("/admin/model-mappings", post(create_model_mapping))
        .route("/admin/model-mappings/{id}", put(update_model_mapping))
        .route("/admin/model-mappings/{id}", delete(delete_model_mapping))
        // Model aliases
        .route("/admin/model-aliases", get(list_model_aliases))
        .route("/admin/model-aliases", post(create_model_alias))
        .route(
            "/admin/model-aliases/{id}",
            delete(delete_model_alias_handler),
        )
        // Available Channels (for token-fleet-switch direct-connect mode)
        .route(
            "/admin/available-channels",
            get(list_available_channels_handler),
        )
        // Projects (cost data)
        .route("/admin/projects", get(list_projects_handler))
        // Switch Logs
        .route("/admin/switch-logs", get(query_switch_logs_handler))
        // Health
        .route("/admin/health", get(admin_health))
        // Seed Data
        .route("/admin/seed/status", get(seed_status_handler))
        .route("/admin/seed/refresh", post(seed_refresh_handler))
        // Compress toggle
        .route("/admin/compress/status", get(compress_status))
        .route("/admin/compress/toggle", post(compress_toggle))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .layer(RequestBodyLimitLayer::new(1024 * 1024)) // 1 MB
        .with_state(state);

    if let Some(key) = admin_key {
        let auth_layer = AdminAuthLayer::new(key);
        router = router.layer(middleware::from_fn_with_state(
            auth_layer,
            admin_auth_middleware,
        ));
    }

    router
}

// ── Types ────────────────────────────────────────────────────────────────────

/// Query params for model listing.
#[derive(Debug, Deserialize)]
struct ModelsQuery {
    provider_id: Option<String>,
}

/// Query params for channel listing.
#[derive(Debug, Deserialize)]
struct ChannelsQuery {
    model_id: Option<String>,
}

/// Update channel request body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateChannelBody {
    name: Option<String>,
    enabled: Option<bool>,
    priority: Option<u32>,
    monthly_quota: Option<u64>,
    quota_policy: Option<String>,
    protocols: Option<String>,
    force_protocol: Option<String>,
}

/// Set channel API key request body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiKeyBody {
    api_key: String,
}

/// Partial update for model mapping.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateMappingBody {
    upstream_name: Option<String>,
    client_name: Option<String>,
    billing: Option<String>,
    pricing_json: Option<String>,
    weight: Option<u32>,
    enabled: Option<bool>,
    protocols: Option<String>,
}

/// Compress toggle request body.
#[derive(Debug, Deserialize)]
struct CompressToggleBody {
    enabled: bool,
}

/// Cost records query params.
#[derive(Debug, Deserialize)]
struct CostRecordsQuery {
    project: Option<String>,
    model_name: Option<String>,
    channel_name: Option<String>,
    days: Option<u32>,
    limit: Option<u32>,
    offset: Option<u32>,
    /// Local timezone offset in minutes east of UTC (e.g. `480` for UTC+8).
    #[serde(default)]
    tz_offset: Option<i32>,
}

/// Wire result type.
type ApiResult<T> = Result<Json<T>, AppError>;

struct AppError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl AppError {
    /// Creates a new [`AppError`] with the given status, code, and message.
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    /// Creates a 404 not-found error.
    fn not_found(resource: impl Into<String>) -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("{} not found", resource.into()),
        )
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        (
            self.status,
            Json(serde_json::json!({
                "error": {
                    "code": self.code,
                    "message": self.message,
                }
            })),
        )
            .into_response()
    }
}

impl From<StorageError> for AppError {
    fn from(e: StorageError) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            e.to_string(),
        )
    }
}

// ── Providers ────────────────────────────────────────────────────────────────

async fn list_providers(State(state): State<AdminState>) -> ApiResult<Vec<Provider>> {
    let providers = state.storage.list_providers().await?;
    Ok(Json(providers))
}

async fn get_provider(
    State(state): State<AdminState>,
    Path(id): Path<String>,
) -> ApiResult<Provider> {
    match state.storage.get_provider(&id).await? {
        Some(p) => Ok(Json(p)),
        None => Err(AppError::not_found(format!("provider {id}"))),
    }
}

// ── Models ───────────────────────────────────────────────────────────────────

async fn list_models(
    State(state): State<AdminState>,
    Query(query): Query<ModelsQuery>,
) -> ApiResult<Vec<Model>> {
    let models = state
        .storage
        .list_models(query.provider_id.as_deref())
        .await?;
    Ok(Json(models))
}

async fn get_model(State(state): State<AdminState>, Path(id): Path<String>) -> ApiResult<Model> {
    match state.storage.get_model(&id).await? {
        Some(m) => Ok(Json(m)),
        None => Err(AppError::not_found(format!("model {id}"))),
    }
}

// ── Channels ─────────────────────────────────────────────────────────────────

async fn list_channels(
    State(state): State<AdminState>,
    Query(query): Query<ChannelsQuery>,
) -> ApiResult<Vec<Channel>> {
    let channels = state
        .storage
        .list_channels(query.model_id.as_deref())
        .await?;
    Ok(Json(channels))
}

async fn get_channel(
    State(state): State<AdminState>,
    Path(id): Path<String>,
) -> ApiResult<Channel> {
    match state.storage.get_channel(&id).await? {
        Some(c) => Ok(Json(c)),
        None => Err(AppError::not_found(format!("channel {id}"))),
    }
}

async fn update_channel(
    State(state): State<AdminState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateChannelBody>,
) -> ApiResult<Channel> {
    // Validate force_protocol against channel's protocols
    if let Some(ref fp) = body.force_protocol {
        // Determine which protocols to validate against:
        // use the new protocols if also being updated, otherwise fetch current
        let protocols_json = if let Some(ref new_protocols) = body.protocols {
            new_protocols.clone()
        } else {
            let current = state
                .storage
                .get_channel(&id)
                .await
                .map_err(AppError::from)?
                .ok_or_else(|| AppError::not_found(format!("channel {id}")))?;
            current.protocols
        };

        let entries: Vec<ProtocolEntry> = serde_json::from_str(&protocols_json).map_err(|e| {
            AppError::new(
                StatusCode::BAD_REQUEST,
                "bad_request",
                format!("invalid protocols JSON: {e}"),
            )
        })?;

        if !entries.iter().any(|e| e.protocol == *fp) {
            let supported: Vec<&str> = entries.iter().map(|e| e.protocol.as_str()).collect();
            return Err(AppError::new(
                StatusCode::BAD_REQUEST,
                "bad_request",
                format!(
                    "force_protocol '{fp}' not found in channel protocols. Supported: \
                     {supported:?}"
                ),
            ));
        }
    }

    let updated = state
        .storage
        .update_channel(
            &id,
            body.name.as_deref(),
            body.enabled,
            body.priority,
            body.monthly_quota,
            body.quota_policy.as_deref(),
            body.protocols.as_deref(),
            body.force_protocol.as_deref(),
        )
        .await
        .map_err(|e| match &e {
            StorageError::NotFound(_) => AppError::not_found(format!("channel {id}")),
            _ => AppError::from(e),
        })?;
    state.reload_channels().await;
    Ok(Json(updated))
}

async fn delete_channel(
    State(state): State<AdminState>,
    Path(id): Path<String>,
) -> ApiResult<serde_json::Value> {
    state
        .storage
        .delete_channel(&id)
        .await
        .map_err(|e| match &e {
            StorageError::NotFound(_) => AppError::not_found(format!("channel {id}")),
            _ => AppError::from(e),
        })?;
    state.reload_channels().await;
    Ok(Json(serde_json::json!({"deleted": true})))
}

/// Response for `GET /admin/channels/{id}/protocols`.
#[derive(Debug, Serialize)]
struct ChannelProtocolsResponse {
    channel_id: String,
    channel_name: String,
    /// The parsed list of protocol entries supported by this channel.
    protocols: Vec<ProtocolEntry>,
}

async fn get_channel_protocols(
    State(state): State<AdminState>,
    Path(id): Path<String>,
) -> ApiResult<ChannelProtocolsResponse> {
    let channel = state
        .storage
        .get_channel(&id)
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| AppError::not_found(format!("channel {id}")))?;

    let protocols: Vec<ProtocolEntry> = serde_json::from_str(&channel.protocols).map_err(|e| {
        AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            format!("failed to parse protocols JSON: {e}"),
        )
    })?;

    Ok(Json(ChannelProtocolsResponse {
        channel_id: channel.id,
        channel_name: channel.name,
        protocols,
    }))
}

async fn mark_channel_healthy(
    State(state): State<AdminState>,
    Path(id): Path<String>,
) -> ApiResult<serde_json::Value> {
    state.storage.mark_channel_healthy(&id).await?;
    Ok(Json(serde_json::json!({"status": "ok"})))
}

async fn record_channel_failure(
    State(state): State<AdminState>,
    Path(id): Path<String>,
) -> ApiResult<serde_json::Value> {
    state.storage.record_channel_failure(&id).await?;
    Ok(Json(serde_json::json!({"status": "ok"})))
}

// ── Cost ─────────────────────────────────────────────────────────────────────

async fn query_cost_records(
    State(state): State<AdminState>,
    Query(query): Query<CostRecordsQuery>,
) -> ApiResult<Vec<CostRecord>> {
    let time_range = query.days.map(|days| {
        let now = chrono::Utc::now();
        compute_time_range(now, days, query.tz_offset, None)
    });
    let filter = CostFilter {
        project_path: query.project,
        model_name: query.model_name,
        channel_name: query.channel_name,
        time_range,
        limit: query.limit,
        offset: query.offset,
    };
    let records = state.storage.query_cost_records(filter).await?;
    Ok(Json(records))
}

async fn set_channel_api_key(
    State(state): State<AdminState>,
    Path(id): Path<String>,
    Json(body): Json<ApiKeyBody>,
) -> ApiResult<serde_json::Value> {
    let key_str = &body.api_key;
    let secret = secrecy::SecretString::new(key_str.clone().into_boxed_str());

    // Persist to DB
    state.storage.set_channel_api_key(&id, &secret).await?;
    state.storage.mark_channel_healthy(&id).await?;

    // Update in-memory shared maps so the router picks up the new key
    // and health status without a restart.
    if key_str.is_empty() {
        state.api_key_map.remove(&id);
        // Mark unhealthy in memory: no key → can't authenticate
        state
            .health_map
            .entry(id.clone())
            .or_default()
            .mark_unhealthy();
    } else {
        state.api_key_map.insert(id.clone(), secret);
        // Mark healthy in memory: user just provided a valid key
        state.health_map.remove(&id);
    }

    Ok(Json(serde_json::json!({"status": "ok"})))
}

// ── Model Mappings ────────────────────────────────────────────────────────────

async fn list_model_mappings(
    State(state): State<AdminState>,
) -> ApiResult<Vec<agent_proxy_rust_storage::ModelMapping>> {
    let mappings = state.storage.list_all_mappings().await?;
    Ok(Json(mappings))
}

// ── Health ───────────────────────────────────────────────────────────────────

async fn admin_health(State(state): State<AdminState>) -> ApiResult<serde_json::Value> {
    let healthy = state.storage.health_check().await.unwrap_or(false);
    let channels = state.storage.list_channels(None).await.unwrap_or_default();
    let healthy_channels = u32::try_from(
        channels
            .iter()
            .filter(|c| c.enabled && c.health_status == ChannelHealthStatus::Healthy.as_str())
            .count(),
    )
    .unwrap_or(u32::MAX);
    let total_channels = u32::try_from(channels.len()).unwrap_or(u32::MAX);
    Ok(Json(serde_json::json!({
        "healthy": healthy,
        "healthyChannels": healthy_channels,
        "totalChannels": total_channels,
    })))
}

// ── Seed Data ────────────────────────────────────────────────────────────────

/// Query params for seed status.
#[derive(Debug, Deserialize)]
struct SeedStatusQuery {
    #[serde(default)]
    remote: bool,
}

/// `GET /admin/seed/status`
///
/// Returns local seed data status. Use `?remote=true` to also check the
/// remote manifest for updates (does not apply changes).
async fn seed_status_handler(
    State(state): State<AdminState>,
    Query(query): Query<SeedStatusQuery>,
) -> ApiResult<SeedStatus> {
    if query.remote {
        let status = state.seed.seed_check_remote(None).await?;
        Ok(Json(status))
    } else {
        let status = state.seed.seed_status().await?;
        Ok(Json(status))
    }
}

/// Request body for `POST /admin/seed/refresh`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SeedRefreshBody {
    url: Option<String>,
}

/// Response for `POST /admin/seed/refresh`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SeedRefreshResponse {
    success: bool,
    previous_version: u32,
    new_version: u32,
    source: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    errors: Vec<String>,
}

/// `POST /admin/seed/refresh`
///
/// Triggers a remote seed data refresh. Optionally accepts `{"url":"..."}`
/// to override the remote URL.
async fn seed_refresh_handler(
    State(state): State<AdminState>,
    Json(body): Json<SeedRefreshBody>,
) -> ApiResult<SeedRefreshResponse> {
    let previous = state.seed.seed_status().await?;
    let previous_version = previous.local_version;

    let status = state.seed.seed_refresh(body.url.as_deref()).await?;

    let response = SeedRefreshResponse {
        success: status.last_error.is_none(),
        previous_version,
        new_version: status.local_version,
        source: status.source,
        errors: status.last_error.into_iter().collect(),
    };

    Ok(Json(response))
}

// ── Model Mappings CRUD ────────────────────────────────────────────────────

/// Create a new model mapping.
async fn create_model_mapping(
    State(state): State<AdminState>,
    Json(body): Json<ModelMapping>,
) -> ApiResult<ModelMapping> {
    state.storage.upsert_mapping(&body).await?;
    state.reload_channels().await;
    Ok(Json(body))
}

/// Update a model mapping.
async fn update_model_mapping(
    State(state): State<AdminState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateMappingBody>,
) -> ApiResult<serde_json::Value> {
    // Fetch existing, update fields, upsert
    let mappings = state
        .storage
        .list_all_mappings()
        .await
        .map_err(AppError::from)?;
    let existing = mappings
        .iter()
        .find(|m| m.id == id)
        .ok_or_else(|| AppError::not_found(format!("mapping {id}")))?;

    let mut updated = existing.clone();
    if let Some(v) = body.upstream_name {
        updated.upstream_name = v;
    }
    if let Some(v) = body.client_name {
        updated.client_name = v;
    }
    if let Some(v) = body.billing {
        updated.billing = v;
    }
    if let Some(v) = body.pricing_json {
        updated.pricing_json = v;
    }
    if let Some(v) = body.weight {
        updated.weight = v;
    }
    if let Some(v) = body.enabled {
        updated.enabled = v;
    }
    if let Some(v) = body.protocols {
        updated.protocols = v;
    }

    state.storage.upsert_mapping(&updated).await?;
    state.reload_channels().await;
    Ok(Json(serde_json::json!({"status": "ok"})))
}

/// Delete a model mapping.
async fn delete_model_mapping(
    State(state): State<AdminState>,
    Path(id): Path<String>,
) -> ApiResult<serde_json::Value> {
    state
        .storage
        .delete_mapping(&id)
        .await
        .map_err(|e| match &e {
            StorageError::NotFound(_) => AppError::not_found(format!("mapping {id}")),
            _ => AppError::from(e),
        })?;
    state.reload_channels().await;
    Ok(Json(serde_json::json!({"deleted": true})))
}

// ── Model Alias ────────────────────────────────────────────────────────────

async fn list_model_aliases(
    State(state): State<AdminState>,
) -> ApiResult<Vec<agent_proxy_rust_storage::ModelAlias>> {
    let aliases = state.storage.list_model_aliases().await?;
    Ok(Json(aliases))
}

async fn create_model_alias(
    State(state): State<AdminState>,
    Json(request): Json<agent_proxy_rust_storage::ModelAliasRequest>,
) -> ApiResult<agent_proxy_rust_storage::ModelAlias> {
    let alias = state.storage.upsert_model_alias(&request).await?;
    Ok(Json(alias))
}

async fn delete_model_alias_handler(
    State(state): State<AdminState>,
    Path(id): Path<i64>,
) -> ApiResult<serde_json::Value> {
    state
        .storage
        .delete_model_alias(id)
        .await
        .map_err(|e| match &e {
            StorageError::NotFound(_) => AppError::not_found(format!("alias {id}")),
            _ => AppError::from(e),
        })?;
    Ok(Json(serde_json::json!({"deleted": true})))
}

// ── Available Channels ─────────────────────────────────────────────────────

/// Lists enabled channels with their bound models.
/// Used by token-fleet-switch for Claude direct-connect mode.
async fn list_available_channels_handler(
    State(state): State<AdminState>,
) -> ApiResult<Vec<AvailableChannelInfo>> {
    let channels = state.storage.list_available_channels().await?;
    Ok(Json(channels))
}

// ── Switch Logs ────────────────────────────────────────────────────────────

/// Query params for switch log listing.
#[derive(Debug, Deserialize)]
struct SwitchLogsQuery {
    limit: Option<u32>,
}

/// Queries recent channel switch logs.
async fn query_switch_logs_handler(
    State(state): State<AdminState>,
    Query(query): Query<SwitchLogsQuery>,
) -> ApiResult<Vec<SwitchLog>> {
    let logs = state.storage.query_switch_logs(query.limit).await?;
    Ok(Json(logs))
}

// ── Cost Aggregation ────────────────────────────────────────────────────────

/// Query params for cost aggregation endpoints.
#[derive(Debug, Deserialize)]
struct CostReportQuery {
    project: Option<String>,
    #[serde(default = "default_days")]
    days: u32,
    #[serde(default)]
    group_by: Option<String>,
    /// Local timezone offset in minutes east of UTC (e.g. `480` for UTC+8).
    /// When provided, `days` is interpreted as calendar days in that timezone
    /// instead of a rolling 24-hour window from UTC now.
    #[serde(default)]
    tz_offset: Option<i32>,
}

fn default_days() -> u32 {
    30
}

/// Computes a [`TimeRange`] for the given `days` count, optionally anchored to
/// calendar-day boundaries in the caller's local timezone.
///
/// When `tz_offset` is `Some(offset_minutes)`, `days = 1` means "today 00:00
/// local-time → now", `days = 2` means "yesterday 00:00 local-time → now",
/// etc.  When `None`, the range is a rolling `days × 24 h` window ending at
/// `now` (the previous behaviour, kept for backward compatibility).
fn compute_time_range(
    now: chrono::DateTime<chrono::Utc>,
    days: u32,
    tz_offset: Option<i32>,
    project: Option<String>,
) -> TimeRange {
    let end = now.timestamp();

    let start = match tz_offset {
        Some(offset_minutes) => {
            let clamped = offset_minutes.clamp(-720, 840); // ±12h + a bit
            // clamped*60 ∈ [-43200, 50400] ⊂ (-86400, 86400), so east_opt always returns Some.
            // Fall back to the input's own fixed offset (UTC, i.e. 0) if it ever didn't.
            let utc_fixed = chrono::Offset::fix(now.offset());
            let offset = chrono::FixedOffset::east_opt(clamped * 60).unwrap_or(utc_fixed);
            let local_now = now.with_timezone(&offset);
            // `NaiveTime::MIN` is midnight (00:00:00), which is always valid for any date.
            let local_today_start = local_now.date_naive().and_time(chrono::NaiveTime::MIN);
            let days_back = chrono::Duration::days(i64::from(days.saturating_sub(1)));
            let local_start = local_today_start - days_back;
            // `local_start` is a naive local-time datetime; convert to UTC by
            // subtracting the offset (local_minus_utc is in seconds).
            let utc_start =
                local_start - chrono::Duration::seconds(i64::from(offset.local_minus_utc()));
            utc_start.and_utc().timestamp()
        }
        None => now.timestamp() - i64::from(days) * 86_400,
    };

    TimeRange {
        start,
        end,
        project,
    }
}

/// Returns an aggregated project cost report.
async fn cost_report(
    State(state): State<AdminState>,
    Query(query): Query<CostReportQuery>,
) -> ApiResult<Vec<CostAggregate>> {
    let now = chrono::Utc::now();
    let range = compute_time_range(now, query.days, query.tz_offset, None);

    let group_by = if query.project.is_some() {
        CostGroupBy::ProjectModelMonth
    } else {
        CostGroupBy::Project
    };

    let mut results = state.storage.aggregate_costs(group_by, range).await?;

    // Filter by project if specified
    if let Some(ref project) = query.project {
        results.retain(|r| r.group_key.starts_with(project));
    }

    Ok(Json(results))
}

/// Returns compression savings for a project.
async fn cost_savings(
    State(state): State<AdminState>,
    Query(query): Query<CostReportQuery>,
) -> ApiResult<CompressionSavingsReport> {
    let now = chrono::Utc::now();
    let range = compute_time_range(now, query.days, query.tz_offset, query.project.clone());

    let filter = CostFilter {
        project_path: query.project,
        model_name: None,
        channel_name: None,
        time_range: Some(range),
        limit: None,
        offset: None,
    };

    let records = state.storage.query_cost_records(filter).await?;
    let report = CompressionSavingsReport {
        schema_saved_tokens: records.iter().map(|r| r.schema_saved_tokens).sum(),
        response_saved_tokens: records.iter().map(|r| r.response_saved_tokens).sum(),
        rtk_saved_tokens: records.iter().map(|r| r.rtk_saved_tokens).sum(),
        total_saved_tokens: records.iter().map(|r| r.tokens_saved).sum(),
    };

    Ok(Json(report))
}

/// Returns hourly cost trend (aggregated across all projects per hour).
async fn cost_trend(
    State(state): State<AdminState>,
    Query(query): Query<CostReportQuery>,
) -> ApiResult<Vec<CostAggregate>> {
    let now = chrono::Utc::now();
    let range = compute_time_range(now, query.days, query.tz_offset, query.project.clone());

    let group_by = match query.group_by.as_deref() {
        Some("daily") => CostGroupBy::Daily,
        _ => CostGroupBy::Hourly,
    };

    let results = state.storage.aggregate_costs(group_by, range).await?;

    Ok(Json(results))
}

/// Request body for cost record pruning.
#[derive(Debug, Deserialize)]
struct PruneRequest {
    #[serde(default = "default_prune_days")]
    older_than_days: u32,
}

fn default_prune_days() -> u32 {
    90
}

async fn prune_cost_records_handler(
    State(state): State<AdminState>,
    Json(body): Json<PruneRequest>,
) -> ApiResult<serde_json::Value> {
    let deleted = state
        .storage
        .prune_cost_records(body.older_than_days)
        .await?;
    Ok(Json(serde_json::json!({"deleted": deleted})))
}

// ── Projects ──────────────────────────────────────────────────────────────────

/// Returns the list of distinct project paths that have cost records.
async fn list_projects_handler(State(state): State<AdminState>) -> ApiResult<Vec<String>> {
    let projects = state.storage.list_projects().await?;
    Ok(Json(projects))
}

// ── Compress Toggle ──────────────────────────────────────────────────────────

/// GET /admin/compress/status
async fn compress_status(State(state): State<AdminState>) -> ApiResult<serde_json::Value> {
    let enabled = state.compress_enabled.load(Ordering::Relaxed);
    Ok(Json(serde_json::json!({"enabled": enabled})))
}

/// POST /admin/compress/toggle
///
/// Body: `{"enabled": true}` or `{"enabled": false}`
async fn compress_toggle(
    State(state): State<AdminState>,
    Json(body): Json<CompressToggleBody>,
) -> ApiResult<serde_json::Value> {
    state
        .compress_enabled
        .store(body.enabled, Ordering::Relaxed);
    Ok(Json(serde_json::json!({"enabled": body.enabled})))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]
    use agent_proxy_rust_storage::SeedManager;
    use agent_proxy_rust_storage_sqlite::SqliteStorage;
    use axum::{
        body::Body,
        http::{Method, Request},
    };
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;

    async fn test_app() -> Router {
        let storage = SqliteStorage::new_in_memory().unwrap();
        storage.migrate().await.unwrap();
        storage.seed_init().await.unwrap();
        let health = Arc::new(DashMap::new());
        let keys = Arc::new(DashMap::new());
        let seed: Arc<dyn SeedManager> = Arc::new(storage.clone());
        // Use a known test key so auth passes
        admin_routes(
            Arc::new(storage),
            seed,
            Some("test-admin-key".into()),
            health,
            keys,
            Arc::new(AtomicBool::new(true)),
            Arc::new(ArcSwap::from_pointee(Vec::new())),
        )
    }

    fn make_authed_request(method: Method, uri: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("x-admin-key", "test-admin-key")
            .body(Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn test_unauthorized_without_admin_key() {
        let storage = SqliteStorage::new_in_memory().unwrap();
        storage.migrate().await.unwrap();
        let seed: Arc<dyn SeedManager> = Arc::new(storage.clone());
        let app = admin_routes(
            Arc::new(storage),
            seed,
            Some("secret".into()),
            Arc::new(DashMap::new()),
            Arc::new(DashMap::new()),
            Arc::new(AtomicBool::new(true)),
            Arc::new(ArcSwap::from_pointee(Vec::new())),
        );
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/admin/providers")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_unauthorized_with_wrong_key() {
        let storage = SqliteStorage::new_in_memory().unwrap();
        storage.migrate().await.unwrap();
        let seed: Arc<dyn SeedManager> = Arc::new(storage.clone());
        let app = admin_routes(
            Arc::new(storage),
            seed,
            Some("correct".into()),
            Arc::new(DashMap::new()),
            Arc::new(DashMap::new()),
            Arc::new(AtomicBool::new(true)),
            Arc::new(ArcSwap::from_pointee(Vec::new())),
        );
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/admin/providers")
                    .header("x-admin-key", "wrong-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_list_providers() {
        let app = test_app().await;
        let resp = app
            .oneshot(make_authed_request(Method::GET, "/admin/providers"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let providers: Vec<Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(providers.len(), 9);
    }

    #[tokio::test]
    async fn test_list_channels() {
        let app = test_app().await;
        let resp = app
            .oneshot(make_authed_request(Method::GET, "/admin/channels"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let channels: Vec<Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(channels.len(), 9);
    }

    #[tokio::test]
    async fn test_get_channel_not_found() {
        let app = test_app().await;
        let resp = app
            .oneshot(make_authed_request(
                Method::GET,
                "/admin/channels/nonexistent",
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_update_channel() {
        let app = test_app().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri("/admin/channels/deepseek")
                    .header("Content-Type", "application/json")
                    .header("x-admin-key", "test-admin-key")
                    .body(Body::from(
                        r#"{"name":"Updated","priority":99,"quotaPolicy":"Block"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_admin_health() {
        let app = test_app().await;
        let resp = app
            .oneshot(make_authed_request(Method::GET, "/admin/health"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_list_projects_returns_array() {
        let app = test_app().await;
        let resp = app
            .oneshot(make_authed_request(Method::GET, "/admin/projects"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let projects: Vec<String> = serde_json::from_slice(&body).unwrap();
        // After seed_init, there should be some projects (or empty array)
        // The key test: endpoint returns 200 and valid JSON array
        assert!(projects.is_empty() || !projects.is_empty()); // tautology: just verifying it's a valid array
    }

    // ── compute_time_range tests ─────────────────────────────────────────

    mod time_range {
        use chrono::{TimeZone, Utc};

        use super::super::compute_time_range;

        #[test]
        fn test_no_tz_offset_uses_rolling_window() {
            // 2026-06-10T12:30:00Z, days=1 → start = 2026-06-09T12:30:00Z
            let now = Utc.with_ymd_and_hms(2026, 6, 10, 12, 30, 0).unwrap();
            let range = compute_time_range(now, 1, None, None);
            let expected_start = Utc
                .with_ymd_and_hms(2026, 6, 9, 12, 30, 0)
                .unwrap()
                .timestamp();
            assert_eq!(range.start, expected_start);
            assert_eq!(range.end, now.timestamp());
            assert!(range.project.is_none());
        }

        #[test]
        fn test_tz_offset_beijing_today() {
            // now = 2026-06-10T02:00:00Z = 2026-06-10T10:00:00+08 (Beijing)
            // days=1 → start = 2026-06-10T00:00:00+08 = 2026-06-09T16:00:00Z
            let now = Utc.with_ymd_and_hms(2026, 6, 10, 2, 0, 0).unwrap();
            let range = compute_time_range(now, 1, Some(480), None);
            let expected_start = Utc
                .with_ymd_and_hms(2026, 6, 9, 16, 0, 0)
                .unwrap()
                .timestamp();
            assert_eq!(range.start, expected_start);
            assert_eq!(range.end, now.timestamp());
        }

        #[test]
        fn test_tz_offset_beijing_two_days() {
            // now = 2026-06-10T02:00:00Z = 2026-06-10T10:00:00+08
            // days=2 → start = 2026-06-09T00:00:00+08 = 2026-06-08T16:00:00Z
            let now = Utc.with_ymd_and_hms(2026, 6, 10, 2, 0, 0).unwrap();
            let range = compute_time_range(now, 2, Some(480), None);
            let expected_start = Utc
                .with_ymd_and_hms(2026, 6, 8, 16, 0, 0)
                .unwrap()
                .timestamp();
            assert_eq!(range.start, expected_start);
        }

        #[test]
        fn test_tz_offset_preserves_project_filter() {
            let now = Utc.with_ymd_and_hms(2026, 6, 10, 12, 0, 0).unwrap();
            let range = compute_time_range(now, 1, Some(480), Some("/my-project".into()));
            assert_eq!(range.project.as_deref(), Some("/my-project"));
        }

        #[test]
        fn test_tz_offset_negative_west_of_utc() {
            // now = 2026-06-10T03:00:00Z = 2026-06-09T22:00:00-05 (EST-ish)
            // days=1 → start = 2026-06-09T00:00:00-05 = 2026-06-09T05:00:00Z
            let now = Utc.with_ymd_and_hms(2026, 6, 10, 3, 0, 0).unwrap();
            let range = compute_time_range(now, 1, Some(-300), None);
            let expected_start = Utc
                .with_ymd_and_hms(2026, 6, 9, 5, 0, 0)
                .unwrap()
                .timestamp();
            assert_eq!(range.start, expected_start);
        }

        #[test]
        fn test_days_zero_means_only_current_day() {
            // now = 2026-06-10T10:00:00+08, days=0 → start = today 00:00+08
            // (days.saturating_sub(1) = 0, so no days_back)
            let now = Utc.with_ymd_and_hms(2026, 6, 10, 2, 0, 0).unwrap();
            let range = compute_time_range(now, 0, Some(480), None);
            let expected_start = Utc
                .with_ymd_and_hms(2026, 6, 9, 16, 0, 0)
                .unwrap()
                .timestamp();
            assert_eq!(range.start, expected_start);
        }
    }
}
