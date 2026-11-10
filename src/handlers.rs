use std::sync::Arc;

use axum::{Router, middleware, routing::get};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use sqlx::SqlitePool;

use crate::{
    assessments::Analyst,
    auth::{AuthState, metadata, require_auth},
    mcp::McpServer,
};

pub fn router(
    pool: SqlitePool,
    auth: Arc<AuthState>,
    analyst: Arc<Analyst>,
    transport: StreamableHttpServerConfig,
) -> Router {
    mcp_router(pool, auth.config.resource.host_str(), analyst, transport)
        // Apply authentication before adding the public discovery route.
        .route_layer(middleware::from_fn_with_state(auth.clone(), require_auth))
        .route("/.well-known/oauth-protected-resource/mcp", get(metadata))
        .with_state(auth)
}

fn mcp_router<S: Clone + Send + Sync + 'static>(
    pool: SqlitePool,
    resource_host: Option<&str>,
    analyst: Arc<Analyst>,
    mut transport: StreamableHttpServerConfig,
) -> Router<S> {
    let server = McpServer::new(pool).with_analyst(analyst);

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
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use axum::{Json, body::Body, extract::Request, middleware::Next, routing::post};
    use serde_json::{Value, json};
    use tokio::{net::TcpListener, sync::Notify};
    use tokio_stream::wrappers::ReceiverStream;

    use super::*;
    use crate::auth::Principal;

    async fn serve(app: Router) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{address}")
    }

    // SSE may be split at any byte. Ignore keepalives and read one JSON-RPC
    // message, preserving any subsequent messages already received.
    async fn event(response: &mut reqwest::Response, pending: &mut Vec<u8>) -> Value {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Some(end) = pending.windows(2).position(|b| b == b"\n\n") {
                    let frame: Vec<u8> = pending.drain(..end + 2).collect();
                    let text = std::str::from_utf8(&frame).unwrap();
                    if let Some(data) = text
                        .lines()
                        .find_map(|l| l.strip_prefix("data:"))
                        .filter(|s| !s.trim().is_empty())
                    {
                        return serde_json::from_str(data.trim()).unwrap();
                    }
                } else {
                    let bytes = response
                        .chunk()
                        .await
                        .unwrap()
                        .expect("SSE ended before a result");
                    pending.extend_from_slice(&bytes);
                }
            }
        })
        .await
        .expect("Timed out waiting for an MCP event")
    }

    async fn result(mut response: reqwest::Response) -> Value {
        assert!(response.status().is_success(), "{response:?}");
        if response.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("application/json")
        {
            return response.json().await.unwrap();
        }
        let mut pending = Vec::new();
        loop {
            let message = event(&mut response, &mut pending).await;
            if message.get("id").is_some() {
                return message;
            }
        }
    }

    #[tokio::test]
    async fn streams_before_model_completion_without_custom_client_capabilities() {
        let release_model = Arc::new(Notify::new());
        let model_finished = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(AtomicUsize::new(0));
        let model = serve(Router::new().route("/api/chat", post({
            let release = release_model.clone();
            let finished = model_finished.clone();
            let calls = calls.clone();
            move |Json(request): Json<Value>| {
                let release = release.clone();
                let finished = finished.clone();
                let ordinal = calls.fetch_add(1, Ordering::SeqCst);
                async move {
                    assert_eq!(request["stream"], true);
                    assert!(request["format"]["properties"]["summary"].is_object());
                    let input: Value = serde_json::from_str(request["messages"][1]["content"].as_str().unwrap()).unwrap();
                    let project_id = input["evidence"]["project"]["id"].as_i64().unwrap();
                    let (sender, receiver) = tokio::sync::mpsc::channel::<Result<String, std::io::Error>>(1);
                    tokio::spawn(async move {
                        let prefix = json!({"message": {"content": "{\"summary\":\"Spending needs "}, "done": false});
                        sender.send(Ok(format!("{prefix}\n"))).await.unwrap();
                        if ordinal == 0 {
                            // Only the test can release generation after it
                            // receives the draft through the MCP transport.
                            release.notified().await;
                        }
                        let evidence_id = if ordinal == 3 { "project:999".into() } else { format!("project:{project_id}") };
                        let suffix = format!("review\",\"evidence_ids\":[\"{evidence_id}\"],\"actions\":[\"Review costs\"]}}");
                        let terminal = json!({
                            "message": {"content": suffix}, "done": true,
                            "done_reason": if ordinal == 5 { "length" } else { "stop" },
                            "prompt_eval_count": 42, "eval_count": 12,
                        });
                        finished.store(true, Ordering::SeqCst);
                        // The last record intentionally has no trailing newline.
                        sender.send(Ok(terminal.to_string())).await.unwrap();
                    });
                    ([("content-type", "application/x-ndjson")], Body::from_stream(ReceiverStream::new(receiver)))
                }
            }
        }))).await;

        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for script in [
            include_str!("../data/seed.sql"),
            include_str!("../data/authorization.sql"),
            include_str!("../data/assessments.sql"),
            include_str!("../data/seed_assessments.sql"),
            "INSERT INTO user_roles VALUES ('test-tenant', 'test-user', 'worker');
             INSERT INTO role_tools VALUES ('worker', 'assess_projects'), ('worker', 'read_assessment_run');",
        ] {
            sqlx::raw_sql(script).execute(&pool).await.unwrap();
        }
        let analyst =
            Arc::new(Analyst::new(format!("{model}/api/chat"), "test-model".into(), None).unwrap());
        // Exercise the real MCP transport and permission checks. Only token
        // validation is replaced by a fixed authenticated test principal.
        let app = mcp_router(
            pool.clone(),
            Some("127.0.0.1"),
            analyst,
            StreamableHttpServerConfig::default(),
        )
        .layer(middleware::from_fn(
            |mut request: Request, next: Next| async move {
                request.extensions_mut().insert(Principal {
                    tenant_id: "test-tenant".into(),
                    object_id: "test-user".into(),
                });
                next.run(request).await
            },
        ));
        let url = format!("{}/mcp", serve(app).await);
        let http = reqwest::Client::new();
        let post = || {
            http.post(&url)
                .header("Accept", "application/json, text/event-stream")
        };

        // A normal desktop-style initialize handshake with no extension.
        let initialized = post()
            .json(&json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {
                    "protocolVersion": "2025-03-26", "capabilities": {},
                    "clientInfo": {"name": "ordinary-client", "version": "1.0"},
                },
            }))
            .send()
            .await
            .unwrap();
        let session = initialized.headers()["mcp-session-id"]
            .to_str()
            .unwrap()
            .to_owned();
        let info = result(initialized).await;
        assert_eq!(info["result"]["protocolVersion"], "2025-03-26");
        assert!(
            info["result"]["capabilities"]["extensions"][crate::assessments::CHUNKS].is_object()
        );
        let session_post = || {
            post()
                .header("Mcp-Session-Id", &session)
                .header("MCP-Protocol-Version", "2025-03-26")
        };
        assert!(
            session_post()
                .json(&json!({"jsonrpc":"2.0", "method":"notifications/initialized"}))
                .send()
                .await
                .unwrap()
                .status()
                .is_success()
        );

        let mut stream = session_post().json(&json!({
            "jsonrpc":"2.0", "id":2, "method":"tools/call",
            "params": {"name":"assess_projects", "arguments":{"run_id":"desktop-demo", "project_ids":[1]},
                "_meta":{"progressToken":"desktop-progress"}},
        })).send().await.unwrap();
        assert!(stream.status().is_success());
        let mut pending = Vec::new();
        let mut previous = -1.0;
        let completed = loop {
            let message = event(&mut stream, &mut pending).await;
            if message.get("id").is_some() {
                break message;
            }
            assert_eq!(message["method"], "notifications/progress");
            let progress = &message["params"];
            assert_eq!(progress["progressToken"], "desktop-progress");
            assert!(progress["_meta"][crate::assessments::CHUNKS].is_null());
            let value = progress["progress"].as_f64().unwrap();
            assert!(value > previous && value <= 1.0);
            previous = value;
            let text = progress["message"].as_str().unwrap();
            if text.contains("draft assessment") {
                assert!(!model_finished.load(Ordering::SeqCst));
                assert!(text.contains("Spending needs"));
                let saved: i64 = sqlx::query_scalar("SELECT count(*) FROM assessment_units")
                    .fetch_one(&pool)
                    .await
                    .unwrap();
                assert_eq!(
                    saved, 0,
                    "Drafts must not be persisted as completed assessments"
                );
                release_model.notify_one();
            }
            if text.starts_with("Completed project") {
                assert!(text.contains("Spending needs review"));
                let saved: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM assessment_units WHERE result_json IS NOT NULL",
                )
                .fetch_one(&pool)
                .await
                .unwrap();
                assert_eq!(saved, 1, "Completion must be saved before it is announced");
            }
        };
        assert_eq!(previous, 1.0);
        assert_eq!(
            completed["result"]["structuredContent"]["completed"][0]["input_tokens"],
            42
        );
        assert_eq!(
            completed["result"]["structuredContent"]["failed"],
            json!([])
        );

        // Recovery works with the same initialized client; cached work never
        // starts a second model request.
        let read = result(
            session_post()
                .json(&json!({
                    "jsonrpc":"2.0", "id":3, "method":"tools/call",
                    "params":{"name":"read_assessment_run", "arguments":{"run_id":"desktop-demo"}},
                }))
                .send()
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(
            read["result"]["structuredContent"],
            completed["result"]["structuredContent"]
        );
        let cached = result(session_post().json(&json!({
            "jsonrpc":"2.0", "id":4, "method":"tools/call",
            "params":{"name":"assess_projects", "arguments":{"run_id":"desktop-demo", "project_ids":[1]}},
        })).send().await.unwrap()).await;
        assert_eq!(
            cached["result"]["structuredContent"],
            read["result"]["structuredContent"]
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // No progress token: inference still streams upstream, but only the
        // final tool result is delivered downstream.
        let mut no_progress = session_post().json(&json!({
            "jsonrpc":"2.0", "id":5, "method":"tools/call",
            "params":{"name":"assess_projects", "arguments":{"run_id":"without-progress", "project_ids":[1]}},
        })).send().await.unwrap();
        let only = event(&mut no_progress, &mut Vec::new()).await;
        assert_eq!(only["id"], 5);
        assert_eq!(
            only["result"]["structuredContent"]["completed"]
                .as_array()
                .unwrap()
                .len(),
            1
        );

        // Newer clients can opt in to structured chunks under the generic key.
        let mut extended = post().header("MCP-Protocol-Version", "2026-07-28")
            .header("Mcp-Method", "tools/call").header("Mcp-Name", "assess_projects")
            .json(&json!({
                "jsonrpc":"2.0", "id":6, "method":"tools/call",
                "params":{"name":"assess_projects", "arguments":{"run_id":"extended", "project_ids":[1]},
                    "_meta":{
                        "progressToken":"extended-progress",
                        "io.modelcontextprotocol/protocolVersion":"2026-07-28",
                        "io.modelcontextprotocol/clientCapabilities":{"extensions":{crate::assessments::CHUNKS:{}}},
                    }},
            })).send().await.unwrap();
        let mut pending = Vec::new();
        let mut statuses = Vec::new();
        loop {
            let message = event(&mut extended, &mut pending).await;
            if message.get("id").is_some() {
                break;
            }
            if let Some(status) =
                message["params"]["_meta"][crate::assessments::CHUNKS]["status"].as_str()
            {
                statuses.push(status.to_owned());
            }
        }
        assert_eq!(statuses, ["draft", "completed"]);

        // Even after emitting a draft, invalid final evidence cannot become a
        // completed assessment.
        let invalid = result(session_post().json(&json!({
            "jsonrpc":"2.0", "id":7, "method":"tools/call",
            "params":{"name":"assess_projects", "arguments":{"run_id":"invalid-final", "project_ids":[1]}},
        })).send().await.unwrap()).await;
        assert_eq!(
            invalid["result"]["structuredContent"]["completed"],
            json!([])
        );
        assert_eq!(
            invalid["result"]["structuredContent"]["failed"][0]["error"],
            "LLM output failed validation"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 4);

        // A client that ignores progress gets a real result after each selected
        // project, with the rest of the same run still pending. No background
        // inference or second user prompt is required to obtain this result.
        let first = result(
            session_post()
                .json(&json!({
                    "jsonrpc":"2.0", "id":8, "method":"tools/call",
                    "params":{"name":"assess_projects", "arguments":{
                        "run_id":"incremental", "project_ids":[1,2,3], "project_id":1,
                    }},
                }))
                .send()
                .await
                .unwrap(),
        )
        .await;
        let first = &first["result"]["structuredContent"];
        assert_eq!(first["completed"].as_array().unwrap().len(), 1);
        assert_eq!(first["completed"][0]["project_id"], 1);
        assert_eq!(first["failed"], json!([]));
        assert_eq!(first["pending_project_ids"], json!([2, 3]));
        assert_eq!(calls.load(Ordering::SeqCst), 5);

        // Keep the entire snapshot across calls, even if live evidence changes.
        sqlx::query("UPDATE project_spend SET actual_minor = 0 WHERE project_id = 3")
            .execute(&pool)
            .await
            .unwrap();
        let second = result(
            session_post()
                .json(&json!({
                    "jsonrpc":"2.0", "id":9, "method":"tools/call",
                    "params":{"name":"assess_projects", "arguments":{
                        "run_id":"incremental", "project_ids":[1,2,3], "project_id":2,
                    }},
                }))
                .send()
                .await
                .unwrap(),
        )
        .await;
        let second = &second["result"]["structuredContent"];
        assert_eq!(second["completed"], first["completed"]);
        assert_eq!(second["failed"][0]["project_id"], 2);
        assert_eq!(second["failed"][0]["error"], "LLM response is incomplete");
        assert_eq!(second["pending_project_ids"], json!([3]));
        assert_eq!(calls.load(Ordering::SeqCst), 6);

        // Failure does not block later projects. Progress is scoped to this
        // one-project call, while structured chunk sequences retain run order.
        let mut third = post().header("MCP-Protocol-Version", "2026-07-28")
            .header("Mcp-Method", "tools/call").header("Mcp-Name", "assess_projects")
            .json(&json!({
                "jsonrpc":"2.0", "id":10, "method":"tools/call",
                "params":{"name":"assess_projects", "arguments":{
                    "run_id":"incremental", "project_ids":[1,2,3], "project_id":3,
                }, "_meta":{
                    "progressToken":"one-project",
                    "io.modelcontextprotocol/protocolVersion":"2026-07-28",
                    "io.modelcontextprotocol/clientCapabilities":{"extensions":{crate::assessments::CHUNKS:{}}},
                }},
            })).send().await.unwrap();
        let mut pending = Vec::new();
        let mut previous = -1.0;
        let third = loop {
            let message = event(&mut third, &mut pending).await;
            if message.get("id").is_some() {
                break message;
            }
            let progress = &message["params"];
            let value = progress["progress"].as_f64().unwrap();
            assert_eq!(progress["total"], 1.0);
            assert!(value > previous && value <= 1.0);
            previous = value;
            if !progress["_meta"][crate::assessments::CHUNKS].is_null() {
                assert_eq!(progress["_meta"][crate::assessments::CHUNKS]["sequence"], 3);
            }
        };
        assert_eq!(previous, 1.0);
        let third = &third["result"]["structuredContent"];
        assert_eq!(third["completed"].as_array().unwrap().len(), 2);
        assert_eq!(third["completed"][1]["project_id"], 3);
        assert_eq!(
            third["completed"][1]["metrics"]["actual_minor"],
            12 * 140300
        );
        assert_eq!(third["failed"], second["failed"]);
        assert_eq!(third["pending_project_ids"], json!([]));
        assert_eq!(calls.load(Ordering::SeqCst), 7);

        // Selectively retry the failed project; replaying a completion is free
        // of additional inference, and the read tool sees the same saved run.
        let retried = result(
            session_post()
                .json(&json!({
                    "jsonrpc":"2.0", "id":11, "method":"tools/call",
                    "params":{"name":"assess_projects", "arguments":{
                        "run_id":"incremental", "project_ids":[1,2,3], "project_id":2,
                    }},
                }))
                .send()
                .await
                .unwrap(),
        )
        .await;
        let retried = &retried["result"]["structuredContent"];
        assert_eq!(retried["completed"].as_array().unwrap().len(), 3);
        assert_eq!(retried["completed"][1]["project_id"], 2);
        assert_eq!(retried["failed"], json!([]));
        assert_eq!(retried["pending_project_ids"], json!([]));
        assert_eq!(calls.load(Ordering::SeqCst), 8);
        for (id, name, args) in [
            (
                12,
                "assess_projects",
                json!({"run_id":"incremental", "project_ids":[1,2,3], "project_id":1}),
            ),
            (13, "read_assessment_run", json!({"run_id":"incremental"})),
        ] {
            let saved = result(
                session_post()
                    .json(&json!({
                        "jsonrpc":"2.0", "id":id, "method":"tools/call",
                        "params":{"name":name, "arguments":args},
                    }))
                    .send()
                    .await
                    .unwrap(),
            )
            .await;
            assert_eq!(&saved["result"]["structuredContent"], retried);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 8);

        // Invalid selections and mismatched run inputs cannot start inference.
        for (id, args, error) in [
            (
                14,
                json!({"run_id":"incremental", "project_ids":[1,2,3], "project_id":4}),
                "project_id must be one of project_ids",
            ),
            (
                15,
                json!({"run_id":"incremental", "project_ids":[2], "project_id":2}),
                "Run ID already belongs to different inputs or analyzer settings",
            ),
        ] {
            let rejected = result(
                session_post()
                    .json(&json!({
                        "jsonrpc":"2.0", "id":id, "method":"tools/call",
                        "params":{"name":"assess_projects", "arguments":args},
                    }))
                    .send()
                    .await
                    .unwrap(),
            )
            .await;
            assert_eq!(rejected["result"]["isError"], true);
            assert_eq!(rejected["result"]["content"][0]["text"], error);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 8);
    }
}
