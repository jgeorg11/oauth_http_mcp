mod auth;
mod config;
mod handlers;
mod mcp;
mod models;
mod storage;
mod tools;

use std::{error::Error, sync::Arc};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

use crate::{
    auth::AuthState,
    config::{AuthConfig, DataConfig, McpConfig},
    handlers::router,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let mcp_config = McpConfig::from_env()?;
    let data_config = DataConfig::from_env()?;
    let auth = Arc::new(AuthState::new(AuthConfig::from_env()?).await?);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(data_config.max_connections)
        .connect(&data_config.database_url)
        .await?;
    let app = router(pool, auth.clone());

    let listener = TcpListener::bind((mcp_config.host.as_str(), mcp_config.port)).await?;
    tracing::info!(
        "MCP server listening on http://{}/mcp",
        listener.local_addr()?
    );

    let key_refresh = tokio::spawn(auth.refresh_keys_periodically());
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("Shutting down MCP server");
        })
        .await;
    key_refresh.abort();
    result?;

    Ok(())
}
