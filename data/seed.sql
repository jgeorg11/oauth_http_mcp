CREATE TABLE IF NOT EXISTS projects (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    owner TEXT NOT NULL,
    status TEXT NOT NULL,
    budget REAL NOT NULL,
    risk_score INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

INSERT INTO projects
    (id, name, owner, status, budget, risk_score)
VALUES
    (1, 'AI Governance Dashboard', 'Alice', 'Active', 150000, 3),
    (2, 'Document Intelligence', 'Bob', 'Planning', 85000, 5),
    (3, 'Export Control Assistant', 'Charlie', 'Active', 225000, 7),
    (4, 'Predictive Maintenance', 'Alice', 'Completed', 310000, 2),
    (5, 'Research Knowledge Graph', 'Diana', 'Active', 175000, 6)
ON CONFLICT(id) DO UPDATE SET
    name = excluded.name,
    owner = excluded.owner,
    status = excluded.status,
    budget = excluded.budget,
    risk_score = excluded.risk_score;
