use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct RecallSpace {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
}
