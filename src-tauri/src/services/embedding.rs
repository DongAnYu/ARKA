//! Provider-neutral embedding service and response validation.
//!
//! Ollama, OpenAI, and OpenRouter share one validated batch contract. Callers
//! can rely on `EmbeddingBatch` only containing non-empty, finite vectors with
//! a consistent dimensionality.

use std::error::Error;
use std::fmt;
use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};

/// Embedding providers supported by ARKA's configuration contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingProvider {
    Ollama,
    OpenAi,
    OpenRouter,
}

impl EmbeddingProvider {
    /// Parses the stable provider identifiers stored in model settings.
    pub fn from_config_value(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "ollama" => Some(Self::Ollama),
            "openai" => Some(Self::OpenAi),
            "openrouter" => Some(Self::OpenRouter),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ollama => "ollama",
            Self::OpenAi => "openai",
            Self::OpenRouter => "openrouter",
        }
    }
}

/// Configuration shared by embedding provider implementations.
#[derive(Clone, PartialEq, Eq)]
pub struct EmbeddingConfig {
    provider: EmbeddingProvider,
    base_url: String,
    model: String,
    timeout_secs: u64,
    api_key: Option<String>,
}

impl EmbeddingConfig {
    /// Creates normalized embedding configuration.
    ///
    /// OpenAI and OpenRouter require a non-empty API key; Ollama does not.
    pub fn new(
        provider: EmbeddingProvider,
        base_url: impl Into<String>,
        model: impl Into<String>,
        timeout_secs: u64,
        api_key: Option<String>,
    ) -> Result<Self, EmbeddingConfigError> {
        let base_url = base_url.into().trim().to_string();
        if base_url.is_empty() {
            return Err(EmbeddingConfigError::EmptyBaseUrl);
        }

        let model = model.into().trim().to_string();
        if model.is_empty() {
            return Err(EmbeddingConfigError::EmptyModel);
        }

        if timeout_secs == 0 {
            return Err(EmbeddingConfigError::ZeroTimeout);
        }

        let api_key = api_key
            .map(|key| key.trim().to_string())
            .filter(|key| !key.is_empty());
        if matches!(
            provider,
            EmbeddingProvider::OpenAi | EmbeddingProvider::OpenRouter
        ) && api_key.is_none()
        {
            return Err(EmbeddingConfigError::MissingApiKey);
        }

        Ok(Self {
            provider,
            base_url,
            model,
            timeout_secs,
            api_key,
        })
    }

    pub fn provider(&self) -> EmbeddingProvider {
        self.provider
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn timeout_secs(&self) -> u64 {
        self.timeout_secs
    }

    pub fn api_key(&self) -> Option<&str> {
        self.api_key.as_deref()
    }
}

impl fmt::Debug for EmbeddingConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EmbeddingConfig")
            .field("provider", &self.provider)
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("timeout_secs", &self.timeout_secs)
            .field("has_api_key", &self.api_key.is_some())
            .finish()
    }
}

/// Errors found before an embedding client is constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingConfigError {
    EmptyBaseUrl,
    EmptyModel,
    ZeroTimeout,
    MissingApiKey,
}

impl fmt::Display for EmbeddingConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyBaseUrl => write!(formatter, "Embedding base URL must not be empty"),
            Self::EmptyModel => write!(formatter, "Embedding model must not be empty"),
            Self::ZeroTimeout => write!(formatter, "Embedding timeout must be greater than zero"),
            Self::MissingApiKey => write!(
                formatter,
                "OpenAI and OpenRouter embedding configurations require an API key"
            ),
        }
    }
}

impl Error for EmbeddingConfigError {}

/// One validated embedding vector.
#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddingVector {
    values: Vec<f32>,
}

impl EmbeddingVector {
    pub fn values(&self) -> &[f32] {
        &self.values
    }

    pub fn dimensions(&self) -> usize {
        self.values.len()
    }

    pub fn into_values(self) -> Vec<f32> {
        self.values
    }
}

/// A validated provider response whose vector order matches input order.
#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddingBatch {
    vectors: Vec<EmbeddingVector>,
    dimensions: usize,
}

impl EmbeddingBatch {
    /// Validates raw provider vectors against the number of submitted inputs.
    ///
    /// An empty batch is valid only when no inputs were submitted. Non-empty
    /// batches must contain non-empty, equally sized, finite vectors.
    pub fn try_from_raw(
        raw_vectors: Vec<Vec<f32>>,
        expected_count: usize,
    ) -> Result<Self, EmbeddingValidationError> {
        if raw_vectors.len() != expected_count {
            return Err(EmbeddingValidationError::VectorCountMismatch {
                expected: expected_count,
                actual: raw_vectors.len(),
            });
        }

        if raw_vectors.is_empty() {
            return Ok(Self {
                vectors: Vec::new(),
                dimensions: 0,
            });
        }

        let dimensions = raw_vectors[0].len();
        if dimensions == 0 {
            return Err(EmbeddingValidationError::EmptyVector { vector_index: 0 });
        }

        let mut vectors = Vec::with_capacity(raw_vectors.len());
        for (vector_index, values) in raw_vectors.into_iter().enumerate() {
            if values.is_empty() {
                return Err(EmbeddingValidationError::EmptyVector { vector_index });
            }

            if values.len() != dimensions {
                return Err(EmbeddingValidationError::DimensionMismatch {
                    vector_index,
                    expected: dimensions,
                    actual: values.len(),
                });
            }

            if let Some(value_index) = values.iter().position(|value| !value.is_finite()) {
                return Err(EmbeddingValidationError::NonFiniteValue {
                    vector_index,
                    value_index,
                });
            }

            vectors.push(EmbeddingVector { values });
        }

        Ok(Self {
            vectors,
            dimensions,
        })
    }

    pub fn vectors(&self) -> &[EmbeddingVector] {
        &self.vectors
    }

    pub fn dimensions(&self) -> usize {
        self.dimensions
    }

    pub fn len(&self) -> usize {
        self.vectors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.vectors.is_empty()
    }

    pub fn into_vectors(self) -> Vec<EmbeddingVector> {
        self.vectors
    }
}

/// Structural problems in an embedding provider response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbeddingValidationError {
    VectorCountMismatch {
        expected: usize,
        actual: usize,
    },
    EmptyVector {
        vector_index: usize,
    },
    DimensionMismatch {
        vector_index: usize,
        expected: usize,
        actual: usize,
    },
    NonFiniteValue {
        vector_index: usize,
        value_index: usize,
    },
    DuplicateVectorIndex {
        index: usize,
    },
    VectorIndexOutOfRange {
        index: usize,
        expected_count: usize,
    },
    MissingVectorIndex {
        index: usize,
    },
}

impl fmt::Display for EmbeddingValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VectorCountMismatch { expected, actual } => write!(
                formatter,
                "Embedding response returned {actual} vectors for {expected} inputs"
            ),
            Self::EmptyVector { vector_index } => {
                write!(formatter, "Embedding vector {vector_index} is empty")
            }
            Self::DimensionMismatch {
                vector_index,
                expected,
                actual,
            } => write!(
                formatter,
                "Embedding vector {vector_index} has {actual} dimensions; expected {expected}"
            ),
            Self::NonFiniteValue {
                vector_index,
                value_index,
            } => write!(
                formatter,
                "Embedding vector {vector_index} contains a non-finite value at index {value_index}"
            ),
            Self::DuplicateVectorIndex { index } => {
                write!(
                    formatter,
                    "Embedding response contains duplicate index {index}"
                )
            }
            Self::VectorIndexOutOfRange {
                index,
                expected_count,
            } => write!(
                formatter,
                "Embedding response index {index} is outside the expected range 0..{expected_count}"
            ),
            Self::MissingVectorIndex { index } => {
                write!(formatter, "Embedding response is missing index {index}")
            }
        }
    }
}

impl Error for EmbeddingValidationError {}

#[derive(Debug, Serialize)]
struct OllamaEmbedRequest<'a> {
    model: &'a str,
    input: &'a [String],
    truncate: bool,
}

#[derive(Debug, Deserialize)]
struct OllamaEmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

#[derive(Debug, Deserialize)]
struct OllamaErrorResponse {
    error: String,
}

#[derive(Debug, Serialize)]
struct OpenAiCompatibleEmbedRequest<'a> {
    model: &'a str,
    input: &'a [String],
    encoding_format: &'static str,
}

#[derive(Debug, Deserialize)]
struct OpenAiCompatibleEmbedResponse {
    data: Vec<OpenAiCompatibleEmbedding>,
}

#[derive(Debug, Deserialize)]
struct OpenAiCompatibleEmbedding {
    index: usize,
    embedding: Vec<f32>,
}

#[derive(Debug, Deserialize)]
struct OpenAiCompatibleErrorResponse {
    error: OpenAiCompatibleErrorDetails,
}

#[derive(Debug, Deserialize)]
struct OpenAiCompatibleErrorDetails {
    message: String,
}

/// Failures while creating the embedding client or requesting vectors.
#[derive(Debug)]
pub enum EmbeddingServiceError {
    HttpClientBuild(reqwest::Error),
    Connect { url: String, source: reqwest::Error },
    Http(reqwest::Error),
    HttpStatus { status: StatusCode, message: String },
    ResponseDecode(serde_json::Error),
    InvalidResponse(EmbeddingValidationError),
}

impl fmt::Display for EmbeddingServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HttpClientBuild(error) => {
                write!(formatter, "Failed to build embedding HTTP client: {error}")
            }
            Self::Connect { url, source } => {
                write!(
                    formatter,
                    "Cannot reach embedding endpoint at {url}: {source}"
                )
            }
            Self::Http(error) => write!(formatter, "Embedding request failed: {error}"),
            Self::HttpStatus { status, message } => write!(
                formatter,
                "Embedding provider rejected the request ({status}): {message}"
            ),
            Self::ResponseDecode(error) => {
                write!(formatter, "Failed to decode embedding response: {error}")
            }
            Self::InvalidResponse(error) => {
                write!(formatter, "Invalid embedding response: {error}")
            }
        }
    }
}

impl Error for EmbeddingServiceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::HttpClientBuild(error) | Self::Http(error) => Some(error),
            Self::Connect { source, .. } => Some(source),
            Self::ResponseDecode(error) => Some(error),
            Self::InvalidResponse(error) => Some(error),
            Self::HttpStatus { .. } => None,
        }
    }
}

impl From<EmbeddingValidationError> for EmbeddingServiceError {
    fn from(error: EmbeddingValidationError) -> Self {
        Self::InvalidResponse(error)
    }
}

/// Sends embedding requests and validates provider responses.
#[derive(Debug, Clone)]
pub struct EmbeddingService {
    client: Client,
    config: EmbeddingConfig,
}

impl EmbeddingService {
    pub fn new(config: EmbeddingConfig) -> Result<Self, EmbeddingServiceError> {
        let mut builder = Client::builder().timeout(Duration::from_secs(config.timeout_secs()));
        if config.provider() == EmbeddingProvider::Ollama {
            builder = builder.no_proxy();
        }

        let client = builder
            .build()
            .map_err(EmbeddingServiceError::HttpClientBuild)?;

        Ok(Self { client, config })
    }

    pub fn config(&self) -> &EmbeddingConfig {
        &self.config
    }

    /// Generates one embedding per input, retaining the submitted input order.
    pub async fn embed_batch(
        &self,
        inputs: &[String],
    ) -> Result<EmbeddingBatch, EmbeddingServiceError> {
        if inputs.is_empty() {
            return EmbeddingBatch::try_from_raw(Vec::new(), 0).map_err(Into::into);
        }

        match self.config.provider() {
            EmbeddingProvider::Ollama => self.embed_batch_ollama(inputs).await,
            EmbeddingProvider::OpenAi | EmbeddingProvider::OpenRouter => {
                self.embed_batch_openai_compatible(inputs).await
            }
        }
    }

    async fn embed_batch_ollama(
        &self,
        inputs: &[String],
    ) -> Result<EmbeddingBatch, EmbeddingServiceError> {
        let endpoint = format!("{}/api/embed", self.config.base_url().trim_end_matches('/'));
        let payload = OllamaEmbedRequest {
            model: self.config.model(),
            input: inputs,
            // Reject inputs that exceed the embedding model's context window instead
            // of silently embedding truncated entity evidence.
            truncate: false,
        };

        let response = self
            .client
            .post(&endpoint)
            .json(&payload)
            .send()
            .await
            .map_err(|source| EmbeddingServiceError::Connect {
                url: endpoint.clone(),
                source,
            })?;

        let status = response.status();
        let body = response.text().await.map_err(EmbeddingServiceError::Http)?;
        if !status.is_success() {
            return Err(EmbeddingServiceError::HttpStatus {
                status,
                message: ollama_error_message(&body),
            });
        }

        let response: OllamaEmbedResponse =
            serde_json::from_str(&body).map_err(EmbeddingServiceError::ResponseDecode)?;

        EmbeddingBatch::try_from_raw(response.embeddings, inputs.len()).map_err(Into::into)
    }

    /// Sends the shared `/embeddings` contract used by OpenAI and OpenRouter.
    async fn embed_batch_openai_compatible(
        &self,
        inputs: &[String],
    ) -> Result<EmbeddingBatch, EmbeddingServiceError> {
        let endpoint = format!(
            "{}/embeddings",
            self.config.base_url().trim_end_matches('/')
        );
        let payload = OpenAiCompatibleEmbedRequest {
            model: self.config.model(),
            input: inputs,
            encoding_format: "float",
        };
        let api_key = self
            .config
            .api_key()
            .expect("validated remote embedding config must contain an API key");

        let response = self
            .client
            .post(&endpoint)
            .bearer_auth(api_key)
            .json(&payload)
            .send()
            .await
            .map_err(|source| EmbeddingServiceError::Connect {
                url: endpoint.clone(),
                source,
            })?;

        let status = response.status();
        let body = response.text().await.map_err(EmbeddingServiceError::Http)?;
        if !status.is_success() {
            return Err(EmbeddingServiceError::HttpStatus {
                status,
                message: openai_compatible_error_message(&body, self.config.provider()),
            });
        }

        let response: OpenAiCompatibleEmbedResponse =
            serde_json::from_str(&body).map_err(EmbeddingServiceError::ResponseDecode)?;
        let vectors = order_openai_compatible_embeddings(response.data, inputs.len())?;

        EmbeddingBatch::try_from_raw(vectors, inputs.len()).map_err(Into::into)
    }
}

fn ollama_error_message(body: &str) -> String {
    let Ok(response) = serde_json::from_str::<OllamaErrorResponse>(body) else {
        return String::from("Ollama embedding request failed");
    };
    bounded_error_message(&response.error, "Ollama embedding request failed")
}

fn openai_compatible_error_message(body: &str, provider: EmbeddingProvider) -> String {
    let fallback = match provider {
        EmbeddingProvider::OpenAi => "OpenAI embedding request failed",
        EmbeddingProvider::OpenRouter => "OpenRouter embedding request failed",
        EmbeddingProvider::Ollama => "Embedding request failed",
    };
    let Ok(response) = serde_json::from_str::<OpenAiCompatibleErrorResponse>(body) else {
        return fallback.to_string();
    };
    bounded_error_message(&response.error.message, fallback)
}

fn bounded_error_message(message: &str, fallback: &str) -> String {
    const MAX_CHARS: usize = 300;

    let normalized = message.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return fallback.to_string();
    }
    if normalized.chars().count() <= MAX_CHARS {
        return normalized;
    }

    let mut truncated = normalized.chars().take(MAX_CHARS - 3).collect::<String>();
    truncated.push_str("...");
    truncated
}

/// Restores OpenAI-compatible embeddings to the original request-input order.
///
/// OpenAI and OpenRouter identify each returned vector with an `index`, so
/// response array order is not assumed to match input order. This function
/// places every vector into its indexed slot and rejects duplicate, missing, or
/// out-of-range indices before entity contexts are associated by position.
fn order_openai_compatible_embeddings(
    embeddings: Vec<OpenAiCompatibleEmbedding>,
    expected_count: usize,
) -> Result<Vec<Vec<f32>>, EmbeddingValidationError> {
    let mut ordered = (0..expected_count)
        .map(|_| None)
        .collect::<Vec<Option<Vec<f32>>>>();

    for item in embeddings {
        if item.index >= expected_count {
            return Err(EmbeddingValidationError::VectorIndexOutOfRange {
                index: item.index,
                expected_count,
            });
        }
        if ordered[item.index].replace(item.embedding).is_some() {
            return Err(EmbeddingValidationError::DuplicateVectorIndex { index: item.index });
        }
    }

    ordered
        .into_iter()
        .enumerate()
        .map(|(index, embedding)| {
            embedding.ok_or(EmbeddingValidationError::MissingVectorIndex { index })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;

    fn config() -> EmbeddingConfig {
        EmbeddingConfig::new(
            EmbeddingProvider::Ollama,
            "http://127.0.0.1:11434",
            "nomic-embed-text",
            60,
            None,
        )
        .expect("test embedding configuration should be valid")
    }

    async fn mock_http_response(
        status_line: &str,
        response_body: &str,
    ) -> (String, oneshot::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock listener should bind");
        let address = listener
            .local_addr()
            .expect("mock listener should have an address");
        let status_line = status_line.to_string();
        let response_body = response_body.to_string();
        let (request_sender, request_receiver) = oneshot::channel();

        tokio::spawn(async move {
            let (mut socket, _) = listener
                .accept()
                .await
                .expect("mock listener should accept one request");
            let mut request = Vec::new();
            let mut chunk = [0_u8; 1024];

            loop {
                let bytes_read = socket
                    .read(&mut chunk)
                    .await
                    .expect("mock server should read the request");
                if bytes_read == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..bytes_read]);

                let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n")
                else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                if request.len() >= header_end + 4 + content_length {
                    break;
                }
            }

            let _ = request_sender.send(String::from_utf8_lossy(&request).into_owned());
            let response = format!(
                "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                response_body.len()
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("mock server should write the response");
        });

        (format!("http://{address}"), request_receiver)
    }

    fn service_for(provider: EmbeddingProvider, base_url: &str) -> EmbeddingService {
        let api_key = matches!(
            provider,
            EmbeddingProvider::OpenAi | EmbeddingProvider::OpenRouter
        )
        .then(|| String::from("test-api-key"));
        let config = EmbeddingConfig::new(provider, base_url, "nomic-embed-text", 5, api_key)
            .expect("mock embedding configuration should be valid");
        EmbeddingService::new(config).expect("mock embedding service should build")
    }

    #[test]
    fn configuration_is_trimmed_and_blank_api_key_becomes_none() {
        let config = EmbeddingConfig::new(
            EmbeddingProvider::Ollama,
            "  http://127.0.0.1:11434  ",
            "  nomic-embed-text  ",
            60,
            Some(String::from("   ")),
        )
        .expect("configuration should normalize successfully");

        assert_eq!(config.base_url(), "http://127.0.0.1:11434");
        assert_eq!(config.model(), "nomic-embed-text");
        assert_eq!(config.timeout_secs(), 60);
        assert_eq!(config.api_key(), None);
    }

    #[test]
    fn parses_the_three_persisted_provider_values() {
        assert_eq!(
            EmbeddingProvider::from_config_value(" ollama "),
            Some(EmbeddingProvider::Ollama)
        );
        assert_eq!(
            EmbeddingProvider::from_config_value("OPENAI"),
            Some(EmbeddingProvider::OpenAi)
        );
        assert_eq!(
            EmbeddingProvider::from_config_value("openrouter"),
            Some(EmbeddingProvider::OpenRouter)
        );
        assert_eq!(EmbeddingProvider::from_config_value("unknown"), None);
    }

    #[test]
    fn configuration_preserves_a_trimmed_api_key() {
        let config = EmbeddingConfig::new(
            EmbeddingProvider::OpenRouter,
            "https://provider.example/api/v1",
            "embedding-model",
            30,
            Some(String::from("  secret-value  ")),
        )
        .expect("configuration should be valid");

        assert_eq!(config.api_key(), Some("secret-value"));
    }

    #[test]
    fn configuration_rejects_empty_required_values() {
        assert_eq!(
            EmbeddingConfig::new(EmbeddingProvider::Ollama, " ", "model", 60, None),
            Err(EmbeddingConfigError::EmptyBaseUrl)
        );
        assert_eq!(
            EmbeddingConfig::new(EmbeddingProvider::Ollama, "http://localhost", " ", 60, None),
            Err(EmbeddingConfigError::EmptyModel)
        );
        assert_eq!(
            EmbeddingConfig::new(
                EmbeddingProvider::Ollama,
                "http://localhost",
                "model",
                0,
                None,
            ),
            Err(EmbeddingConfigError::ZeroTimeout)
        );
        assert_eq!(
            EmbeddingConfig::new(
                EmbeddingProvider::OpenAi,
                "https://api.openai.com/v1",
                "text-embedding-3-small",
                60,
                None,
            ),
            Err(EmbeddingConfigError::MissingApiKey)
        );
        assert_eq!(
            EmbeddingConfig::new(
                EmbeddingProvider::OpenRouter,
                "https://openrouter.ai/api/v1",
                "embedding-model",
                60,
                None,
            ),
            Err(EmbeddingConfigError::MissingApiKey)
        );
        assert_eq!(
            EmbeddingConfig::new(
                EmbeddingProvider::OpenRouter,
                "https://openrouter.ai/api/v1",
                "embedding-model",
                60,
                Some(String::from("   ")),
            ),
            Err(EmbeddingConfigError::MissingApiKey)
        );
    }

    #[test]
    fn service_retains_validated_configuration() {
        let service = EmbeddingService::new(config())
            .expect("test embedding service should build its HTTP client");

        assert_eq!(service.config().model(), "nomic-embed-text");
    }

    #[tokio::test]
    async fn ollama_embed_batch_posts_expected_payload_and_returns_vectors() {
        let (base_url, request_receiver) = mock_http_response(
            "200 OK",
            r#"{"model":"nomic-embed-text","embeddings":[[0.1,0.2],[0.3,0.4]]}"#,
        )
        .await;
        let service = service_for(EmbeddingProvider::Ollama, &format!("{base_url}/"));
        let inputs = vec![String::from("first entity"), String::from("second entity")];

        let batch = service
            .embed_batch(&inputs)
            .await
            .expect("valid Ollama response should produce a batch");
        let request = request_receiver
            .await
            .expect("mock server should capture the request");
        let request_body = request
            .split_once("\r\n\r\n")
            .map(|(_, body)| body)
            .expect("request should contain a body");
        let payload: serde_json::Value =
            serde_json::from_str(request_body).expect("request body should be valid JSON");

        assert!(request.starts_with("POST /api/embed HTTP/1.1"));
        assert_eq!(payload["model"], "nomic-embed-text");
        assert_eq!(payload["input"], serde_json::json!(inputs));
        assert_eq!(payload["truncate"], false);
        assert_eq!(batch.len(), 2);
        assert_eq!(batch.dimensions(), 2);
        assert_eq!(batch.vectors()[1].values(), &[0.3, 0.4]);
    }

    #[tokio::test]
    async fn empty_input_returns_without_contacting_the_provider() {
        let service = service_for(EmbeddingProvider::Ollama, "http://127.0.0.1:1");

        let batch = service
            .embed_batch(&[])
            .await
            .expect("empty input should not require a provider request");

        assert!(batch.is_empty());
    }

    #[tokio::test]
    async fn ollama_errors_use_the_provider_message() {
        let (base_url, _) = mock_http_response(
            "404 Not Found",
            r#"{"error":"  model   'missing-model'   not found  "}"#,
        )
        .await;
        let service = service_for(EmbeddingProvider::Ollama, &base_url);

        let error = service
            .embed_batch(&[String::from("entity")])
            .await
            .expect_err("non-success response should fail");

        assert!(matches!(
            error,
            EmbeddingServiceError::HttpStatus {
                status: StatusCode::NOT_FOUND,
                ref message,
            } if message == "model 'missing-model' not found"
        ));
    }

    #[tokio::test]
    async fn ollama_response_is_checked_by_batch_validation() {
        let (base_url, _) = mock_http_response("200 OK", r#"{"embeddings":[[0.1,0.2]]}"#).await;
        let service = service_for(EmbeddingProvider::Ollama, &base_url);

        let error = service
            .embed_batch(&[String::from("one"), String::from("two")])
            .await
            .expect_err("missing response vectors should fail validation");

        assert!(matches!(
            error,
            EmbeddingServiceError::InvalidResponse(EmbeddingValidationError::VectorCountMismatch {
                expected: 2,
                actual: 1,
            })
        ));
    }

    #[tokio::test]
    async fn malformed_ollama_response_is_rejected() {
        let (base_url, _) = mock_http_response("200 OK", r#"{"embeddings":"invalid"}"#).await;
        let service = service_for(EmbeddingProvider::Ollama, &base_url);

        let error = service
            .embed_batch(&[String::from("entity")])
            .await
            .expect_err("malformed response should fail decoding");

        assert!(matches!(error, EmbeddingServiceError::ResponseDecode(_)));
    }

    #[tokio::test]
    async fn openrouter_embed_batch_authenticates_and_restores_index_order() {
        let (base_url, request_receiver) = mock_http_response(
            "200 OK",
            r#"{"data":[{"index":1,"embedding":[0.3,0.4]},{"index":0,"embedding":[0.1,0.2]}]}"#,
        )
        .await;
        let service = service_for(
            EmbeddingProvider::OpenRouter,
            &format!("{base_url}/api/v1/"),
        );
        let inputs = vec![String::from("first entity"), String::from("second entity")];

        let batch = service
            .embed_batch(&inputs)
            .await
            .expect("valid OpenRouter response should produce a batch");
        let request = request_receiver
            .await
            .expect("mock server should capture the request");
        let request_body = request
            .split_once("\r\n\r\n")
            .map(|(_, body)| body)
            .expect("request should contain a body");
        let payload: serde_json::Value =
            serde_json::from_str(request_body).expect("request body should be valid JSON");

        assert!(request.starts_with("POST /api/v1/embeddings HTTP/1.1"));
        assert!(request
            .to_ascii_lowercase()
            .contains("authorization: bearer test-api-key"));
        assert_eq!(payload["model"], "nomic-embed-text");
        assert_eq!(payload["input"], serde_json::json!(inputs));
        assert_eq!(payload["encoding_format"], "float");
        assert_eq!(batch.vectors()[0].values(), &[0.1, 0.2]);
        assert_eq!(batch.vectors()[1].values(), &[0.3, 0.4]);
    }

    #[tokio::test]
    async fn openai_embed_batch_authenticates_and_restores_index_order() {
        let (base_url, request_receiver) = mock_http_response(
            "200 OK",
            r#"{"data":[{"object":"embedding","index":1,"embedding":[0.3,0.4]},{"object":"embedding","index":0,"embedding":[0.1,0.2]}],"model":"text-embedding-3-small","object":"list"}"#,
        )
        .await;
        let config = EmbeddingConfig::new(
            EmbeddingProvider::OpenAi,
            format!("{base_url}/v1/"),
            "text-embedding-3-small",
            5,
            Some(String::from("test-api-key")),
        )
        .expect("OpenAI test configuration should be valid");
        let service =
            EmbeddingService::new(config).expect("OpenAI test embedding service should build");
        let inputs = vec![String::from("first entity"), String::from("second entity")];

        let batch = service
            .embed_batch(&inputs)
            .await
            .expect("valid OpenAI response should produce a batch");
        let request = request_receiver
            .await
            .expect("mock server should capture the request");
        let request_body = request
            .split_once("\r\n\r\n")
            .map(|(_, body)| body)
            .expect("request should contain a body");
        let payload: serde_json::Value =
            serde_json::from_str(request_body).expect("request body should be valid JSON");

        assert!(request.starts_with("POST /v1/embeddings HTTP/1.1"));
        assert!(request
            .to_ascii_lowercase()
            .contains("authorization: bearer test-api-key"));
        assert_eq!(payload["model"], "text-embedding-3-small");
        assert_eq!(payload["input"], serde_json::json!(inputs));
        assert_eq!(payload["encoding_format"], "float");
        assert_eq!(batch.vectors()[0].values(), &[0.1, 0.2]);
        assert_eq!(batch.vectors()[1].values(), &[0.3, 0.4]);
    }

    #[tokio::test]
    async fn openai_errors_use_the_provider_message() {
        let (base_url, _) = mock_http_response(
            "401 Unauthorized",
            r#"{"error":{"message":"  Incorrect   API key  ","type":"invalid_request_error"}}"#,
        )
        .await;
        let service = service_for(EmbeddingProvider::OpenAi, &format!("{base_url}/v1"));

        let error = service
            .embed_batch(&[String::from("entity")])
            .await
            .expect_err("non-success OpenAI response should fail");

        assert!(matches!(
            error,
            EmbeddingServiceError::HttpStatus {
                status: StatusCode::UNAUTHORIZED,
                ref message,
            } if message == "Incorrect API key"
        ));
    }

    #[tokio::test]
    async fn openrouter_errors_use_the_provider_message() {
        let (base_url, _) = mock_http_response(
            "401 Unauthorized",
            r#"{"error":{"code":401,"message":"  Invalid   API key  "}}"#,
        )
        .await;
        let service = service_for(EmbeddingProvider::OpenRouter, &format!("{base_url}/api/v1"));

        let error = service
            .embed_batch(&[String::from("entity")])
            .await
            .expect_err("non-success response should fail");

        assert!(matches!(
            error,
            EmbeddingServiceError::HttpStatus {
                status: StatusCode::UNAUTHORIZED,
                ref message,
            } if message == "Invalid API key"
        ));
    }

    #[test]
    fn openai_compatible_indices_must_be_unique_complete_and_in_range() {
        let duplicate = order_openai_compatible_embeddings(
            vec![
                OpenAiCompatibleEmbedding {
                    index: 0,
                    embedding: vec![0.1],
                },
                OpenAiCompatibleEmbedding {
                    index: 0,
                    embedding: vec![0.2],
                },
            ],
            2,
        )
        .expect_err("duplicate indices should fail");
        assert_eq!(
            duplicate,
            EmbeddingValidationError::DuplicateVectorIndex { index: 0 }
        );

        let missing = order_openai_compatible_embeddings(
            vec![OpenAiCompatibleEmbedding {
                index: 1,
                embedding: vec![0.1],
            }],
            2,
        )
        .expect_err("missing indices should fail");
        assert_eq!(
            missing,
            EmbeddingValidationError::MissingVectorIndex { index: 0 }
        );

        let out_of_range = order_openai_compatible_embeddings(
            vec![OpenAiCompatibleEmbedding {
                index: 2,
                embedding: vec![0.1],
            }],
            2,
        )
        .expect_err("out-of-range indices should fail");
        assert_eq!(
            out_of_range,
            EmbeddingValidationError::VectorIndexOutOfRange {
                index: 2,
                expected_count: 2,
            }
        );
    }

    #[test]
    fn accepts_a_valid_embedding_batch() {
        let batch = EmbeddingBatch::try_from_raw(vec![vec![0.1, 0.2, 0.3], vec![0.4, 0.5, 0.6]], 2)
            .expect("valid vectors should be accepted");

        assert_eq!(batch.len(), 2);
        assert_eq!(batch.dimensions(), 3);
        assert_eq!(batch.vectors()[0].values(), &[0.1, 0.2, 0.3]);
        assert_eq!(batch.vectors()[1].dimensions(), 3);
    }

    #[test]
    fn accepts_an_empty_batch_for_empty_input() {
        let batch = EmbeddingBatch::try_from_raw(Vec::new(), 0)
            .expect("empty input should allow an empty batch");

        assert!(batch.is_empty());
        assert_eq!(batch.dimensions(), 0);
    }

    #[test]
    fn rejects_a_vector_count_mismatch() {
        let error = EmbeddingBatch::try_from_raw(vec![vec![0.1, 0.2]], 2)
            .expect_err("one response vector cannot satisfy two inputs");

        assert_eq!(
            error,
            EmbeddingValidationError::VectorCountMismatch {
                expected: 2,
                actual: 1,
            }
        );
    }

    #[test]
    fn rejects_empty_vectors() {
        let error = EmbeddingBatch::try_from_raw(vec![vec![0.1], Vec::new()], 2)
            .expect_err("empty vectors must be rejected");

        assert_eq!(
            error,
            EmbeddingValidationError::EmptyVector { vector_index: 1 }
        );
    }

    #[test]
    fn rejects_inconsistent_dimensions() {
        let error = EmbeddingBatch::try_from_raw(vec![vec![0.1, 0.2], vec![0.3]], 2)
            .expect_err("all vectors in a batch need the same dimensions");

        assert_eq!(
            error,
            EmbeddingValidationError::DimensionMismatch {
                vector_index: 1,
                expected: 2,
                actual: 1,
            }
        );
    }

    #[test]
    fn rejects_non_finite_values() {
        for invalid_value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let error = EmbeddingBatch::try_from_raw(vec![vec![0.1, invalid_value, 0.3]], 1)
                .expect_err("non-finite embedding values must be rejected");

            assert_eq!(
                error,
                EmbeddingValidationError::NonFiniteValue {
                    vector_index: 0,
                    value_index: 1,
                }
            );
        }
    }

    #[test]
    fn validation_errors_have_actionable_messages() {
        let error = EmbeddingValidationError::DimensionMismatch {
            vector_index: 2,
            expected: 768,
            actual: 384,
        };

        assert_eq!(
            error.to_string(),
            "Embedding vector 2 has 384 dimensions; expected 768"
        );
    }
}
