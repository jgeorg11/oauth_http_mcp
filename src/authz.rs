use std::collections::HashSet;

use sqlx::SqlitePool;

use crate::auth::Principal;

pub struct ToolPermissions {
    tools: HashSet<String>,
}

impl ToolPermissions {
    pub fn can_use(&self, tool_name: &str) -> bool {
        self.tools.contains(tool_name)
    }
}

pub async fn load_tool_permissions(
    pool: &SqlitePool,
    principal: &Principal,
) -> Result<ToolPermissions, sqlx::Error> {
    let tools = sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT rt.tool_name
         FROM user_roles AS ur
         JOIN role_tools AS rt ON rt.role = ur.role
         WHERE ur.tenant_id = ? AND ur.object_id = ?",
    )
    .bind(&principal.tenant_id)
    .bind(&principal.object_id)
    .fetch_all(pool)
    .await?;

    Ok(ToolPermissions {
        tools: tools.into_iter().collect(),
    })
}
