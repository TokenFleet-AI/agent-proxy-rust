//! agent-proxy CLI — AI Agent Protocol Proxy
//!
//! Entry point for the `agent-proxy` binary. Parses CLI args,
//! loads configuration, validates it, and starts the proxy server
//! or runs channel management commands.

mod config;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

/// AI Agent Protocol Proxy — route, compress, bridge, and track costs.
#[derive(Parser, Debug)]
#[command(name = "agent-proxy", version, about)]
pub struct Cli {
    /// Top-level command to execute.
    #[command(subcommand)]
    pub command: Command,
}

/// Top-level subcommands for the agent-proxy CLI.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Start the proxy server.
    Serve(config::ServeArgs),

    /// Manage proxy channels.
    #[command(subcommand)]
    Channel(ChannelCommand),
}

/// Subcommands for channel management.
#[derive(Subcommand, Debug)]
pub enum ChannelCommand {
    /// List all configured channels.
    List,

    /// Set or update the API key for a channel.
    SetKey {
        /// Channel ID.
        id: String,

        /// API key value.
        #[arg(long = "api-key")]
        api_key: String,
    },

    /// Add a new channel.
    Add {
        /// Human-readable channel name.
        name: String,

        /// Upstream URL.
        #[arg(long = "url")]
        url: String,

        /// Protocol: `anthropic_messages`, `openai_chat`, `openai_responses`.
        #[arg(long = "protocol")]
        protocol: String,

        /// API key for authenticating with this channel.
        #[arg(long = "api-key")]
        api_key: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    match cli.command {
        Command::Serve(args) => {
            let cfg = config::load_config(&args).context("failed to load configuration")?;
            tracing::info!(?cfg.listen, ?cfg.log_format, "agent-proxy starting");
            // TODO: start server with config — blocked on Core Agent completing
            // ProxyMiddleware + axum engine. When ready, this will launch the
            // axum server on config.listen and await shutdown signal.
            tracing::info!("server would start here (core axum engine not yet ready)");
            tracing::info!(
                "loaded config: listen={}, data_dir={}, max_body_size={}, upstream_timeout={}s",
                cfg.listen,
                cfg.data_dir.display(),
                cfg.max_body_size,
                cfg.upstream_timeout
            );
            tracing::info!("agent-proxy shutting down");
        }
        Command::Channel(cmd) => match cmd {
            ChannelCommand::List => {
                tracing::info!("channel list requested");
                println!("[]"); // placeholder
            }
            ChannelCommand::SetKey { id, api_key } => {
                tracing::info!(%id, "set API key for channel");
                println!("ok");
                drop(api_key);
            }
            ChannelCommand::Add {
                name,
                url,
                protocol,
                api_key,
            } => {
                tracing::info!(%name, %url, %protocol, "add channel");
                println!("ok");
                drop(api_key);
            }
        },
    }

    Ok(())
}
