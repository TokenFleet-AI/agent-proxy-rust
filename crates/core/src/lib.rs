//! `agent-proxy-rust-core` — middleware trait, axum server engine, and upstream forwarding.
//!
//! This crate provides the foundation for the `agent-proxy-rust` project.
//! It defines the [`ProxyMiddleware`] trait (the central extension point),
//! core domain types, error handling, configuration, an authentication layer,
//! and the axum-based HTTP proxy engine.
//!
//! # Architecture
//!
//! ```text
//! Client → Auth Layer → Router → handle_proxy_request
//!                                     ├── on_request chain (registration order)
//!                                     ├── forward to upstream
//!                                     └── on_response chain (reverse order)
//! ```
//!
//! Middleware crates (`compress`, `bridge`, `model-router`, `cost`)
//! implement [`ProxyMiddleware`] and are composed via the builder.

#![forbid(unsafe_code)]
#![warn(missing_docs, missing_debug_implementations)]

pub mod auth;
pub mod compression;
pub mod config;
pub mod error;
pub mod extensions;
pub mod middleware;
pub mod report;
pub mod server;
pub mod testing;
pub mod types;

// Re-export key types for convenience
pub use compression::{CompressionStats, read_tokenless_stats};
pub use config::ProxyConfig;
pub use error::ProxyError;
pub use middleware::{CostRecorder, ProxyMiddleware};
pub use server::{AgentProxy, AgentProxyBuilder};
pub use types::{
    AgentType, ApiFormat, ChannelConfig, ConnectionContext, ProxyRequest, ProxyResponse,
    detect_api_format,
};
