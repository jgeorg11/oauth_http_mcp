---
name: assess-projects
description: Assess project spending and overdue milestones through OAuth HTTP MCP, reporting each project's result as it completes and continuing the run without another user prompt.
---

Use `assess_projects` once per selected project for live chat updates. Choose
and announce a fresh run ID unless the user is resuming an existing run. Keep
the full ordered `project_ids` list and `run_id` identical in every call; set
`project_id` to the current project. For example, a run for `[1, 2, 3]` uses
that same list three times with `project_id` set to `1`, then `2`, then `3`.
All calls share the original evidence snapshot.

Await each call's actual result. If the host returns a running handle or yields
execution, use its available wait/resume mechanism until the call completes.
A yielded call is still active; sending a final response at that point leaves
the user without the results. Keep waiting in the same turn unless the user
stops the work. MCP progress notifications can update a tool's UI but do not
guarantee a model-visible result or an assistant reply.

After each call, report that project's saved assessment or failure in a short
progress message, then call the next project without asking the user to
continue. Responses contain cumulative `completed`, `failed`, and
`pending_project_ids`; report only new outcomes. Partial model drafts are not
validated assessments. Use the returned metrics for spending calculations:
`*_minor` values are USD cents, while the legacy project budget has no declared
currency or units and cannot support a comparison without clarification.

Advance past a failed project so other projects still finish. After the initial
pass, retry each failed project at most once with its `project_id`, preserving
the run ID and full project list. Report failures that remain. End the turn
after every selected project has a saved completion or a reported failure.

After a disconnection, use `read_assessment_run` if available to recover saved
outcomes, then resume pending projects. If that read tool is not authorized,
replay the per-project calls: completed assessments return from cache. Never
start a duplicate call while its original is still running. Omitting
`project_id` is for callers that want one final batch response or consume MCP
progress directly.
