//! agent-proxy-rust server — proxy + admin API.

mod admin;
mod admin_auth;

use std::{path::PathBuf, sync::Arc};

use agent_proxy_rust_bridge::BridgeMiddleware;
use agent_proxy_rust_compress::CompressMiddleware;
use agent_proxy_rust_core::{AgentProxyBuilder, ProxyConfig, middleware::ModelAliasMiddleware};
use agent_proxy_rust_cost::CostMiddleware;
use agent_proxy_rust_model_router::ModelRouterMiddleware;
use agent_proxy_rust_storage::{SeedManager, Storage};
use agent_proxy_rust_storage_sqlite::SqliteStorage;
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // --db-path <PATH> or AGENT_PROXY_DB_PATH env
    let db_path = parse_db_path();
    tracing::info!(path = %db_path.display(), "opening database");

    if let Some(parent) = db_path.parent() {
        #[allow(clippy::disallowed_methods)]
        std::fs::create_dir_all(parent)?;
    }

    let db_storage = SqliteStorage::new(&db_path)?;
    db_storage.migrate().await?;
    match db_storage.seed_init().await {
        Ok(status) => tracing::info!(
            version = status.local_version,
            source = %status.source,
            "seed data initialized"
        ),
        Err(e) => {
            tracing::warn!(error = %e, "seed data initialization failed, continuing with DB schema only");
        }
    }

    let storage: Arc<dyn Storage> = Arc::new(db_storage.clone());
    let seed: Arc<dyn SeedManager> = Arc::new(db_storage);
    let model_router = ModelRouterMiddleware::from_storage(storage.clone()).await?;

    // Share the in-memory health map, API-key map, and channel list with the
    // admin API so that runtime changes (e.g. priority, enabled, API key)
    // take effect immediately without restarting the proxy.
    let health_map = Arc::clone(model_router.health_map());
    let api_key_map = Arc::clone(model_router.api_key_map());
    let channels_swap = model_router.channels_swap();

    let admin_key = admin_auth::resolve_admin_key();
    if admin_key.generated {
        tracing::warn!(
            "auto-generated admin key — set AGENT_PROXY_ADMIN_KEY to avoid this. \
             Key (first 8 chars): {}...",
            &admin_key.key[..8]
        );
    }
    let compress = CompressMiddleware::new();
    let compress_enabled = compress.enabled_flag();
    let admin = admin::admin_routes(
        storage.clone(),
        seed,
        Some(admin_key.key.clone()),
        health_map,
        api_key_map,
        compress_enabled,
        channels_swap,
    );

    let proxy_api_key = std::env::var("AGENT_PROXY_API_KEY")
        .ok()
        .map(|k| secrecy::SecretString::from(k.into_boxed_str()));
    let proxy_token = std::env::var("AGENT_PROXY_TOKEN")
        .ok()
        .map(|t| secrecy::SecretString::from(t.into_boxed_str()));

    let config = ProxyConfig {
        listen: "127.0.0.1:11837".parse()?,
        proxy_api_key,
        proxy_token,
        ..Default::default()
    };

    let cost_middleware = Arc::new(CostMiddleware::new(storage.clone(), "unknown".to_string()));

    // Build model alias map from DB (only enabled aliases)
    let aliases = storage.list_model_aliases().await.unwrap_or_default();
    let alias_map: std::collections::HashMap<String, String> = aliases
        .into_iter()
        .filter(|a| a.enabled)
        .map(|a| (a.alias_name, a.target_model))
        .collect();
    let model_alias = ModelAliasMiddleware::new(alias_map);

    let proxy = AgentProxyBuilder::default()
        .config(config)
        .cost_recorder(cost_middleware)
        .middleware(compress)
        .middleware(model_alias)
        .middleware(model_router)
        .middleware(BridgeMiddleware::new())
        .build()?
        .into_router()?;

    let app = admin.merge(proxy);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:11837").await?;
    tracing::info!("listening on 127.0.0.1:11837");

    axum::serve(listener, app).await?;
    Ok(())
}

fn parse_db_path() -> PathBuf {
    // 1. --db-path <PATH> CLI argument
    let args: Vec<String> = std::env::args().collect();
    #[allow(clippy::collapsible_if)]
    if let Some(pos) = args.iter().position(|a| a == "--db-path") {
        if let Some(p) = args.get(pos + 1) {
            return PathBuf::from(p);
        }
    }
    // 2. AGENT_PROXY_DB_PATH env var
    if let Ok(p) = std::env::var("AGENT_PROXY_DB_PATH") {
        return PathBuf::from(p);
    }
    // 3. Default: ~/.tokenfleet-ai/token-fleet-switch/agent-proxy.db
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".tokenfleet-ai")
        .join("token-fleet-switch")
        .join("agent-proxy.db")
}
