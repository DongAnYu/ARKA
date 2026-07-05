use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct ModelConfig {
    pub provider: String,
    pub base_url: String,
    pub selected_model: String,
    pub timeout_secs: i64,
}
