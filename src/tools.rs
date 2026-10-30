use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock},
    schemars, tool, tool_router,
};
use serde::Deserialize;
use serde_json::json;

use crate::{mcp::McpServer, storage};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetProjectById {
    pub project_id: i64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchProjects {
    pub query: String,
}

#[tool_router(vis = "pub(crate)")]
impl McpServer {
    #[tool(
        description = "List all projects ordered by ID",
        annotations(read_only_hint = true)
    )]
    pub async fn get_projects(&self) -> CallToolResult {
        match storage::get_projects(&self.pool).await {
            Ok(projects) => {
                tracing::info!(
                    tool = "get_projects",
                    result_count = projects.len(),
                    "MCP tool completed"
                );
                CallToolResult::structured(json!({ "projects": projects }))
            }
            Err(error) => query_error("get_projects", error),
        }
    }

    #[tool(
        description = "Get one project by its numeric ID",
        annotations(read_only_hint = true)
    )]
    pub async fn get_project_by_id(
        &self,
        Parameters(args): Parameters<GetProjectById>,
    ) -> CallToolResult {
        match storage::get_project_by_id(&self.pool, args.project_id).await {
            Ok(Some(project)) => {
                tracing::info!(
                    tool = "get_project_by_id",
                    result_count = 1,
                    "MCP tool completed"
                );
                CallToolResult::structured(json!({ "project": project }))
            }
            Ok(None) => {
                tracing::warn!(
                    tool = "get_project_by_id",
                    project_id = args.project_id,
                    "MCP tool: project not found"
                );
                CallToolResult::error(vec![ContentBlock::text(format!(
                    "Project {} was not found",
                    args.project_id
                ))])
            }
            Err(error) => query_error("get_project_by_id", error),
        }
    }

    #[tool(
        description = "Search project names using SQLite LIKE (supports % and _ wildcards)",
        annotations(read_only_hint = true)
    )]
    pub async fn get_project_by_query(
        &self,
        Parameters(args): Parameters<SearchProjects>,
    ) -> CallToolResult {
        match storage::get_project_by_query(&self.pool, &args.query).await {
            Ok(projects) => {
                tracing::info!(
                    tool = "get_project_by_query",
                    result_count = projects.len(),
                    "MCP tool completed"
                );
                CallToolResult::structured(json!({ "projects": projects }))
            }
            Err(error) => query_error("get_project_by_query", error),
        }
    }
}

fn query_error(tool: &'static str, error: sqlx::Error) -> CallToolResult {
    tracing::error!(tool, %error, "Project query failed");
    CallToolResult::error(vec![ContentBlock::text("Unable to query projects")])
}
