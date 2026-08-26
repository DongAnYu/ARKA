use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct ModelConfig {
    pub provider: String,
    pub base_url: String,
    pub selected_model: String,
    pub timeout_secs: i64,
    pub api_key: Option<String>,
    pub embedding_provider: String,
    pub embedding_base_url: String,
    pub embedding_selected_model: String,
    pub embedding_timeout_secs: i64,
    pub embedding_api_key: Option<String>,
}

/// Embedding settings submitted independently for connection testing.
#[derive(Debug, Serialize, Deserialize)]
pub struct EmbeddingModelConfig {
    pub provider: String,
    pub base_url: String,
    pub selected_model: String,
    pub timeout_secs: i64,
    pub api_key: Option<String>,
}

/// Successful embedding connection details shown by the Models page.
#[derive(Debug, Serialize)]
pub struct EmbeddingConnectionResult {
    pub provider: String,
    pub model: String,
    pub dimensions: usize,
}
