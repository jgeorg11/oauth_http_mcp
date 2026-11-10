# OAuth HTTP MCP local plugin

This source package connects the ChatGPT desktop app with local computer access
and Codex to the repository's Rust MCP server at `http://localhost:8001/mcp`
using Streamable HTTP. The host discovers capabilities from the server's tool
descriptions and schemas. A bundled assessment skill guides per-project replies
and waiting for long calls. The server defines these tools; each user sees only the
tools allowed by their database role:

| Tool | Arguments | Result |
| --- | --- | --- |
| `get_projects` | `{}` | All projects ordered by ID |
| `get_project_by_id` | `{ "project_id": 1 }` | One project, or a not-found tool error |
| `get_project_by_query` | `{ "query": "AI" }` | Projects with matching names |
| `create_project` | `name`, `owner`, `status`, `budget`, `risk_score` | New project with an assigned ID and timestamp |
| `update_project` | `project_id` and any fields to change | Updated project; omitted or null fields are preserved |
| `delete_project` | `{ "project_id": 6 }` | `{ "deleted": true }`, or a not-found tool error |
| `assess_projects` | `run_id`, `project_ids`, optional `project_id` | Returns after the selected project or full batch; cumulative completed, failed, and pending assessments, plus optional MCP progress |
| `read_assessment_run` | `run_id` | The calling user's saved completed, failed, and pending assessments |

List and search results use `{ "projects": [...] }`. Lookup, create, and update
results use `{ "project": {...} }`. Each tool advertises its output schema.

Search uses SQLite `LIKE` with a substring pattern; `%` and `_` act as
wildcards. Project records contain `id`, `name`, `owner`, `status`, `budget`,
`risk_score`, and `created_at`. The seed records are fictional. The schema
does not specify a budget currency or a risk-score scale.

The complete server and Entra setup is in the README at the
[source repository](https://github.com/YOUR_GITHUB_OWNER/YOUR_REPOSITORY).
Run the commands below from the repository root.

## Configure your own registrations

Create two single-tenant Microsoft Entra app registrations in your tenant:

1. **MCP API:** Set `api.requestedAccessTokenVersion` to `2`, add
   `http://localhost:8001/mcp` as an Application ID URI, and expose a delegated
   scope named `access_as_user`.
2. **OAuth client:** Add the API's delegated `access_as_user` permission and
   grant consent if required. Under **Authentication**, add a **Mobile and
   desktop applications** platform with this redirect URI:
   `http://localhost:1456/callback/nUXMQeJafTLA`.

See Microsoft's [MCP registration guidance](https://learn.microsoft.com/en-us/entra/agent-id/secure-mcp-server-with-entra-id)
and [identifier URI restrictions](https://learn.microsoft.com/en-us/entra/identity-platform/identifier-uri-restrictions)
if tenant policy rejects the local Application ID URI.

For a new checkout, create the local configuration files:

```bash
cp .env.example .env
cp plugin/.mcp.json.example plugin/.mcp.json
```

If either file already exists, edit it instead of overwriting it. Replace the
placeholders as follows:

| File | Setting | Value |
| --- | --- | --- |
| `.env` | `ENTRA_TENANT_ID` | Your directory's tenant ID |
| `.env` | `ENTRA_API_CLIENT_ID` | The API registration's application ID |
| `plugin/.mcp.json` | `oauth.clientId` | The OAuth client registration's application ID |

These are separate API and client IDs. This public desktop client uses
Authorization Code with PKCE and requires no client secret. Keep configured
`.env` and `.mcp.json` files local; Git ignores both. Publish their `.example`
templates instead. Never put tokens or client secrets in the package.

The example callback uses port `1456`, and the MCP server uses port `8001`.
The callback suffix is a public identifier derived from the local server URL.
Confirm the actual `redirect_uri` used during sign-in if you change the server
URL or client configuration. Keep the Entra redirect URI and the client
callback identical. See the
[official OAuth callback guidance](https://learn.chatgpt.com/docs/extend/mcp?surface=cli#oauth-client-registration-and-callbacks).

## Run the server

Create the local database from the seed script, check the build, and start
the server:

```bash
mkdir -p data
sqlite3 data/demo.db < data/seed.sql
sqlite3 data/demo.db < data/authorization.sql
sqlite3 data/demo.db < data/assessments.sql
sqlite3 data/demo.db < data/seed_assessments.sql
sqlite3 data/demo.db <<'SQL'
INSERT OR IGNORE INTO role_tools (role, tool_name) VALUES
    ('worker', 'assess_projects'),
    ('worker', 'read_assessment_run');
SQL
cargo check --locked
cargo run --locked
```

The seed command resets matching IDs to the seed values; skip it if you need
to preserve an existing database. The database file is ignored by Git and is
not distributed with the source.

Assign your verified Entra tenant ID and user object ID a role in `user_roles`,
as described in the repository README. With these grants, the `worker` role
has all eight tools; the `reporter` role has the three read tools. The placeholder assignments in
`data/authorization.sql` do not grant access to a real user. Tool permissions
are checked on discovery and again on every call.

Set `LLM_MODEL` in `.env` to an installed model that supports structured output.
`LLM_URL` defaults to `http://localhost:11434/api/chat`; an optional `LLM_API_KEY`
authenticates a compatible endpoint. Model requests use `stream: true`, and
the server reads NDJSON content as it arrives. Leave the model endpoint and
MCP server running while connecting the desktop app. Startup requires outbound
HTTPS access to Microsoft Entra for discovery and public signing keys.
`MCP_HOST` and `MCP_PORT` control the bind address; the template binds to
`127.0.0.1:8001` for local development.

## Install the local plugin and sign in

The repository marketplace at `.agents/plugins/marketplace.json` points to
this plugin directory. After configuring the local files and starting the
server, run:

```bash
codex plugin marketplace add .
codex plugin add oauth-http-mcp@oauth-local
codex mcp get oauth-http-mcp --json
codex mcp login oauth-http-mcp
```

These commands were checked with Codex CLI `0.162.1`. Inspect the `mcp get`
output to confirm your client ID, local server URL, and scopes. Complete
Microsoft sign-in in the browser, then close the callback window after it
confirms authentication. Restart the ChatGPT desktop app and start a new
conversation after the initial installation and login.

If the CLI commands are unavailable, use the desktop app's Plugins Directory
to install **OAuth HTTP MCP** from **OAuth Local Plugins** and complete its
authentication prompts. See the
[official local marketplace guide](https://developers.openai.com/plugins/build/plugins)
and [MCP configuration documentation](https://learn.chatgpt.com/docs/extend/mcp?surface=cli).

Select **OAuth HTTP MCP** and ask:

```text
Show me the projects.
Get project 1.
Find projects with AI in the name.
Create a project named MCP smoke test, owned by Test User, with status Planning, budget 1000, and risk score 2. Show its ID.
For that new project, change the budget to 1200 and leave everything else unchanged.
Get that project by its ID and verify the change.
Delete the MCP smoke test project using its returned ID.
```

### Run the incremental assessment demo in the desktop app

This workflow was verified in the desktop app with plugin `0.1.7` on
November 10, 2026. Assessments appeared as they completed, and the assistant
continued the run without a follow-up prompt.

Start a new desktop conversation with local computer access, select
**OAuth HTTP MCP**, and ask:

```text
Assess projects 1, 2, and 3 for spending pressure and overdue milestones.
Choose a fresh run ID, tell me that ID, and show each assessment as it finishes.
```

The assistant announces one run ID, then calls `assess_projects` with the
same full `project_ids: [1,2,3]` and `project_id: 1`, then `2`, then `3`.
Each call returns after that project is saved, giving the assistant a result
to report before starting the next call. It continues without another
user prompt, including when a call yields a running handle that needs waiting.
Each response contains the cumulative run; the assistant reports new outcomes.
Failed projects do not block the rest, and the skill limits automatic retries
to one additional attempt per failed project.

Omitting `project_id` retains a single batch response. That call can send MCP
progress messages when the host supplies a progress token, but those notifications
do not guarantee assistant replies during execution or restart an ended turn.
Messages include provisional model drafts (partial JSON), then saved summaries
and actions. Drafts are limited to ten updates per second. Only validated, saved
assessments appear as completed results. The verified assistant updates use
per-project tool returns; draft notification rendering remains controlled by
the host.

The server's `Assessment run started` log includes `selected_project_id` and
`progress_requested`. `Assessment unit completed` records individual outcomes.
Set `RUST_LOG=info,oauth_http_mcp::tools=debug` to log notification send attempts.

No custom extension is needed for ordinary progress. Hosts that advertise
`mcp.streaming/project-assessments-v1` additionally receive structured draft,
completed, and failed chunks in notification metadata. A prompt or plugin
manifest cannot make the host advertise this capability or supply a progress
token. See the repository README for the protocol and display limitations.

For recovery, stop a run after a project finishes and ask:

```text
Read the saved results for the assessment run we just started.
Resume that run with the same run ID and projects 1, 2, and 3 in the same order.
```

Completed units return from storage immediately. Use a fresh run ID for each
new streaming demonstration so the server performs new inference.

Confirm a successful tool result and a matching `MCP tool completed` log entry in the server
terminal. The entry includes the tool name and result count. The current
server has no `/health` endpoint. Run write prompts only as a user with the
`worker` role and use the newly created test project's ID.

## Reload after source changes

Stop the running server, then rebuild and restart it:

```bash
cargo test --locked
cargo run --locked
```

After changing this plugin package, install its updated source with:

```bash
codex plugin add oauth-http-mcp@oauth-local
```

Version `0.1.7` adds the assessment workflow skill and per-project return guidance.
Restart the desktop app and start a new conversation so refreshed
tool definitions are in context. Verify the installed version with `codex plugin list
--json`, then use the prompts above. If only read tools appear, check the
signed-in user's database role and refresh the MCP connection.

Future tool additions belong in the server. Restart the server and refresh
tool discovery; update this package only when its metadata or connection
configuration changes.

## Package contents and deployment

- `.codex-plugin/plugin.json` identifies the plugin and declares its MCP configuration and skills.
- `skills/assess-projects/SKILL.md` describes per-project calls, waiting, reporting, and bounded retries.
- `.mcp.json.example` is the published connection template. Copy it to
  `.mcp.json` and fill in your client ID before installation. OAuth endpoints
  are discovered through the server's public metadata.

This package uses the supported Codex compatibility manifest and `type: "http"`
MCP configuration. The portable Agent Plugins format has a different schema;
changing only the transport name or filename does not convert the package.
After changing an installed plugin, refresh or reinstall it using your
client's supported workflow and restart the desktop app.

Keep `MCP_RESOURCE`, the plugin URL, the requested API scope, and the API's
Entra Application ID URI consistent. The local scope is
`http://localhost:8001/mcp/access_as_user`; the expected v2 token audience
remains the API registration's application ID. Do not add a conflicting
`oauth_resource` override.

For remote hosting, use HTTPS and update these values together. Forward both
`/mcp` and `/.well-known/oauth-protected-resource/mcp` to the server. The
server checks Entra access tokens before running queries; missing or invalid
credentials return `401`, and a valid token without the required scope returns
`403`. The server periodically refreshes signing keys. See the repository
README for server setup and tool-level permissions.
