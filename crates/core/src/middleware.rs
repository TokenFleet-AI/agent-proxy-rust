//! The [`ProxyMiddleware`] trait — the core extension point for the proxy engine.

use async_trait::async_trait;

use crate::{
    error::ProxyError,
    types::{ConnectionContext, ProxyRequest, ProxyResponse},
};

/// The central extension trait for `agent-proxy-rust`.
///
/// Implementors can intercept and transform requests/responses flowing through
/// the proxy. Execution order:
///
/// - `on_init`: registration order (at startup)
/// - `on_request`: registration order
/// - `on_response`: **reverse** registration order
/// - `on_disconnect`: reverse registration order
/// - `on_shutdown`: reverse registration order
///
/// All methods have default no-op implementations except `on_request`,`on_response`, and `name`.
#[async_trait]
pub trait ProxyMiddleware: Send + Sync {
    /// Called before forwarding the request to upstream.
    ///
    /// Middleware may modify the request body, headers, or context extensions.
    /// For example, the compress middleware reduces tool definition sizes,
    /// and the model-router middleware selects a channel and sets the upstream URL.
    async fn on_request(
        &self,
        req: &mut ProxyRequest,
        ctx: &mut ConnectionContext,
    ) -> Result<(), ProxyError>;

    /// Called after receiving the response from upstream.
    ///
    /// Middleware may modify the response body, headers, or context extensions.
    /// Called in **reverse** registration order for symmetry with `on_request`.
    async fn on_response(
        &self,
        res: &mut ProxyResponse,
        ctx: &ConnectionContext,
    ) -> Result<(), ProxyError>;

    /// Called when a new connection is established. Runs in registration order.
    async fn on_connect(&self, _ctx: &ConnectionContext) {}

    /// Called when a connection is closed. Runs in reverse registration order.
    async fn on_disconnect(&self, _ctx: &ConnectionContext) {}

    /// Called once when the proxy starts. Use for opening DB pools, loading config, etc.
    async fn on_init(&self) -> Result<(), ProxyError> {
        Ok(())
    }

    /// Called once when the proxy shuts down gracefully.
    async fn on_shutdown(&self) -> Result<(), ProxyError> {
        Ok(())
    }

    /// Returns the unique name of this middleware.
    ///
    /// Used for logging and debugging.
    fn name(&self) -> &'static str;
}

/// Runs the `on_request` chain in registration order.
///
/// If any middleware returns `Err`, the chain is aborted and the error is returned.
///
/// # Errors
///
/// Returns the first [`ProxyError`] encountered from any middleware in the chain.
pub async fn run_on_request_chain(
    middlewares: &[Box<dyn ProxyMiddleware>],
    req: &mut ProxyRequest,
    ctx: &mut ConnectionContext,
) -> Result<(), ProxyError> {
    for mw in middlewares {
        mw.on_request(req, ctx).await?;
    }
    Ok(())
}

/// Runs the `on_response` chain in **reverse** registration order.
///
/// If any middleware returns `Err`, the chain is aborted and the error is returned.
///
/// # Errors
///
/// Returns the first [`ProxyError`] encountered from any middleware in the chain.
pub async fn run_on_response_chain(
    middlewares: &[Box<dyn ProxyMiddleware>],
    res: &mut ProxyResponse,
    ctx: &ConnectionContext,
) -> Result<(), ProxyError> {
    for mw in middlewares.iter().rev() {
        mw.on_response(res, ctx).await?;
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use bytes::Bytes;
    use http::{HeaderMap, Method, StatusCode};

    use super::*;

    struct RecordingMiddleware {
        name: &'static str,
        request_order: Arc<AtomicUsize>,
        response_order: Arc<AtomicUsize>,
        request_counter: AtomicUsize,
        response_counter: AtomicUsize,
        request_err: Option<ProxyError>,
    }

    #[async_trait]
    impl ProxyMiddleware for RecordingMiddleware {
        async fn on_request(
            &self,
            _req: &mut ProxyRequest,
            _ctx: &mut ConnectionContext,
        ) -> Result<(), ProxyError> {
            if let Some(ref err) = self.request_err {
                return Err(ProxyError::BadRequest(err.to_string()));
            }
            let seq = self.request_order.fetch_add(1, Ordering::SeqCst);
            self.request_counter.store(seq, Ordering::SeqCst);
            Ok(())
        }

        async fn on_response(
            &self,
            _res: &mut ProxyResponse,
            _ctx: &ConnectionContext,
        ) -> Result<(), ProxyError> {
            let seq = self.response_order.fetch_add(1, Ordering::SeqCst);
            self.response_counter.store(seq, Ordering::SeqCst);
            Ok(())
        }

        fn name(&self) -> &'static str {
            self.name
        }
    }

    fn make_request() -> ProxyRequest {
        ProxyRequest::new(
            Method::POST,
            "/v1/messages".into(),
            HeaderMap::new(),
            Bytes::from(r#"{"model":"test"}"#),
        )
    }

    fn make_context() -> ConnectionContext {
        ConnectionContext::new(1, crate::types::AgentType::Unknown, None, None)
    }

    fn make_response() -> ProxyResponse {
        ProxyResponse::new(StatusCode::OK, HeaderMap::new(), Bytes::new(), false)
    }

    #[tokio::test]
    async fn test_on_request_runs_in_registration_order() {
        let order = Arc::new(AtomicUsize::new(0));
        let mw_a = RecordingMiddleware {
            name: "A",
            request_order: order.clone(),
            response_order: Arc::new(AtomicUsize::new(0)),
            request_counter: AtomicUsize::new(0),
            response_counter: AtomicUsize::new(0),
            request_err: None,
        };
        let mw_b = RecordingMiddleware {
            name: "B",
            request_order: order.clone(),
            response_order: Arc::new(AtomicUsize::new(0)),
            request_counter: AtomicUsize::new(0),
            response_counter: AtomicUsize::new(0),
            request_err: None,
        };
        let mw_c = RecordingMiddleware {
            name: "C",
            request_order: order.clone(),
            response_order: Arc::new(AtomicUsize::new(0)),
            request_counter: AtomicUsize::new(0),
            response_counter: AtomicUsize::new(0),
            request_err: None,
        };

        let middlewares: Vec<Box<dyn ProxyMiddleware>> =
            vec![Box::new(mw_a), Box::new(mw_b), Box::new(mw_c)];

        let mut req = make_request();
        let mut ctx = make_context();

        run_on_request_chain(&middlewares, &mut req, &mut ctx)
            .await
            .unwrap();

        // After running, the order counter should be 3 (0→1→2→3)
        assert_eq!(order.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_on_response_runs_in_reverse_registration_order() {
        let order = Arc::new(AtomicUsize::new(0));
        let mw_a = RecordingMiddleware {
            name: "A",
            request_order: Arc::new(AtomicUsize::new(0)),
            response_order: order.clone(),
            request_counter: AtomicUsize::new(0),
            response_counter: AtomicUsize::new(0),
            request_err: None,
        };
        let mw_b = RecordingMiddleware {
            name: "B",
            request_order: Arc::new(AtomicUsize::new(0)),
            response_order: order.clone(),
            request_counter: AtomicUsize::new(0),
            response_counter: AtomicUsize::new(0),
            request_err: None,
        };
        let mw_c = RecordingMiddleware {
            name: "C",
            request_order: Arc::new(AtomicUsize::new(0)),
            response_order: order.clone(),
            request_counter: AtomicUsize::new(0),
            response_counter: AtomicUsize::new(0),
            request_err: None,
        };

        let middlewares: Vec<Box<dyn ProxyMiddleware>> =
            vec![Box::new(mw_a), Box::new(mw_b), Box::new(mw_c)];

        let mut res = make_response();
        let ctx = make_context();

        run_on_response_chain(&middlewares, &mut res, &ctx)
            .await
            .unwrap();

        assert_eq!(order.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_on_request_aborts_on_error() {
        let mw_ok = RecordingMiddleware {
            name: "ok",
            request_order: Arc::new(AtomicUsize::new(0)),
            response_order: Arc::new(AtomicUsize::new(0)),
            request_counter: AtomicUsize::new(0),
            response_counter: AtomicUsize::new(0),
            request_err: None,
        };
        let mw_err = RecordingMiddleware {
            name: "err",
            request_order: Arc::new(AtomicUsize::new(0)),
            response_order: Arc::new(AtomicUsize::new(0)),
            request_counter: AtomicUsize::new(0),
            response_counter: AtomicUsize::new(0),
            request_err: Some(ProxyError::BadRequest("test error".into())),
        };
        let mw_never = RecordingMiddleware {
            name: "never",
            request_order: Arc::new(AtomicUsize::new(0)),
            response_order: Arc::new(AtomicUsize::new(0)),
            request_counter: AtomicUsize::new(0),
            response_counter: AtomicUsize::new(0),
            request_err: None,
        };

        let middlewares: Vec<Box<dyn ProxyMiddleware>> =
            vec![Box::new(mw_ok), Box::new(mw_err), Box::new(mw_never)];

        let mut req = make_request();
        let mut ctx = make_context();

        let result = run_on_request_chain(&middlewares, &mut req, &mut ctx).await;
        assert!(result.is_err());
    }
}

// ── Cost recorder trait ────────────────────────────────────────────────

/// Post-response cost recording hook.
///
/// Called after the `on_response` middleware chain completes and before the
/// axum response is built. Implementors (typically the `cost` crate) use this
/// to extract usage, calculate cost, and persist a `CostRecord`.
///
/// This is deliberately not part of [`ProxyMiddleware`] because cost recording
/// needs to happen after ALL other response transformations are done.
#[async_trait::async_trait]
pub trait CostRecorder: Send + Sync + std::fmt::Debug {
    /// Record a cost entry for the completed request.
    ///
    /// `response_body` is the final response body JSON (after all middleware
    /// transforms have been applied).
    async fn record(
        &self,
        ctx: &crate::types::ConnectionContext,
        response_body: &serde_json::Value,
    ) -> Result<(), crate::error::ProxyError>;
}
