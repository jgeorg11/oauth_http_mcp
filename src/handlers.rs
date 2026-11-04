use std::sync::Arc;

use axum::{Router, middleware, routing::get};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use sqlx::SqlitePool;

use crate::{
    auth::{AuthState, metadata, require_auth},
    mcp::McpServer,
};

pub fn router(pool: SqlitePool, auth: Arc<AuthState>) -> Router {
    mcp_router(pool, auth.config.resource.host_str())
        // Apply authentication before adding the public discovery route.
        .route_layer(middleware::from_fn_with_state(auth.clone(), require_auth))
        .route("/.well-known/oauth-protected-resource/mcp", get(metadata))
        .with_state(auth)
}

fn mcp_router<S: Clone + Send + Sync + 'static>(
    pool: SqlitePool,
    resource_host: Option<&str>,
) -> Router<S> {
    let server = McpServer::new(pool);
    let mut transport = StreamableHttpServerConfig::default();
    if let Some(host) = resource_host {
        transport.allowed_hosts.push(host.to_owned());
    }
    let mcp = StreamableHttpService::new(
        // Cloning the pool shares its connections; it does not create a new pool.
        move || Ok(server.clone()),
        LocalSessionManager::default().into(),
        transport,
    );

    Router::new().nest_service("/mcp", mcp)
}

#[cfg(test)]
mod tests;
