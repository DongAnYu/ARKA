use serde::{Deserialize, Serialize};
use sqlx::FromRow;

pub const DEFAULT_LLM_CONCURRENCY: usize = 5;
pub const MAX_LLM_CONCURRENCY: usize = 20;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct ModelConfig {
    pub provider: String,
    pub base_url: String,
    pub selected_model: String,
    pub timeout_secs: i64,
    pub api_key: Option<String>,
    pub llm_concurrency: i64,
    pub embedding_provider: String,
    pub embedding_base_url: String,
    pub embedding_selected_model: String,
    pub embedding_timeout_secs: i64,
    pub embedding_api_key: Option<String>,
}

impl ModelConfig {
    /// Returns the saved application-wide LLM request limit after validation.
    pub fn validated_llm_concurrency(&self) -> Result<usize, String> {
        let concurrency = usize::try_from(self.llm_concurrency).map_err(|_| {
            format!(
                "LLM concurrency must be between 1 and {MAX_LLM_CONCURRENCY}; received {}.",
                self.llm_concurrency
            )
        })?;

        if !(1..=MAX_LLM_CONCURRENCY).contains(&concurrency) {
            return Err(format!(
                "LLM concurrency must be between 1 and {MAX_LLM_CONCURRENCY}; received {concurrency}."
            ));
        }

        Ok(concurrency)
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with_concurrency(llm_concurrency: i64) -> ModelConfig {
        ModelConfig {
            provider: String::from("ollama"),
            base_url: String::from("http://localhost:11434"),
            selected_model: String::from("test-model"),
            timeout_secs: 60,
            api_key: None,
            llm_concurrency,
            embedding_provider: String::from("ollama"),
            embedding_base_url: String::from("http://localhost:11434"),
            embedding_selected_model: String::new(),
            embedding_timeout_secs: 60,
            embedding_api_key: None,
        }
    }

    #[test]
    fn validates_supported_llm_concurrency_range() {
        assert_eq!(
            config_with_concurrency(DEFAULT_LLM_CONCURRENCY as i64).validated_llm_concurrency(),
            Ok(DEFAULT_LLM_CONCURRENCY)
        );
        assert!(config_with_concurrency(0)
            .validated_llm_concurrency()
            .is_err());
        assert!(config_with_concurrency((MAX_LLM_CONCURRENCY + 1) as i64)
            .validated_llm_concurrency()
            .is_err());
    }
}
