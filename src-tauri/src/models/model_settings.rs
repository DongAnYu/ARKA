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

impl ModelConfig {
    /// Extracts the embedding-specific settings used by connection testing and
    /// graph generation without duplicating field mapping at each call site.
    pub fn embedding_config(&self) -> EmbeddingModelConfig {
        EmbeddingModelConfig {
            provider: self.embedding_provider.clone(),
            base_url: self.embedding_base_url.clone(),
            selected_model: self.embedding_selected_model.clone(),
            timeout_secs: self.embedding_timeout_secs,
            api_key: self.embedding_api_key.clone(),
        }
    }
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
