use rmcp::{
    handler::server::wrapper::{Json, Parameters},
    model::{CallToolResult, ContentBlock},
    schemars, tool, tool_router,
};
use serde::{Deserialize, Serialize};

use crate::{mcp::McpServer, models::Project, storage};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetProjectById {
    /// Numeric ID returned by a project lookup, search, list, or creation.
    pub project_id: i64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchProjects {
    /// Name substring to match. % matches any sequence and _ matches one character.
    /// An empty string matches all projects. Owners and statuses are not searched.
    pub query: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateProject {
    /// Name of the new project.
    pub name: String,
    /// Owner of the new project.
    pub owner: String,
    /// Status text, such as Planning or Active. There is no fixed status enum.
    pub status: String,
    /// Budget amount. No currency or numeric range is defined by the database.
    pub budget: f64,
    /// Integer risk score. No scale, thresholds, or numeric range are defined.
    pub risk_score: i64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UpdateProject {
    /// Numeric ID of the existing project to update.
    pub project_id: i64,
    /// Replacement name. Omit or pass null to preserve the current value.
    pub name: Option<String>,
    /// Replacement owner. Omit or pass null to preserve the current value.
    pub owner: Option<String>,
    /// Replacement status text; no fixed enum. Omit or pass null to preserve it.
    pub status: Option<String>,
    /// Replacement budget; no defined currency or range. Omit or pass null to preserve it.
    pub budget: Option<f64>,
    /// Replacement integer risk score; no defined scale or range. Omit or pass null to preserve it.
    pub risk_score: Option<i64>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ProjectsResult {
    /// Matching project records ordered by ID. An empty array means no matches.
    pub projects: Vec<Project>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ProjectResult {
    /// The retrieved, created, or updated project record.
    pub project: Project,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct DeleteProjectResult {
    /// True when the project was deleted. A missing ID returns a tool error instead.
    pub deleted: bool,
}

#[tool_router(vis = "pub(crate)")]
impl McpServer {
    #[tool(
        title = "List projects",
        description = "List all project records in the connected database, ordered by ID. Use for project overviews or to filter by owner or status locally. Returns {projects: [...]}, with an empty array when there are no projects. No pagination. Seed data is fictional.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub async fn get_projects(&self) -> Result<Json<ProjectsResult>, CallToolResult> {
        match storage::get_projects(&self.pool).await {
            Ok(projects) => {
                tracing::info!(
                    tool = "get_projects",
                    result_count = projects.len(),
                    "MCP tool completed"
                );
                Ok(Json(ProjectsResult { projects }))
            }
            Err(error) => Err(query_error("get_projects", error)),
        }
    }

    #[tool(
        title = "Get project",
        description = "Retrieve one project from the connected database using its numeric ID. Use an ID returned by the server; search by name when the ID is unknown. Returns {project: {...}}. A missing ID is a tool error.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub async fn get_project_by_id(
        &self,
        Parameters(args): Parameters<GetProjectById>,
    ) -> Result<Json<ProjectResult>, CallToolResult> {
        match storage::get_project_by_id(&self.pool, args.project_id).await {
            Ok(Some(project)) => {
                tracing::info!(
                    tool = "get_project_by_id",
                    result_count = 1,
                    "MCP tool completed"
                );
                Ok(Json(ProjectResult { project }))
            }
            Ok(None) => {
                tracing::warn!(
                    tool = "get_project_by_id",
                    project_id = args.project_id,
                    "MCP tool: project not found"
                );
                Err(CallToolResult::error(vec![ContentBlock::text(format!(
                    "Project {} was not found",
                    args.project_id
                ))]))
            }
            Err(error) => Err(query_error("get_project_by_id", error)),
        }
    }

    #[tool(
        title = "Search projects by name",
        description = "Find projects by name substring using SQLite LIKE, with % and _ wildcards supported. Searches names only. Returns all matches ordered by ID as {projects: [...]}, with no pagination; an empty array means no matches. If multiple projects match an intended update or deletion, resolve which ID the user means first.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub async fn get_project_by_query(
        &self,
        Parameters(args): Parameters<SearchProjects>,
    ) -> Result<Json<ProjectsResult>, CallToolResult> {
        match storage::get_project_by_query(&self.pool, &args.query).await {
            Ok(projects) => {
                tracing::info!(
                    tool = "get_project_by_query",
                    result_count = projects.len(),
                    "MCP tool completed"
                );
                Ok(Json(ProjectsResult { projects }))
            }
            Err(error) => Err(query_error("get_project_by_query", error)),
        }
    }

    #[tool(
        title = "Create project",
        description = "Create a project in the connected database when the user requests creation and supplies its name, owner, status, budget, and risk score. The database assigns its ID and creation timestamp. Returns {project: {...}}. Each call inserts a new record, so repeating the call creates another project.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    pub async fn create_project(
        &self,
        Parameters(args): Parameters<CreateProject>,
    ) -> Result<Json<ProjectResult>, CallToolResult> {
        match storage::create_project(
            &self.pool,
            &args.name,
            &args.owner,
            &args.status,
            args.budget,
            args.risk_score,
        )
        .await
        {
            Ok(project) => {
                tracing::info!(
                    tool = "create_project",
                    result_count = 1,
                    "MCP tool completed"
                );
                Ok(Json(ProjectResult { project }))
            }
            Err(error) => Err(query_error("create_project", error)),
        }
    }

    #[tool(
        title = "Update project",
        description = "Update an existing project's name, owner, status, budget, or risk score using its numeric ID when the user requests a change. Omitted or null fields keep their current values; fields cannot be cleared to null. Returns the updated record as {project: {...}}. A missing ID is a tool error. Resolve ambiguous project names before updating.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    pub async fn update_project(
        &self,
        Parameters(args): Parameters<UpdateProject>,
    ) -> Result<Json<ProjectResult>, CallToolResult> {
        match storage::update_project(
            &self.pool,
            args.project_id,
            args.name.as_deref(),
            args.budget,
            args.owner.as_deref(),
            args.risk_score,
            args.status.as_deref(),
        )
        .await
        {
            Ok(Some(project)) => {
                tracing::info!(
                    tool = "update_project",
                    result_count = 1,
                    "MCP tool completed"
                );
                Ok(Json(ProjectResult { project }))
            }

            Ok(None) => {
                tracing::warn!(
                    tool = "update_project",
                    project_id = args.project_id,
                    "MCP tool: project not found"
                );
                Err(CallToolResult::error(vec![ContentBlock::text(format!(
                    "Project {} was not found",
                    args.project_id
                ))]))
            }
            Err(error) => Err(query_error("update_project", error)),
        }
    }

    #[tool(
        title = "Delete project",
        description = "Permanently delete a project from the connected database using its numeric ID when the user requests deletion. Returns {deleted: true}; a missing ID is a tool error. Resolve ambiguous project names before deleting. There is no undo tool.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    pub async fn delete_project(
        &self,
        Parameters(args): Parameters<GetProjectById>,
    ) -> Result<Json<DeleteProjectResult>, CallToolResult> {
        match storage::delete_project(&self.pool, args.project_id).await {
            Ok(true) => {
                tracing::info!(
                    tool = "delete_project",
                    result_count = 1,
                    "MCP tool completed"
                );
                Ok(Json(DeleteProjectResult { deleted: true }))
            }
            Ok(false) => {
                tracing::warn!(
                    tool = "delete_project",
                    project_id = args.project_id,
                    "MCP tool: project not found"
                );
                Err(CallToolResult::error(vec![ContentBlock::text(format!(
                    "Project {} was not found",
                    args.project_id
                ))]))
            }
            Err(error) => Err(query_error("delete_project", error)),
        }
    }
}

fn query_error(tool: &'static str, error: sqlx::Error) -> CallToolResult {
    tracing::error!(tool, %error, "Project operation failed");
    CallToolResult::error(vec![ContentBlock::text(
        "Unable to complete project operation",
    )])
}

#[cfg(test)]
mod tests {
    use rmcp::{handler::server::tool::IntoCallToolResult, model::CallToolResponse};
    use serde_json::json;
    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;

    fn wire_result(value: impl IntoCallToolResult) -> CallToolResult {
        match value.into_call_tool_result().unwrap() {
            CallToolResponse::Complete(result) => result,
            _ => panic!("Expected a completed tool call"),
        }
    }

    #[tokio::test]
    async fn project_tools_complete_lifecycle_and_preserve_omitted_fields() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(include_str!("../data/seed.sql"))
            .execute(&pool)
            .await
            .unwrap();
        let server = McpServer::new(pool);

        let listed = wire_result(server.get_projects().await);
        assert_eq!(
            listed.structured_content.unwrap()["projects"]
                .as_array()
                .unwrap()
                .len(),
            5
        );

        let created = wire_result(
            server
                .create_project(Parameters(CreateProject {
                    name: "MCP contract test".into(),
                    owner: "Test owner".into(),
                    status: "Planning".into(),
                    budget: 1250.5,
                    risk_score: 4,
                }))
                .await,
        );
        assert_ne!(created.is_error, Some(true));
        let mut expected = created.structured_content.unwrap()["project"].clone();
        let project_id = expected["id"].as_i64().unwrap();
        assert!(project_id > 5);
        assert!(expected["created_at"].as_str().is_some());

        let updated = wire_result(
            server
                .update_project(Parameters(
                    serde_json::from_value(json!({
                        "project_id": project_id,
                        "name": "MCP contract test renamed"
                    }))
                    .unwrap(),
                ))
                .await,
        );
        expected["name"] = json!("MCP contract test renamed");
        assert_eq!(updated.structured_content.unwrap()["project"], expected);

        let updated = wire_result(
            server
                .update_project(Parameters(
                    serde_json::from_value(json!({
                        "project_id": project_id,
                        "name": null,
                        "owner": "Updated owner",
                        "status": "Active",
                        "budget": 0.0,
                        "risk_score": 0
                    }))
                    .unwrap(),
                ))
                .await,
        );
        expected["owner"] = json!("Updated owner");
        expected["status"] = json!("Active");
        expected["budget"] = json!(0.0);
        expected["risk_score"] = json!(0);
        assert_eq!(updated.structured_content.unwrap()["project"], expected);

        let fetched = wire_result(
            server
                .get_project_by_id(Parameters(GetProjectById { project_id }))
                .await,
        );
        assert_eq!(fetched.structured_content.unwrap()["project"], expected);
        let searched = wire_result(
            server
                .get_project_by_query(Parameters(SearchProjects {
                    query: "MCP contract test".into(),
                }))
                .await,
        );
        assert_eq!(
            searched.structured_content,
            Some(json!({"projects": [expected]}))
        );

        let deleted = wire_result(
            server
                .delete_project(Parameters(GetProjectById { project_id }))
                .await,
        );
        assert_eq!(deleted.structured_content, Some(json!({"deleted": true})));
        let searched = wire_result(
            server
                .get_project_by_query(Parameters(SearchProjects {
                    query: "MCP contract test".into(),
                }))
                .await,
        );
        assert_ne!(searched.is_error, Some(true));
        assert_eq!(searched.structured_content, Some(json!({"projects": []})));

        let missing_read = wire_result(
            server
                .get_project_by_id(Parameters(GetProjectById { project_id }))
                .await,
        );
        let missing_update = wire_result(
            server
                .update_project(Parameters(
                    serde_json::from_value(json!({"project_id": project_id, "name": "Missing"}))
                        .unwrap(),
                ))
                .await,
        );
        let missing_delete = wire_result(
            server
                .delete_project(Parameters(GetProjectById { project_id }))
                .await,
        );
        for error in [missing_read, missing_update, missing_delete] {
            assert_eq!(error.is_error, Some(true));
            assert!(error.structured_content.is_none());
        }
    }

    #[test]
    fn discovered_tools_describe_inputs_and_declare_output_schemas() {
        let tools = McpServer::tool_router().list_all();
        assert_eq!(tools.len(), 6);
        for tool in tools {
            assert!(tool.title.is_some(), "{} needs a title", tool.name);
            assert!(
                tool.description.is_some(),
                "{} needs a description",
                tool.name
            );
            assert!(
                tool.output_schema.is_some(),
                "{} needs an output schema",
                tool.name
            );
            for (name, parameter) in tool.input_schema["properties"].as_object().unwrap() {
                assert!(
                    parameter["description"]
                        .as_str()
                        .is_some_and(|text| !text.is_empty()),
                    "{}.{name} needs a parameter description",
                    tool.name
                );
            }
        }
        assert_eq!(
            McpServer::update_project_tool_attr().input_schema["required"],
            json!(["project_id"])
        );
        assert_eq!(
            McpServer::create_project_tool_attr().input_schema["required"]
                .as_array()
                .unwrap()
                .len(),
            5
        );
    }
}
