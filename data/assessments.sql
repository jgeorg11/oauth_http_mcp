CREATE TABLE IF NOT EXISTS project_spend (
    project_id INTEGER NOT NULL REFERENCES projects(id),
    period TEXT NOT NULL CHECK (
        date(period) IS period
        AND substr(period, 9, 2) = '01'
    ),
    planned_minor INTEGER NOT NULL CHECK (
        planned_minor BETWEEN 0 AND 1000000000000
    ),
    actual_minor INTEGER NOT NULL CHECK (
        actual_minor BETWEEN 0 AND 1000000000000
    ),
    PRIMARY KEY (project_id, period)
);

CREATE TABLE IF NOT EXISTS project_milestones (
    id INTEGER PRIMARY KEY,
    project_id INTEGER NOT NULL REFERENCES projects(id),
    title TEXT NOT NULL CHECK (length(title) <= 200),
    due_date TEXT NOT NULL CHECK (date(due_date) IS due_date),
    completed_date TEXT CHECK (
        completed_date IS NULL
        OR date(completed_date) IS completed_date
    )
);

CREATE TABLE IF NOT EXISTS project_updates (
    id INTEGER PRIMARY KEY,
    project_id INTEGER NOT NULL REFERENCES projects(id),
    reported_date TEXT NOT NULL CHECK (
        date(reported_date) IS reported_date
    ),
    body TEXT NOT NULL CHECK (length(body) <= 2000)
);

CREATE TABLE IF NOT EXISTS assessment_runs (
    owner TEXT NOT NULL,
    run_id TEXT NOT NULL,
    request_json TEXT NOT NULL,
    snapshot_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (owner, run_id)
);

CREATE TABLE IF NOT EXISTS assessment_units (
    owner TEXT NOT NULL,
    run_id TEXT NOT NULL,
    project_id INTEGER NOT NULL,
    result_json TEXT,
    error TEXT,
    CHECK ((result_json IS NULL) != (error IS NULL)),
    PRIMARY KEY (owner, run_id, project_id),
    FOREIGN KEY (owner, run_id)
        REFERENCES assessment_runs(owner, run_id)
);

