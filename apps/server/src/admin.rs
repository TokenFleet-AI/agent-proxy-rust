//! Admin API handlers for provider/model/channel management.
//!
//! These endpoints let token-fleet-switch manage configuration via HTTP
//! instead of direct `SQLite` access.

use std::sync::Arc;

use agent_proxy_rust_storage::{
    Channel, CostFilter, CostRecord, Model, Provider, Storage, StorageError,
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post, put},
};
use serde::{Deserialize, Serialize};
use tower_http::cors::{Any, CorsLayer};

/// Shared state for admin handlers.
#[derive(Clone)]
pub struct AdminState {
    pub storage: Arc<dyn Storage>,
}

/// Builds the admin API router with auth middleware.
///
/// If `admin_key` is `Some`, all `/admin/*` routes are protected by
/// the `x-admin-key` header check.
pub fn admin_routes(storage: Arc<dyn Storage>, admin_key: Option<String>) -> Router {
    use crate::admin_auth::{AdminAuthLayer, admin_auth_middleware};
    use axum::middleware;

    let state = AdminState { storage };
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
        // Cost
        .route("/admin/cost-records", get(query_cost_records))
        // Model Mappings
        .route("/admin/model-mappings", get(list_model_mappings))
        // Health
        .route("/admin/health", get(admin_health))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
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
}

/// Cost records query params.
#[derive(Debug, Deserialize)]
struct CostRecordsQuery {
    project: Option<String>,
    model_name: Option<String>,
    channel_name: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
}

/// Wire result type.
type ApiResult<T> = Result<Json<T>, AppError>;

/// Unified error response.
#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

struct AppError {
    status: StatusCode,
    message: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        (
            self.status,
            Json(ErrorBody {
                error: self.message,
            }),
        )
            .into_response()
    }
}

impl From<StorageError> for AppError {
    fn from(e: StorageError) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: e.to_string(),
        }
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
        None => Err(AppError {
            status: StatusCode::NOT_FOUND,
            message: format!("provider not found: {id}"),
        }),
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
        None => Err(AppError {
            status: StatusCode::NOT_FOUND,
            message: format!("model not found: {id}"),
        }),
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
        None => Err(AppError {
            status: StatusCode::NOT_FOUND,
            message: format!("channel not found: {id}"),
        }),
    }
}

async fn update_channel(
    State(state): State<AdminState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateChannelBody>,
) -> ApiResult<Channel> {
    let updated = state
        .storage
        .update_channel(
            &id,
            body.name.as_deref(),
            body.enabled,
            body.priority,
            body.monthly_quota,
            body.quota_policy.as_deref(),
        )
        .await
        .map_err(|e| match &e {
            StorageError::NotFound(_) => AppError {
                status: StatusCode::NOT_FOUND,
                message: format!("channel not found: {id}"),
            },
            _ => AppError::from(e),
        })?;
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
            StorageError::NotFound(_) => AppError {
                status: StatusCode::NOT_FOUND,
                message: format!("channel not found: {id}"),
            },
            _ => AppError::from(e),
        })?;
    Ok(Json(serde_json::json!({"deleted": true})))
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
    let filter = CostFilter {
        project_path: query.project,
        model_name: query.model_name,
        channel_name: query.channel_name,
        time_range: None,
        limit: query.limit,
        offset: query.offset,
    };
    let records = state.storage.query_cost_records(filter).await?;
    Ok(Json(records))
}

async fn set_channel_api_key(
    State(state): State<AdminState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<serde_json::Value> {
    let key_str = body["apiKey"].as_str().unwrap_or("");
    let secret = secrecy::SecretString::new(key_str.to_string().into_boxed_str());
    state.storage.set_channel_api_key(&id, &secret).await?;
    // Also mark healthy since user just provided a valid key
    state.storage.mark_channel_healthy(&id).await?;
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
    Ok(Json(serde_json::json!({"healthy": healthy})))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]
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
        // Use a known test key so auth passes
        admin_routes(Arc::new(storage), Some("test-admin-key".into()))
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
        let app = admin_routes(Arc::new(storage), Some("secret".into()));
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
        let app = admin_routes(Arc::new(storage), Some("correct".into()));
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
        assert_eq!(providers.len(), 5);
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
        assert_eq!(channels.len(), 7);
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
}
