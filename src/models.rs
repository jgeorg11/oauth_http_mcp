use rmcp::schemars;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, schemars::JsonSchema)]
pub struct Project {
    /// Stable numeric project ID for subsequent lookups, updates, or deletion.
    pub id: i64,
    /// Project name.
    pub name: String,
    /// Project owner as recorded in the connected database.
    pub owner: String,
    /// Project status as recorded in the connected database; no fixed status enum is defined.
    pub status: String,
    /// Recorded budget amount. The database does not define a currency.
    pub budget: f64,
    /// Recorded risk score. The database does not define a scale or thresholds.
    pub risk_score: i64,
    /// Creation timestamp stored by SQLite, normally in YYYY-MM-DD HH:MM:SS UTC format.
    pub created_at: String,
}
