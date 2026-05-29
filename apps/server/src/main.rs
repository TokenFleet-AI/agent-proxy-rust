//! agent-proxy-rust server — proxy + admin API.

mod admin;

use std::{path::PathBuf, sync::Arc};

use agent_proxy_rust_core::{AgentProxyBuilder, ProxyConfig};
use agent_proxy_rust_model_router::ModelRouterMiddleware;
use agent_proxy_rust_storage::Storage;
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

    let storage: Arc<dyn Storage> = Arc::new(db_storage);
    let model_router = ModelRouterMiddleware::from_storage(storage.clone()).await?;

    let admin = admin::admin_routes(storage.clone());

    let config = ProxyConfig {
        listen: "127.0.0.1:4000".parse()?,
        ..Default::default()
    };

    let proxy = AgentProxyBuilder::default()
        .config(config)
        .middleware(model_router)
        .build()?
        .into_router()?;

    let app = admin.merge(proxy);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:4000").await?;
    tracing::info!("listening on 127.0.0.1:4000");

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
    // 3. Default: same dir as binary
    let mut p = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    p.push("agent-proxy-rust.db");
    p
}
