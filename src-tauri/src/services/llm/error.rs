use std::error::Error;
use std::fmt;

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{LlmProvider, LlmSchemaError};

/// Provider-neutral details extracted from an LLM provider error response.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProviderErrorDetails {
    code: Option<String>,
    message: String,
}

/// Error response returned by Ollama, including Ollama instances run in Docker.
#[derive(Debug, Deserialize)]
struct OllamaErrorEnvelope {
    error: String,
}

/// OpenAI-compatible error envelope returned by OpenAI and OpenRouter.
#[derive(Debug, Deserialize)]
struct OpenAiCompatibleErrorEnvelope {
    error: OpenAiCompatibleErrorDetails,
}

/// Error details nested inside an OpenAI-compatible error response.
#[derive(Debug, Deserialize)]
struct OpenAiCompatibleErrorDetails {
    code: Option<Value>,
    message: String,
}

/// Detailed errors produced while loading, validating, or applying LLM configuration.
///
/// These variants retain backend context for logging and debugging. Convert them
/// to [`LlmFailure`] before returning an error through the application boundary.
#[derive(Debug)]
pub enum LlmConfigError {
    /// No LLM provider and model have been configured.
    NotConfigured,
    /// A numeric configuration value could not be parsed.
    InvalidInteger { key: String, value: String },
    /// A configuration value failed validation.
    InvalidValue {
        key: String,
        value: String,
        reason: String,
    },
    /// The shared runtime configuration lock is unavailable.
    RuntimeConfigPoisoned,
    /// The HTTP client could not be created from the configuration.
    HttpClientBuild(reqwest::Error),
}

/// Formats the detailed configuration error for backend logs and diagnostics.
impl fmt::Display for LlmConfigError {
    /// Writes a developer-facing description of the configuration failure.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConfigured => write!(f, "No LLM provider and model have been configured"),
            Self::InvalidInteger { key, value } => {
                write!(f, "Invalid integer for {key}: {value}")
            }
            Self::InvalidValue { key, value, reason } => {
                write!(f, "Invalid value for {key} ('{value}'): {reason}")
            }
            Self::RuntimeConfigPoisoned => {
                write!(f, "Runtime LLM config state is unavailable")
            }
            Self::HttpClientBuild(err) => write!(f, "Failed to build HTTP client: {err}"),
        }
    }
}

/// Enables `LlmConfigError` to participate in standard Rust error handling.
impl Error for LlmConfigError {}

/// Stable, user-facing categories shared by backend commands and the frontend.
///
/// Multiple internal errors may map to one category so UI handling does not
/// depend on provider-specific or implementation-specific error details.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmFailureCode {
    /// Model settings are missing, invalid, or reference an unavailable model.
    Setup,
    /// Provider credentials, credits, or account permissions require attention.
    Account,
    /// The configured provider could not be reached or the connection failed.
    Connection,
    /// The provider temporarily rejected requests because a rate limit was reached.
    RateLimited,
    /// The provider or upstream model is temporarily unavailable.
    ProviderUnavailable,
    /// The provider rejected the request parameters, content, or payload size.
    RequestRejected,
    /// The model returned empty, malformed, or schema-incompatible output.
    InvalidResponse,
    /// The failure does not match a known actionable category.
    Unknown,
}

/// Serializable error contract returned to callers when an LLM operation fails.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LlmFailure {
    /// Stable category used by the frontend to choose its response.
    pub code: LlmFailureCode,
    /// Safe, actionable message that may be shown to the user.
    pub message: String,
    /// Whether retrying the same operation may succeed without changing settings.
    pub retryable: bool,
    /// Provider-supplied retry delay when one is available.
    pub retry_after_secs: Option<u64>,
}

impl LlmFailure {
    /// Creates a failure without a provider-specific retry delay.
    fn new(code: LlmFailureCode, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code,
            message: message.into(),
            retryable,
            retry_after_secs: None,
        }
    }
}

impl LlmConfigError {
    /// Converts a detailed configuration error into the stable user-facing contract.
    pub fn to_failure(&self) -> LlmFailure {
        match self {
            Self::NotConfigured => LlmFailure::new(
                LlmFailureCode::Setup,
                "Choose an LLM provider and model in Models, then save your settings before generating questions.",
                false,
            ),
            Self::InvalidInteger { .. } | Self::InvalidValue { .. } => LlmFailure::new(
                LlmFailureCode::Setup,
                "The saved model settings are invalid. Review and save them again in Models.",
                false,
            ),
            Self::RuntimeConfigPoisoned | Self::HttpClientBuild(_) => LlmFailure::new(
                LlmFailureCode::Unknown,
                "ARKA could not initialize the LLM client. Restart the app and try again.",
                false,
            ),
        }
    }
}

/// Detailed errors produced while sending an LLM request or processing its response.
#[derive(Debug)]
pub enum LlmServiceError {
    Connect { url: String, source: reqwest::Error },
    Http(reqwest::Error),
    HttpStatus { failure: LlmFailure },
    ResponseDecode(serde_json::Error),
    ModelNotFound { model: String },
    MissingApiKey { provider: LlmProvider },
    Serialize(serde_json::Error),
    EmptyModelResponse,
    Schema(LlmSchemaError),
    InvalidOutput(String),
}

impl fmt::Display for LlmServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect { url, source } => {
                write!(
                    f,
                    "Cannot reach LLM endpoint at {url}: {source}. Ensure the provider endpoint is reachable and credentials are valid."
                )
            }
            Self::Http(err) => write!(f, "LLM request failed: {err}"),
            Self::HttpStatus { failure, .. } => write!(f, "{}", failure.message),
            Self::ResponseDecode(err) => {
                write!(f, "Failed to decode LLM response envelope: {err}")
            }
            Self::ModelNotFound { model } => write!(
                f,
                "Model '{model}' was not found on this provider URL. Try fetching all models first."
            ),
            Self::MissingApiKey { provider } => write!(
                f,
                "Missing API key for {} request. Add it in Models or configure the provider environment variable.",
                provider.as_str()
            ),
            Self::Serialize(err) => write!(f, "Failed to serialize LLM request context: {err}"),
            Self::EmptyModelResponse => write!(f, "LLM returned an empty response message"),
            Self::Schema(err) => write!(f, "{err}"),
            Self::InvalidOutput(err) => write!(f, "LLM returned invalid output: {err}"),
        }
    }
}

impl Error for LlmServiceError {}

impl LlmServiceError {
    /// Converts an LLM operation error into a safe, actionable frontend failure.
    pub fn to_failure(&self) -> LlmFailure {
        match self {
            Self::Connect { .. } => LlmFailure::new(
                LlmFailureCode::Connection,
                "ARKA could not reach the configured LLM provider. Check the provider URL and your network connection.",
                true,
            ),
            Self::Http(source) if source.is_timeout() || source.is_connect() => LlmFailure::new(
                LlmFailureCode::Connection,
                "The LLM connection failed or timed out. Check the provider and try again.",
                true,
            ),
            Self::Http(_) => LlmFailure::new(
                LlmFailureCode::Unknown,
                "The LLM request failed unexpectedly. Try again.",
                false,
            ),
            Self::HttpStatus { failure, .. } => failure.clone(),
            Self::ModelNotFound { .. } => LlmFailure::new(
                LlmFailureCode::Setup,
                "The selected model was not found. Choose an available model in Models.",
                false,
            ),
            Self::MissingApiKey { .. } => LlmFailure::new(
                LlmFailureCode::Account,
                "The provider API key is missing. Add it in Models and save your settings.",
                false,
            ),
            Self::ResponseDecode(_)
            | Self::EmptyModelResponse
            | Self::Schema(_)
            | Self::InvalidOutput(_) => LlmFailure::new(
                LlmFailureCode::InvalidResponse,
                "The model did not return a valid response. Try again or choose another model.",
                true,
            ),
            Self::Serialize(_) => LlmFailure::new(
                LlmFailureCode::Unknown,
                "ARKA could not prepare the LLM request.",
                false,
            ),
        }
    }
}

/// Centrally classifies a non-success response from any supported LLM provider.
pub(super) fn classify_provider_error(
    provider: LlmProvider,
    status: StatusCode,
    body: &str,
) -> LlmServiceError {
    let mut failure = failure_from_http_status(status);
    let provider_code = parse_provider_error(provider, body).and_then(|details| {
        failure.message = bounded_provider_message(&details.message);
        details.code
    });
    failure.message = format!(
        "LLM provider request failed (provider={}, status={}, code={}): {}",
        provider.as_str(),
        status,
        provider_code.as_deref().unwrap_or("unknown"),
        failure.message,
    );

    log::warn!("{}", failure.message);
    LlmServiceError::HttpStatus { failure }
}

/// Returns whether the same request may succeed when attempted again.
pub(super) fn is_retryable_error(error: &LlmServiceError) -> bool {
    error.to_failure().retryable
}

/// Parses a provider-specific response body into normalized diagnostic details.
fn parse_provider_error(provider: LlmProvider, body: &str) -> Option<ProviderErrorDetails> {
    match provider {
        LlmProvider::Ollama => parse_ollama_error(body),
        LlmProvider::OpenAi | LlmProvider::OpenRouter => parse_openai_compatible_error(body),
    }
}

fn parse_ollama_error(body: &str) -> Option<ProviderErrorDetails> {
    let envelope: OllamaErrorEnvelope = serde_json::from_str(body).ok()?;
    let message = envelope.error.trim();
    if message.is_empty() {
        return None;
    }

    Some(ProviderErrorDetails {
        code: None,
        message: message.to_string(),
    })
}

fn parse_openai_compatible_error(body: &str) -> Option<ProviderErrorDetails> {
    let envelope: OpenAiCompatibleErrorEnvelope = serde_json::from_str(body).ok()?;
    let message = envelope.error.message.trim();
    if message.is_empty() {
        return None;
    }

    Some(ProviderErrorDetails {
        code: envelope.error.code.and_then(provider_error_code),
        message: message.to_string(),
    })
}

fn provider_error_code(value: Value) -> Option<String> {
    match value {
        Value::String(code) if !code.trim().is_empty() => Some(code),
        Value::Number(code) => Some(code.to_string()),
        _ => None,
    }
}

/// Produces one compact provider message for logs, `Display`, and the frontend.
fn bounded_provider_message(message: &str) -> String {
    const MAX_CHARS: usize = 300;
    const ELLIPSIS: &str = "...";

    let normalized = message.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= MAX_CHARS {
        return normalized;
    }

    let kept_chars = MAX_CHARS.saturating_sub(ELLIPSIS.chars().count());
    let mut truncated = normalized.chars().take(kept_chars).collect::<String>();
    truncated.push_str(ELLIPSIS);
    truncated
}

/// Maps an HTTP response status to its user-facing failure category and message.
fn failure_from_http_status(status: StatusCode) -> LlmFailure {
    match status {
        StatusCode::UNAUTHORIZED => LlmFailure::new(
            LlmFailureCode::Account,
            "The LLM provider rejected the API key. Check the key in Models.",
            false,
        ),
        StatusCode::PAYMENT_REQUIRED => LlmFailure::new(
            LlmFailureCode::Account,
            "The LLM account has insufficient credits. Add credits or choose another provider.",
            false,
        ),
        StatusCode::FORBIDDEN => LlmFailure::new(
            LlmFailureCode::Account,
            "The LLM provider denied this request. Check the API key permissions and provider account.",
            false,
        ),
        StatusCode::NOT_FOUND => LlmFailure::new(
            LlmFailureCode::Setup,
            "The selected model or provider endpoint was not found. Review the settings in Models.",
            false,
        ),
        StatusCode::TOO_MANY_REQUESTS => LlmFailure::new(
            LlmFailureCode::RateLimited,
            "The LLM provider is rate limiting requests. Wait briefly and try again.",
            true,
        ),
        StatusCode::INTERNAL_SERVER_ERROR
        | StatusCode::BAD_GATEWAY
        | StatusCode::SERVICE_UNAVAILABLE
        | StatusCode::GATEWAY_TIMEOUT => LlmFailure::new(
            LlmFailureCode::ProviderUnavailable,
            "The LLM provider is temporarily unavailable. Try again shortly.",
            true,
        ),
        StatusCode::BAD_REQUEST
        | StatusCode::PAYLOAD_TOO_LARGE
        | StatusCode::UNPROCESSABLE_ENTITY => LlmFailure::new(
            LlmFailureCode::RequestRejected,
            "The LLM provider rejected the request. Try a smaller note or choose another model.",
            false,
        ),
        _ => LlmFailure::new(
            LlmFailureCode::Unknown,
            "The LLM provider returned an unexpected error. Try again.",
            false,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ollama_error_envelope() {
        let details = parse_provider_error(
            LlmProvider::Ollama,
            r#"{"error":"model 'qwen2.5' not found"}"#,
        )
        .expect("Ollama error response should parse");

        assert_eq!(details.code, None);
        assert_eq!(details.message, "model 'qwen2.5' not found");
    }

    #[test]
    fn parses_openrouter_error_envelope() {
        let details = parse_provider_error(
            LlmProvider::OpenRouter,
            r#"{
                "error": {
                    "code": 429,
                    "message": "Rate limit exceeded",
                    "metadata": { "provider_name": "Example" }
                }
            }"#,
        )
        .expect("OpenRouter error response should parse");

        assert_eq!(details.code.as_deref(), Some("429"));
        assert_eq!(details.message, "Rate limit exceeded");
    }

    #[test]
    fn parses_openai_error_envelope() {
        let details = parse_provider_error(
            LlmProvider::OpenAi,
            r#"{
                "error": {
                    "code": "invalid_request_error",
                    "message": "Invalid response format"
                }
            }"#,
        )
        .expect("OpenAI error response should parse");

        assert_eq!(details.code.as_deref(), Some("invalid_request_error"));
        assert_eq!(details.message, "Invalid response format");
    }

    #[test]
    fn rejects_unrecognized_or_empty_provider_error_envelopes() {
        assert!(parse_provider_error(LlmProvider::Ollama, r#"{"error":"  "}"#).is_none());
        assert!(parse_provider_error(LlmProvider::OpenAi, "not-json").is_none());
        assert!(parse_provider_error(LlmProvider::OpenRouter, "not-json").is_none());
    }

    #[test]
    fn central_classifier_uses_http_status_when_provider_body_is_unknown() {
        let error = classify_provider_error(
            LlmProvider::OpenRouter,
            StatusCode::UNAUTHORIZED,
            "not-json",
        );
        let failure = error.to_failure();

        assert_eq!(failure.code, LlmFailureCode::Account);
        assert!(!failure.retryable);
        assert_eq!(
            failure.message,
            "LLM provider request failed (provider=openrouter, status=401 Unauthorized, code=unknown): The LLM provider rejected the API key. Check the key in Models."
        );
    }

    #[test]
    fn central_classifier_returns_parsed_openrouter_message() {
        let error = classify_provider_error(
            LlmProvider::OpenRouter,
            StatusCode::BAD_REQUEST,
            r#"{"error":{"code":429,"message":"  Request   is invalid  "}}"#,
        );
        let failure = error.to_failure();

        assert_eq!(failure.code, LlmFailureCode::RequestRejected);
        assert!(!failure.retryable);
        assert_eq!(
            failure.message,
            "LLM provider request failed (provider=openrouter, status=400 Bad Request, code=429): Request is invalid"
        );
        assert_eq!(error.to_string(), failure.message);
        assert!(!error.to_string().contains(r#"{"error"#));
    }

    #[test]
    fn central_classifier_returns_parsed_ollama_message() {
        let error = classify_provider_error(
            LlmProvider::Ollama,
            StatusCode::BAD_REQUEST,
            r#"{"error":"model 'private-model-name' not found"}"#,
        );
        let failure = error.to_failure();

        assert_eq!(failure.code, LlmFailureCode::RequestRejected);
        assert!(!failure.retryable);
        assert_eq!(
            failure.message,
            "LLM provider request failed (provider=ollama, status=400 Bad Request, code=unknown): model 'private-model-name' not found"
        );
    }

    #[test]
    fn bounded_provider_message_is_limited_to_300_characters() {
        let message = "a".repeat(400);
        let truncated = bounded_provider_message(&message);

        assert_eq!(truncated.chars().count(), 300);
        assert!(truncated.ends_with("..."));
    }

    #[test]
    fn classifies_supported_http_statuses_into_the_contract() {
        let cases = [
            (StatusCode::UNAUTHORIZED, LlmFailureCode::Account, false),
            (StatusCode::PAYMENT_REQUIRED, LlmFailureCode::Account, false),
            (StatusCode::FORBIDDEN, LlmFailureCode::Account, false),
            (StatusCode::NOT_FOUND, LlmFailureCode::Setup, false),
            (
                StatusCode::TOO_MANY_REQUESTS,
                LlmFailureCode::RateLimited,
                true,
            ),
            (
                StatusCode::BAD_GATEWAY,
                LlmFailureCode::ProviderUnavailable,
                true,
            ),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                LlmFailureCode::ProviderUnavailable,
                true,
            ),
            (
                StatusCode::BAD_REQUEST,
                LlmFailureCode::RequestRejected,
                false,
            ),
        ];

        for (status, expected_code, expected_retryable) in cases {
            let failure = failure_from_http_status(status);
            assert_eq!(failure.code, expected_code, "status={status}");
            assert_eq!(failure.retryable, expected_retryable, "status={status}");
            assert_eq!(failure.retry_after_secs, None, "status={status}");
            assert!(!failure.message.trim().is_empty(), "status={status}");
        }
    }

    #[test]
    fn classifies_unhandled_http_status_as_unknown() {
        let failure = failure_from_http_status(StatusCode::IM_A_TEAPOT);

        assert_eq!(failure.code, LlmFailureCode::Unknown);
        assert!(!failure.retryable);
    }

    #[test]
    fn classifies_missing_configuration_as_setup_failure() {
        let failure = LlmConfigError::NotConfigured.to_failure();

        assert_eq!(failure.code, LlmFailureCode::Setup);
        assert!(!failure.retryable);
        assert!(failure.message.contains("Models"));
    }

    #[test]
    fn serializes_failure_codes_as_stable_snake_case_values() {
        let failure = LlmFailure::new(
            LlmFailureCode::RateLimited,
            "Wait before trying again.",
            true,
        );
        let json = serde_json::to_value(failure).expect("failure contract should serialize");

        assert_eq!(json["code"], "rate_limited");
        assert_eq!(json["retryable"], true);
        assert!(json["retry_after_secs"].is_null());
    }
}
