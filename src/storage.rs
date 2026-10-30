use sqlx::SqlitePool;

use crate::models::Project;

pub async fn get_projects(pool: &SqlitePool) -> Result<Vec<Project>, sqlx::Error> {
    sqlx::query_as::<_, Project>("SELECT * FROM Projects ORDER BY id")
        .fetch_all(pool)
        .await
}

pub async fn get_project_by_id(
    pool: &SqlitePool,
    project_id: i64,
) -> Result<Option<Project>, sqlx::Error> {
    sqlx::query_as::<_, Project>("SELECT * FROM Projects WHERE id = ?")
        .bind(project_id)
        .fetch_optional(pool)
        .await
}

pub async fn get_project_by_query(
    pool: &SqlitePool,
    query: &str,
) -> Result<Vec<Project>, sqlx::Error> {
    sqlx::query_as::<_, Project>("SELECT * FROM Projects WHERE name LIKE ? ORDER BY id")
        .bind(format!("%{}%", query))
        .fetch_all(pool)
        .await
}
