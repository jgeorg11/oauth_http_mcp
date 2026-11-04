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

pub async fn create_project(
    pool: &SqlitePool,
    name: &str,
    owner: &str,
    status: &str,
    budget: f64,
    risk_score: i64,
) -> Result<Project, sqlx::Error> {
    sqlx::query_as::<_, Project>(
        "INSERT INTO Projects (name, owner, status, budget, risk_score)
         VALUES (?, ?, ?, ?, ?) RETURNING *",
    )
    .bind(name)
    .bind(owner)
    .bind(status)
    .bind(budget)
    .bind(risk_score)
    .fetch_one(pool)
    .await
}

pub async fn update_project(
    pool: &SqlitePool,
    project_id: i64,
    name: Option<&str>,
    budget: Option<f64>,
    owner: Option<&str>,
    risk_score: Option<i64>,
    status: Option<&str>,
) -> Result<Option<Project>, sqlx::Error> {
    let mut tx = pool.begin().await?;

    let rows_affected = sqlx::query(
        "UPDATE Projects SET name = COALESCE(?, name), budget = COALESCE(?, budget),
         owner = COALESCE(?, owner), risk_score = COALESCE(?, risk_score),
         status = COALESCE(?, status) WHERE id = ?",
    )
    .bind(name)
    .bind(budget)
    .bind(owner)
    .bind(risk_score)
    .bind(status)
    .bind(project_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();

    if rows_affected == 0 {
        tx.rollback().await?;
        return Ok(None);
    }

    let updated_project = sqlx::query_as::<_, Project>("SELECT * FROM Projects WHERE id = ?")
        .bind(project_id)
        .fetch_one(&mut *tx)
        .await?;

    tx.commit().await?;

    Ok(Some(updated_project))
}

pub async fn delete_project(pool: &SqlitePool, project_id: i64) -> Result<bool, sqlx::Error> {
    let rows_affected = sqlx::query("DELETE FROM Projects WHERE id = ?")
        .bind(project_id)
        .execute(pool)
        .await?
        .rows_affected();

    Ok(rows_affected > 0)
}
