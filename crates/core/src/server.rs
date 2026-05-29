//! The axum-based proxy server engine.
//!
//! Provides the [`AgentProxy`] builder, router, and request handler.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::{Request, Response, StatusCode},
    middleware,
    response::IntoResponse,
    routing::{get, post},
};
use tokio::task::JoinHandle;
use tower_http::limit::RequestBodyLimitLayer;

use crate::{
    auth::{self, AgentRole, AuthState},
    config::ProxyConfig,
    error::ProxyError,
    middleware::{ProxyMiddleware, run_on_request_chain, run_on_response_chain},
    types::{ApiFormat, ConnectionContext, ProxyRequest, ProxyResponse, detect_agent_type},
};

/// Shared state for the proxy server.
#[derive(Clone)]
pub struct ProxyState {
    /// Proxy configuration.
    pub config: Arc<ProxyConfig>,
    /// Registered middleware chain.
    pub middlewares: Arc<Vec<Box<dyn ProxyMiddleware>>>,
    /// Reusable HTTP client for upstream forwarding.
    pub client: reqwest::Client,
    next_request_id: Arc<AtomicU64>,
}

impl ProxyState {
    fn next_request_id(&self) -> u64 {
        self.next_request_id.fetch_add(1, Ordering::SeqCst)
    }
}

impl std::fmt::Debug for ProxyState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mw_names: Vec<&str> = self.middlewares.iter().map(|m| m.name()).collect();
        f.debug_struct("ProxyState")
            .field("config", &self.config)
            .field("middlewares", &mw_names)
            .field("client", &self.client)
            .field("next_request_id", &self.next_request_id)
            .finish()
    }
}

/// The proxy application.
///
/// Created via [`AgentProxy::builder()`] and started with [`AgentProxy::serve()`].
pub struct AgentProxy {
    config: ProxyConfig,
    middlewares: Arc<Vec<Box<dyn ProxyMiddleware>>>,
}

impl AgentProxy {
    /// Creates a new [`AgentProxyBuilder`].
    #[must_use]
    pub fn builder() -> AgentProxyBuilder {
        AgentProxyBuilder::default()
    }

    /// Returns the axum [`Router`] for this proxy without starting a server.
    /// Useful for combining with other routers (e.g., admin API).
    ///
    /// # Errors
    ///
    /// Returns [`ProxyError::Config`] if the reqwest client cannot be built
    /// from the proxy configuration.
    pub fn into_router(self) -> Result<Router, ProxyError> {
        let client = build_reqwest_client(&self.config)?;
        let state = Arc::new(ProxyState {
            config: Arc::new(self.config),
            middlewares: self.middlewares,
            client,
            next_request_id: Arc::new(AtomicU64::new(1)),
        });
        Ok(build_router(state))
    }

    /// Starts the proxy server and returns a [`JoinHandle`].
    ///
    /// Runs `on_init` on all middlewares before binding.
    ///
    /// # Errors
    ///
    /// Returns a [`ProxyError`] if binding to the listen address fails.
    pub async fn serve(self) -> Result<JoinHandle<()>, ProxyError> {
        let client = build_reqwest_client(&self.config)?;

        let state = Arc::new(ProxyState {
            config: Arc::new(self.config),
            middlewares: self.middlewares,
            client,
            next_request_id: Arc::new(AtomicU64::new(1)),
        });

        // Run on_init hooks
        for mw in state.middlewares.iter() {
            mw.on_init().await?;
        }

        let app = build_router(state.clone());
        let listener = tokio::net::TcpListener::bind(state.config.listen)
            .await
            .map_err(|e| ProxyError::Internal(e.into()))?;

        tracing::info!("agent-proxy listening on {}", state.config.listen);

        let handle = tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                tracing::error!("server error: {e}");
            }
        });

        Ok(handle)
    }
}

/// Builder for [`AgentProxy`].
///
/// # Example
///
/// ```rust,ignore
/// use agent_proxy_rust_core::{AgentProxy, ProxyConfig};
///
/// let proxy = AgentProxy::builder()
///     .config(ProxyConfig::default())
///     .middleware(my_middleware)
///     .build()
///     .unwrap();
/// ```
#[derive(Default)]
pub struct AgentProxyBuilder {
    config: Option<ProxyConfig>,
    middlewares: Vec<Box<dyn ProxyMiddleware>>,
}

impl AgentProxyBuilder {
    /// Sets the proxy configuration.
    #[must_use]
    pub fn config(mut self, config: ProxyConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// Adds a middleware to the chain (in registration order).
    #[must_use]
    pub fn middleware<M: ProxyMiddleware + 'static>(mut self, m: M) -> Self {
        self.middlewares.push(Box::new(m));
        self
    }

    /// Builds the [`AgentProxy`].
    ///
    /// # Errors
    ///
    /// Returns a [`ProxyError`] if no config was provided.
    pub fn build(self) -> Result<AgentProxy, ProxyError> {
        let config = self
            .config
            .ok_or_else(|| ProxyError::Internal(anyhow::anyhow!("config is required")))?;
        Ok(AgentProxy {
            config,
            middlewares: Arc::new(self.middlewares),
        })
    }
}

impl std::fmt::Debug for AgentProxyBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mw_names: Vec<&str> = self.middlewares.iter().map(|m| m.name()).collect();
        f.debug_struct("AgentProxyBuilder")
            .field("config", &self.config)
            .field("middlewares", &mw_names)
            .finish()
    }
}

impl std::fmt::Debug for AgentProxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mw_names: Vec<&str> = self.middlewares.iter().map(|m| m.name()).collect();
        f.debug_struct("AgentProxy")
            .field("config", &self.config)
            .field("middlewares", &mw_names)
            .finish()
    }
}

/// Builds the reqwest client for upstream forwarding.
fn build_reqwest_client(config: &ProxyConfig) -> Result<reqwest::Client, ProxyError> {
    reqwest::Client::builder()
        .connect_timeout(config.upstream_connect_timeout)
        .timeout(config.upstream_timeout)
        .http1_only()
        .build()
        .map_err(|e| ProxyError::Internal(e.into()))
}

/// Builds the axum router.
fn build_router(state: Arc<ProxyState>) -> Router {
    let auth_state = AuthState::from_config(&state.config);

    Router::new()
        .route("/v1/messages", post(handle_proxy_request))
        .route("/v1/chat/completions", post(handle_proxy_request))
        .route("/v1/responses", post(handle_proxy_request))
        .route("/health", get(handle_health))
        .layer(middleware::from_fn_with_state(
            auth_state,
            auth::auth_middleware,
        ))
        .layer(RequestBodyLimitLayer::new(state.config.max_body_size))
        .with_state(state)
}

/// Health check handler.
async fn handle_health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status": "ok"}))
}

/// Single dispatch handler for all AI API endpoints.
///
/// 1. Detects the API format from the path.
/// 2. Detects the agent type from headers.
/// 3. Reads the auth role from request extensions.
/// 4. Runs the `on_request` middleware chain.
/// 5. Forwards to upstream (streaming or non-streaming).
/// 6. Runs the `on_response` middleware chain.
/// 7. Returns the response to the client.
async fn handle_proxy_request(
    State(state): State<Arc<ProxyState>>,
    req: Request<Body>,
) -> Response<Body> {
    let request_id = state.next_request_id();
    let path = req.uri().path().to_string();
    let detected_format = ApiFormat::from_path(&path);

    // Unknown path → 404
    if detected_format.is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": {"code": "not_found", "message": format!("no route for {path}")}
            })),
        )
            .into_response();
    }

    // Read body with size check
    let (parts, body) = req.into_parts();

    // Check Content-Length header for early rejection
    let body_too_large = parts
        .headers
        .get("content-length")
        .and_then(|cl| cl.to_str().ok())
        .and_then(|s| s.parse::<usize>().ok())
        .is_some_and(|len| len > state.config.max_body_size);

    if body_too_large {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({
                "error": {
                    "code": "body_too_large",
                    "message": format!("request body exceeds size limit of {}", state.config.max_body_size)
                }
            })),
        )
            .into_response();
    }

    let body_bytes = match axum::body::to_bytes(body, state.config.max_body_size).await {
        Ok(b) => b,
        Err(_e) => {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(serde_json::json!({
                    "error": {
                        "code": "body_too_large",
                        "message": "request body exceeds size limit"
                    }
                })),
            )
                .into_response();
        }
    };

    let agent_type = detect_agent_type(&parts.headers, &path);
    let agent_role = parts.extensions.get::<AgentRole>().map(|r| r.0.clone());

    let mut proxy_req = ProxyRequest::new(parts.method, path, parts.headers, body_bytes);

    let mut ctx = ConnectionContext::new(request_id, agent_type, agent_role, detected_format);

    // on_request chain (registration order)
    if let Err(e) = run_on_request_chain(&state.middlewares, &mut proxy_req, &mut ctx).await {
        log_error(&e, &ctx);
        return e.to_response();
    }

    // Get upstream target from extensions (set by model-router middleware)
    let channel = ctx.get::<crate::types::ChannelConfig>(crate::extensions::EXT_SELECTED_CHANNEL);

    if let Some(ch) = channel {
        let url = ch.url.clone();
        let api_key = Some(ch.api_key.clone());
        let is_streaming = proxy_req.is_streaming();

        match forward_to_upstream(&state.client, &proxy_req, &url, api_key.as_deref()).await {
            Ok(upstream_resp) => {
                if is_streaming {
                    handle_streaming_response(upstream_resp, &state, &ctx).await
                } else {
                    handle_non_streaming_response(upstream_resp, &state, &ctx).await
                }
            }
            Err(e) => {
                log_error(&e, &ctx);
                e.to_response()
            }
        }
    } else {
        let err = ProxyError::ChannelSelection {
            model: "unknown".into(),
        };
        log_error(&err, &ctx);
        err.to_response()
    }
}

/// Handles a non-streaming upstream response.
async fn handle_non_streaming_response(
    upstream_resp: reqwest::Response,
    state: &Arc<ProxyState>,
    ctx: &ConnectionContext,
) -> Response<Body> {
    let status = upstream_resp.status();
    let headers = upstream_resp.headers().clone();

    let body_bytes = match upstream_resp.bytes().await {
        Ok(b) => b,
        Err(e) => {
            let err = ProxyError::Upstream {
                source: format!("failed to read upstream response: {e}"),
                inner: Some(e.into()),
            };
            log_error(&err, ctx);
            return err.to_response();
        }
    };

    let mut proxy_resp = ProxyResponse::new(status, headers, body_bytes, false);

    if let Err(e) = run_on_response_chain(&state.middlewares, &mut proxy_resp, ctx).await {
        log_error(&e, ctx);
        return e.to_response();
    }

    build_axum_response(proxy_resp)
}

/// Handles a streaming upstream response.
///
/// For the MVP, buffers the full response body. SSE frame-by-frame transformation
/// will be implemented when the bridge middleware adds `transform_stream` support.
async fn handle_streaming_response(
    upstream_resp: reqwest::Response,
    state: &Arc<ProxyState>,
    ctx: &ConnectionContext,
) -> Response<Body> {
    let status = upstream_resp.status();
    let headers = upstream_resp.headers().clone();

    // Buffer the full streaming response (frame-by-frame transform is Phase 2)
    let body_bytes = match upstream_resp.bytes().await {
        Ok(b) => b,
        Err(e) => {
            let err = ProxyError::Upstream {
                source: format!("failed to read streaming response: {e}"),
                inner: Some(e.into()),
            };
            log_error(&err, ctx);
            return err.to_response();
        }
    };

    let mut proxy_resp = ProxyResponse::new(status, headers, body_bytes, true);

    if let Err(e) = run_on_response_chain(&state.middlewares, &mut proxy_resp, ctx).await {
        log_error(&e, ctx);
        return e.to_response();
    }

    build_axum_response(proxy_resp)
}

/// Forwards the proxy request to the upstream server.
async fn forward_to_upstream(
    client: &reqwest::Client,
    proxy_req: &ProxyRequest,
    upstream_url: &str,
    api_key: Option<&str>,
) -> Result<reqwest::Response, ProxyError> {
    // Build the upstream URL (append the path)
    let url = if upstream_url.ends_with('/') {
        format!("{}{}", upstream_url.trim_end_matches('/'), &proxy_req.path)
    } else {
        format!("{}{}", upstream_url, proxy_req.path)
    };

    let mut req_builder = client
        .request(proxy_req.method.clone(), &url)
        .body(proxy_req.body.to_vec());

    // Apply header forwarding policy: drop hop-by-hop and auth headers
    for (key, value) in &proxy_req.headers {
        let key_str = key.as_str().to_lowercase();
        let should_drop = matches!(
            key_str.as_str(),
            "transfer-encoding"
                | "connection"
                | "keep-alive"
                | "accept-encoding"
                | "host"
                | "content-length"
                | "authorization"
                | "x-api-key"
        );
        if !should_drop {
            req_builder = req_builder.header(key.clone(), value.clone());
        }
    }

    // Inject upstream auth
    if let Some(key) = api_key {
        req_builder = req_builder.header("Authorization", format!("Bearer {key}"));
    }

    req_builder.send().await.map_err(|e| {
        if e.is_timeout() {
            ProxyError::Upstream {
                source: format!("upstream timeout: {e}"),
                inner: Some(e.into()),
            }
        } else if e.is_connect() {
            ProxyError::Upstream {
                source: format!("upstream connection failed: {e}"),
                inner: Some(e.into()),
            }
        } else {
            ProxyError::Upstream {
                source: format!("upstream request failed: {e}"),
                inner: Some(e.into()),
            }
        }
    })
}

/// Builds an axum [`Response`] from a [`ProxyResponse`].
fn build_axum_response(proxy_resp: ProxyResponse) -> Response<Body> {
    let mut response = Response::new(Body::from(proxy_resp.body));
    *response.status_mut() = proxy_resp.status;
    for (key, value) in &proxy_resp.headers {
        if is_forward_header(key.as_str()) {
            response.headers_mut().insert(key.clone(), value.clone());
        }
    }
    response
}

/// Returns `true` if the header should be forwarded from upstream to client.
fn is_forward_header(name: &str) -> bool {
    let lower = name.to_lowercase();
    !matches!(
        lower.as_str(),
        "transfer-encoding"
            | "connection"
            | "keep-alive"
            | "content-length"
            | "host"
            | "authorization"
            | "x-api-key"
    )
}

/// Logs an error with appropriate severity.
fn log_error(err: &ProxyError, ctx: &ConnectionContext) {
    match err {
        ProxyError::Internal(e) => {
            tracing::error!(
                request_id = ctx.request_id,
                error = %e,
                "internal error"
            );
        }
        ProxyError::Upstream { source, .. } => {
            tracing::warn!(
                request_id = ctx.request_id,
                error = %source,
                "upstream error"
            );
        }
        _ => {
            tracing::debug!(
                request_id = ctx.request_id,
                error = %err,
                "request error"
            );
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use async_trait::async_trait;
    use axum::{body::Body, http::StatusCode};
    use tower::ServiceExt;

    use super::*;
    use crate::{middleware::ProxyMiddleware, types::ChannelConfig};

    /// Mock middleware that sets an upstream URL via extensions.
    struct UpstreamMiddleware {
        url: String,
    }

    #[async_trait]
    impl ProxyMiddleware for UpstreamMiddleware {
        async fn on_request(
            &self,
            _req: &mut ProxyRequest,
            ctx: &mut ConnectionContext,
        ) -> Result<(), ProxyError> {
            ctx.insert(
                crate::extensions::EXT_SELECTED_CHANNEL,
                ChannelConfig {
                    url: self.url.clone(),
                    api_key: "sk-test".into(),
                    protocol: ApiFormat::AnthropicMessages,
                    name: "test".into(),
                },
            );
            Ok(())
        }

        async fn on_response(
            &self,
            _res: &mut ProxyResponse,
            _ctx: &ConnectionContext,
        ) -> Result<(), ProxyError> {
            Ok(())
        }

        fn name(&self) -> &'static str {
            "upstream"
        }
    }

    /// Builds a test-only router (without server binding).
    fn build_test_router(
        config: ProxyConfig,
        middlewares: Vec<Box<dyn ProxyMiddleware>>,
    ) -> Router {
        let client = reqwest::Client::builder()
            .http1_only()
            .build()
            .expect("build test client");

        let state = Arc::new(ProxyState {
            config: Arc::new(config),
            middlewares: Arc::new(middlewares),
            client,
            next_request_id: Arc::new(AtomicU64::new(1)),
        });

        build_router(state)
    }

    #[tokio::test]
    async fn test_health_endpoint_returns_200() {
        let config = ProxyConfig::default();
        let router = build_test_router(config, vec![]);

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_unknown_path_returns_404() {
        let config = ProxyConfig::default();
        let router = build_test_router(config, vec![]);

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/unknown/path")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"model":"test"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_auth_failure_returns_401() {
        let config = ProxyConfig {
            proxy_api_key: Some(secrecy::SecretString::new("sk-secret".into())),
            ..Default::default()
        };
        let router = build_test_router(config, vec![]);

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_auth_success_passes_through() {
        let config = ProxyConfig {
            proxy_api_key: Some(secrecy::SecretString::new("sk-secret".into())),
            ..Default::default()
        };
        let router = build_test_router(config, vec![]);

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .header("authorization", "Bearer sk-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_body_too_large_returns_413() {
        let config = ProxyConfig {
            max_body_size: 1024, // 1KB limit
            ..Default::default()
        };
        let router = build_test_router(config, vec![]);

        let big_body = "x".repeat(2048);
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/v1/messages")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(big_body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn test_no_channel_returns_503() {
        let config = ProxyConfig::default();
        let router = build_test_router(config, vec![]);

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/v1/messages")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"model":"claude-sonnet","messages":[{"role":"user","content":"hi"}]}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    /// Starts a simple HTTP server for testing upstream forwarding.
    async fn start_mock_upstream() -> (String, JoinHandle<()>) {
        use axum::routing::post;

        async fn mock_messages_handler() -> Json<serde_json::Value> {
            Json(serde_json::json!({
                "id": "msg_123",
                "type": "message",
                "role": "assistant",
                "content": [{"type": "text", "text": "Hello from upstream!"}],
                "model": "claude-sonnet",
                "usage": {"input_tokens": 10, "output_tokens": 20}
            }))
        }

        let app = Router::new().route("/v1/messages", post(mock_messages_handler));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("local addr");

        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn test_successful_proxy_returns_200() {
        let (upstream_url, _upstream_handle) = start_mock_upstream().await;

        let config = ProxyConfig::default();
        let middlewares: Vec<Box<dyn ProxyMiddleware>> =
            vec![Box::new(UpstreamMiddleware { url: upstream_url })];

        let router = build_test_router(config, middlewares);

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/v1/messages")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"model":"claude-sonnet","max_tokens":1024,"messages":[{"role":"user","content":"hello"}]}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
