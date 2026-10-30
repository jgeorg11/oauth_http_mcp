use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub owner: String,
    pub status: String,
    pub budget: f64,
    pub risk_score: i64,
    pub created_at: String,
}
