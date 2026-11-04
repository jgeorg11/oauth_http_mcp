CREATE TABLE IF NOT EXISTS user_roles (
    tenant_id TEXT NOT NULL,
    object_id TEXT NOT NULL,
    role TEXT NOT NULL,
    PRIMARY KEY (tenant_id, object_id, role)
);

CREATE TABLE IF NOT EXISTS role_tools (
    role TEXT NOT NULL,
    tool_name TEXT NOT NULL,
    PRIMARY KEY (role, tool_name)
);

INSERT OR IGNORE INTO role_tools (role, tool_name) VALUES
    ('reporter', 'get_project_by_id'),
    ('reporter', 'get_project_by_query'),
    ('reporter', 'get_projects'),
    ('worker', 'get_project_by_id'),
    ('worker', 'get_project_by_query'),
    ('worker', 'get_projects'),
    ('worker', 'create_project'),
    ('worker', 'update_project'),
    ('worker', 'delete_project');

INSERT OR IGNORE INTO user_roles (tenant_id, object_id, role)
VALUES
    ('YOUR_TENANT_ID', 'JOHN_OBJECT_ID', 'worker'),
    ('YOUR_TENANT_ID', 'JANE_OBJECT_ID', 'reporter');
