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

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AssessProjects {
    /// Caller-generated idempotency key: 1–64 ASCII letters,
    /// digits, hyphens, or underscores.
    pub run_id: String,

    /// Between 1 and 20 distinct positive project IDs.
    /// Keep this full ordered list identical across calls for the same run.
    pub project_ids: Vec<i64>,

    /// For live chat updates, select one ID from project_ids per call. Report
    /// its returned outcome, then call the next ID with the same run_id and
    /// full project_ids. Omit or pass null to process the entire batch.
    pub project_id: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReadAssessmentRun {
    /// The run ID previously supplied to assess_projects.
    pub run_id: String,
}

fn assessment_error(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message)])
}

fn assessment_owner(
    context: &rmcp::service::RequestContext<rmcp::RoleServer>,
) -> Result<String, CallToolResult> {
    let principal = crate::mcp::principal_from_context(context)
        .map_err(|_| assessment_error("Authenticated identity is missing"))?;

    // Structured encoding avoids ambiguous concatenated identity keys.
    serde_json::to_string(&(principal.tenant_id, principal.object_id))
        .map_err(|_| assessment_error("Unable to identify run owner"))
}

async fn assessment_progress(
    context: &rmcp::service::RequestContext<rmcp::RoleServer>,
    done: f64,
    total: usize,
    message: String,
    chunk: Option<serde_json::Value>,
) -> Result<(), CallToolResult> {
    use rmcp::model::{NotificationMetaObject, ProgressNotificationParam};

    let Some(token) = context.meta.get_progress_token() else {
        return Ok(());
    };

    let mut notification = ProgressNotificationParam::new(token, done)
        .with_total(total as f64)
        .with_message(message);

    let supports_chunks = context.client_capabilities().is_some_and(|caps| {
        caps.extensions
            .is_some_and(|extensions| extensions.contains_key(crate::assessments::CHUNKS))
    });

    if supports_chunks && let Some(chunk) = chunk {
        let mut meta = NotificationMetaObject::new();
        meta.insert(crate::assessments::CHUNKS.into(), chunk);
        notification.meta = Some(meta);
    }

    tracing::debug!(progress = done, total, "Sending assessment progress");
    context
        .peer
        .notify_progress(notification)
        .await
        .map_err(|error| {
            tracing::warn!(%error, "Assessment progress delivery failed");
            assessment_error(
                "Unable to deliver progress; use \
                 read_assessment_run to recover saved results",
            )
        })
}

async fn assess_with_progress(
    analyst: &crate::assessments::Analyst,
    input: &crate::assessments::Evidence,
    args: &AssessProjects,
    index: usize,
    context: &rmcp::service::RequestContext<rmcp::RoleServer>,
) -> Result<Result<crate::assessments::Assessment, String>, CallToolResult> {
    // A bounded channel propagates backpressure. Dropping this future (on
    // cancellation or notification failure) also drops the HTTP inference.
    let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
    let inference = analyst.assess(&args.run_id, input, sender);
    tokio::pin!(inference);
    let mut draft = String::new();
    let mut last_update: Option<tokio::time::Instant> = None;

    loop {
        tokio::select! {
            biased;
            Some(delta) = receiver.recv() => {
                draft.push_str(&delta);
                if last_update.is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(100)) {
                    continue;
                }
                last_update = Some(tokio::time::Instant::now());
                // Generation has no known total token count. This increasing
                // fraction approaches the next unit without claiming it is done.
                let fraction = draft.len() as f64 / (draft.len() as f64 + 4096.0);
                let progress_index = if args.project_id.is_some() { 0 } else { index };
                let total = if args.project_id.is_some() { 1 } else { args.project_ids.len() };
                assessment_progress(
                    context,
                    progress_index as f64 + fraction,
                    total,
                    format!("Project {} ({}), draft assessment:\n{}",
                        input.project.id, input.project.name, draft),
                    Some(serde_json::json!({
                        "runId": args.run_id,
                        "sequence": index + 1,
                        "projectId": input.project.id,
                        "status": "draft",
                        "text": draft,
                    })),
                ).await?;
            }
            result = &mut inference => return Ok(result),
        }
    }
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

    #[tool(
        title = "Assess project health",
        description = "Assess project spending and overdue milestones \
        using one LLM request per project. For live chat updates, call sequentially \
        with project_id set to each selected ID and the same run_id and full ordered \
        project_ids. Each call returns after that project is saved; report its outcome \
        before calling the next ID. Keep waiting if a call yields or is still running; \
        do not end the turn early. Omitting project_id runs the full batch with one \
        final tool result. Reuse run_id and identical \
        inputs to recover completed assessments and retry unfinished \
        or failed units. Streams model drafts and saved assessment summaries \
        in standard MCP progress messages when the host supplies a progress \
        token. Drafts are provisional until validated and saved. Clients declaring \
        mcp.streaming/project-assessments-v1 also receive structured chunks. \
        Use a new run_id for fresh generation; cached results return immediately.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = true
        )
    )]
    pub async fn assess_projects(
        &self,
        Parameters(args): Parameters<AssessProjects>,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<Json<crate::assessments::RunResult>, CallToolResult> {
        use crate::assessments;

        let owner = assessment_owner(&context)?;

        let valid_run_id = !args.run_id.is_empty()
            && args.run_id.len() <= 64
            && args
                .run_id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_');

        let distinct_ids = args
            .project_ids
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len();

        if !valid_run_id
            || !(1..=20).contains(&args.project_ids.len())
            || args.project_ids.iter().any(|id| *id <= 0)
            || distinct_ids != args.project_ids.len()
        {
            return Err(assessment_error(
                "Provide a valid run_id and \
             1–20 distinct positive project IDs",
            ));
        }

        if args
            .project_id
            .is_some_and(|id| !args.project_ids.contains(&id))
        {
            return Err(assessment_error("project_id must be one of project_ids"));
        }

        let analyst = self
            .analyst
            .as_ref()
            .ok_or_else(|| assessment_error("The analyst endpoint is not configured"))?;

        let _permit = analyst
            .gate
            .try_acquire()
            .map_err(|_| assessment_error("An assessment run is already active; retry later"))?;

        tracing::info!(
            tool = "assess_projects",
            run_id = %args.run_id,
            project_count = args.project_ids.len(),
            selected_project_id = ?args.project_id,
            progress_requested = context.meta.get_progress_token().is_some(),
            "Assessment run started"
        );

        let work = async {
            let total = if args.project_id.is_some() {
                1
            } else {
                args.project_ids.len()
            };
            assessment_progress(
                &context,
                0.0,
                total,
                args.project_id.map_or_else(
                    || {
                        format!(
                            "Starting assessments for {} projects",
                            args.project_ids.len()
                        )
                    },
                    |id| format!("Starting assessment for project {id}"),
                ),
                None,
            )
            .await?;

            let inputs = analyst
                .snapshot(&self.pool, &owner, &args.run_id, &args.project_ids)
                .await
                .map_err(assessment_error)?;

            for (index, input) in inputs.iter().enumerate() {
                if args.project_id.is_some_and(|id| id != input.project.id) {
                    continue;
                }
                let saved = assessments::cached(&self.pool, &owner, &args.run_id, input.project.id)
                    .await
                    .map_err(assessment_error)?;
                let reused = saved.is_some();

                let outcome = if let Some(saved) = saved {
                    Ok(saved)
                } else {
                    // Real asynchronous HTTP inference.
                    let outcome =
                        assess_with_progress(analyst, input, &args, index, &context).await?;

                    // Commit before trying to deliver the chunk.
                    assessments::save(&self.pool, &owner, &args.run_id, input.project.id, &outcome)
                        .await
                        .map_err(assessment_error)?;

                    outcome
                };

                tracing::info!(
                    run_id = %args.run_id,
                    project_id = input.project.id,
                    cached = reused,
                    succeeded = outcome.is_ok(),
                    "Assessment unit completed"
                );

                let chunk = match &outcome {
                    Ok(value) => serde_json::json!({
                        "runId": args.run_id,
                        "sequence": index + 1,
                        "status": "completed",
                        "result": value,
                    }),
                    Err(error) => serde_json::json!({
                        "runId": args.run_id,
                        "sequence": index + 1,
                        "projectId": input.project.id,
                        "status": "failed",
                        "error": error,
                    }),
                };

                let message = match &outcome {
                    Ok(value) => format!(
                        "Completed project {} ({}) [{}/{}]: {}\nActions: {}",
                        input.project.id,
                        input.project.name,
                        index + 1,
                        inputs.len(),
                        value.narrative.summary,
                        value.narrative.actions.join("; "),
                    ),
                    Err(error) => format!(
                        "Failed project {} ({}) [{}/{}]: {}",
                        input.project.id,
                        input.project.name,
                        index + 1,
                        inputs.len(),
                        error,
                    ),
                };
                assessment_progress(
                    &context,
                    if args.project_id.is_some() {
                        1.0
                    } else {
                        (index + 1) as f64
                    },
                    total,
                    message,
                    Some(chunk),
                )
                .await?;
            }

            let result = assessments::read(&self.pool, &owner, &args.run_id)
                .await
                .map_err(assessment_error)?;
            tracing::info!(
                tool = "assess_projects",
                run_id = %args.run_id,
                selected_project_id = ?args.project_id,
                result_count = result.completed.len(),
                failed_count = result.failed.len(),
                pending_count = result.pending_project_ids.len(),
                "MCP tool completed"
            );
            Ok(Json(result))
        };

        tokio::select! {
            biased;

            _ = context.ct.cancelled() => {
                Err(assessment_error(
                    "Request cancelled; completed units remain saved",
                ))
            }

            result = tokio::time::timeout(
                std::time::Duration::from_secs(180),
                work,
            ) => {
                result.map_err(|_| {
                    assessment_error(
                        "Run deadline exceeded; read or retry \
                         the same run_id",
                    )
                })?
            }
        }
    }

    #[tool(
        title = "Read assessment run",
        description = "Read your saved completed, failed, and pending \
        project assessments after a run or connection ends.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub async fn read_assessment_run(
        &self,
        Parameters(args): Parameters<ReadAssessmentRun>,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<Json<crate::assessments::RunResult>, CallToolResult> {
        let owner = assessment_owner(&context)?;

        crate::assessments::read(&self.pool, &owner, &args.run_id)
            .await
            .map(Json)
            .map_err(assessment_error)
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
        assert_eq!(tools.len(), 8);
        for name in ["assess_projects", "read_assessment_run"] {
            assert!(tools.iter().any(|tool| tool.name == name));
        }
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
