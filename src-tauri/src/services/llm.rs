use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::fmt;
use std::sync::{OnceLock, RwLock};
use std::time::Duration;

use reqwest::Client;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::time::sleep;

pub mod default_generation;
pub mod default_generation_schema;
pub use default_generation_schema::{LlmSchemaError, StageBMcq};

const DEFAULT_OLLAMA_BASE_URL: &str = "http://127.0.0.1:11434";
const DEFAULT_OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
const DEFAULT_TIMEOUT_SECS: u64 = 60;
const GENERATION_MAX_ATTEMPTS: usize = 4;

static RUNTIME_LLM_CONFIG: OnceLock<RwLock<Option<LlmConfig>>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmProvider {
    Ollama,
    OpenRouter,
}

impl LlmProvider {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Ollama => "ollama",
            Self::OpenRouter => "openrouter",
        }
    }

    fn from_str(raw: &str) -> Result<Self, LlmConfigError> {
        match raw.trim().to_lowercase().as_str() {
            "ollama" => Ok(Self::Ollama),
            "openrouter" => Ok(Self::OpenRouter),
            _ => Err(LlmConfigError::InvalidValue {
                key: String::from("LLM_PROVIDER"),
                value: raw.to_string(),
                reason: String::from("expected 'ollama' or 'openrouter'"),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ProviderProfile {
    default_base_url: &'static str,
    chat_path: &'static str,
    bypass_proxy: bool,
}

fn provider_registry() -> HashMap<&'static str, ProviderProfile> {
    HashMap::from([
        (
            "ollama",
            ProviderProfile {
                default_base_url: DEFAULT_OLLAMA_BASE_URL,
                chat_path: "/api/chat",
                bypass_proxy: true,
            },
        ),
        (
            "openrouter",
            ProviderProfile {
                default_base_url: DEFAULT_OPENROUTER_BASE_URL,
                chat_path: "/chat/completions",
                bypass_proxy: false,
            },
        ),
    ])
}

fn provider_profile(provider: LlmProvider) -> ProviderProfile {
    let registry = provider_registry();
    registry
        .get(provider.as_str())
        .copied()
        .expect("provider profile should always exist")
}

#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub provider: LlmProvider,
    pub base_url: String,
    pub model: String,
    pub timeout_secs: u64,
    pub api_key: Option<String>,
}

impl LlmConfig {
    /// Builds config from environment variables for supported providers.
    ///
    /// Supported env vars:
    /// - LLM_PROVIDER: ollama | openrouter
    /// - LLM_BASE_URL: default base URL override (all providers)
    /// - OPENROUTER_BASE_URL: OpenRouter base URL override
    /// - LLM_MODEL: model id
    /// - OPENROUTER_API_KEY: required when LLM_PROVIDER=openrouter
    /// - LLM_TIMEOUT_SECS: request timeout in seconds
    pub fn from_env() -> Result<Self, LlmConfigError> {
        let provider = LlmProvider::from_str(
            &env::var("LLM_PROVIDER").map_err(|_| LlmConfigError::NotConfigured)?,
        )?;
        let profile = provider_profile(provider);

        let base_url = match provider {
            LlmProvider::Ollama => {
                env::var("LLM_BASE_URL").unwrap_or_else(|_| profile.default_base_url.to_string())
            }
            LlmProvider::OpenRouter => env::var("OPENROUTER_BASE_URL")
                .or_else(|_| env::var("LLM_BASE_URL"))
                .unwrap_or_else(|_| profile.default_base_url.to_string()),
        };
        let model = env::var("LLM_MODEL").map_err(|_| LlmConfigError::NotConfigured)?;
        if model.trim().is_empty() {
            return Err(LlmConfigError::NotConfigured);
        }
        let timeout_secs = parse_u64_env("LLM_TIMEOUT_SECS", DEFAULT_TIMEOUT_SECS)?;
        let api_key = match provider {
            LlmProvider::Ollama => None,
            LlmProvider::OpenRouter => {
                let key =
                    env::var("OPENROUTER_API_KEY").map_err(|_| LlmConfigError::InvalidValue {
                        key: String::from("OPENROUTER_API_KEY"),
                        value: String::new(),
                        reason: String::from("is required when LLM_PROVIDER=openrouter"),
                    })?;

                let normalized = key.trim().to_string();
                if normalized.is_empty() {
                    return Err(LlmConfigError::InvalidValue {
                        key: String::from("OPENROUTER_API_KEY"),
                        value: String::from("<empty>"),
                        reason: String::from("must be non-empty when LLM_PROVIDER=openrouter"),
                    });
                }

                Some(normalized)
            }
        };

        Ok(Self {
            provider,
            base_url,
            model,
            timeout_secs,
            api_key,
        })
    }

    pub fn chat_url(&self) -> String {
        let profile = provider_profile(self.provider);
        let base = self.base_url.trim_end_matches('/');
        format!("{base}{}", profile.chat_path)
    }
}

fn runtime_config_lock() -> &'static RwLock<Option<LlmConfig>> {
    RUNTIME_LLM_CONFIG.get_or_init(|| RwLock::new(None))
}

pub fn set_runtime_llm_config(
    provider: &str,
    base_url: &str,
    model: &str,
    timeout_secs: u64,
    api_key: Option<&str>,
) -> Result<(), LlmConfigError> {
    let parsed_provider = LlmProvider::from_str(provider)?;

    let normalized_base_url = base_url.trim();
    if normalized_base_url.is_empty() {
        return Err(LlmConfigError::InvalidValue {
            key: String::from("base_url"),
            value: String::from(base_url),
            reason: String::from("must be non-empty"),
        });
    }

    let normalized_model = model.trim();
    if normalized_model.is_empty() {
        return Err(LlmConfigError::InvalidValue {
            key: String::from("model"),
            value: String::from(model),
            reason: String::from("must be non-empty"),
        });
    }

    if timeout_secs == 0 {
        return Err(LlmConfigError::InvalidValue {
            key: String::from("timeout_secs"),
            value: timeout_secs.to_string(),
            reason: String::from("must be greater than 0"),
        });
    }

    let normalized_api_key = match parsed_provider {
        LlmProvider::Ollama => None,
        LlmProvider::OpenRouter => {
            let key = api_key
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or(LlmConfigError::InvalidValue {
                    key: String::from("api_key"),
                    value: String::from("<empty>"),
                    reason: String::from("is required when provider=openrouter"),
                })?;

            Some(key.to_string())
        }
    };

    let config = LlmConfig {
        provider: parsed_provider,
        base_url: normalized_base_url.to_string(),
        model: normalized_model.to_string(),
        timeout_secs,
        api_key: normalized_api_key,
    };

    let lock = runtime_config_lock();
    let mut guard = lock
        .write()
        .map_err(|_| LlmConfigError::RuntimeConfigPoisoned)?;
    *guard = Some(config);

    Ok(())
}

pub fn resolve_llm_config() -> Result<LlmConfig, LlmConfigError> {
    let lock = runtime_config_lock();
    let guard = lock
        .read()
        .map_err(|_| LlmConfigError::RuntimeConfigPoisoned)?;
    if let Some(config) = guard.as_ref() {
        return Ok(config.clone());
    }

    LlmConfig::from_env()
}

#[derive(Debug)]
pub struct LlmService {
    client: Client,
    config: LlmConfig,
}

pub(crate) struct JsonGenerationRequest<'a> {
    pub(crate) stage_label: &'a str,
    pub(crate) system_prompt: &'a str,
    pub(crate) user_prompt: &'a str,
    pub(crate) format_schema: Value,
    pub(crate) payload_preview_chars: usize,
}

#[derive(Debug, Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaChatMessage>,
    stream: bool,
    format: Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct OllamaChatResponse {
    message: Option<OllamaChatMessage>,
}

#[derive(Debug, Serialize)]
struct OpenRouterChatRequest {
    model: String,
    messages: Vec<OllamaChatMessage>,
    response_format: Value,
}

#[derive(Debug, Deserialize)]
struct OpenRouterChatResponse {
    choices: Vec<OpenRouterChatChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterChatChoice {
    message: Option<OpenRouterChatMessage>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterChatMessage {
    content: Value,
}

#[derive(Debug, Deserialize)]
struct OllamaTagsResponse {
    models: Vec<OllamaTagModel>,
}

#[derive(Debug, Deserialize)]
struct OllamaTagModel {
    name: String,
}

/// Provider-neutral details extracted from an LLM provider error response.
///
/// These details are for backend diagnostics. User-facing categorization and
/// messages are produced separately through [`LlmFailure`].
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

/// Top-level error response returned by OpenRouter.
#[derive(Debug, Deserialize)]
struct OpenRouterErrorEnvelope {
    error: OpenRouterErrorDetails,
}

/// Error details nested inside an OpenRouter error response.
#[derive(Debug, Deserialize)]
struct OpenRouterErrorDetails {
    code: Option<Value>,
    message: String,
}

/// Parses a provider-specific response body into normalized diagnostic details.
///
/// When support for another LLM provider is added, its error-envelope parser
/// must also be added to this dispatcher so it participates in error handling.
fn parse_provider_error(provider: LlmProvider, body: &str) -> Option<ProviderErrorDetails> {
    match provider {
        LlmProvider::Ollama => parse_ollama_error(body),
        LlmProvider::OpenRouter => parse_openrouter_error(body),
    }
}

/// Parses Ollama's `{ "error": "..." }` response format.
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

/// Parses OpenRouter's nested `{ "error": { "code", "message" } }` format.
fn parse_openrouter_error(body: &str) -> Option<ProviderErrorDetails> {
    let envelope: OpenRouterErrorEnvelope = serde_json::from_str(body).ok()?;
    let message = envelope.error.message.trim();
    if message.is_empty() {
        return None;
    }

    Some(ProviderErrorDetails {
        code: envelope.error.code.and_then(provider_error_code),
        message: message.to_string(),
    })
}

/// Converts a JSON string or number error code into a normalized string.
fn provider_error_code(value: Value) -> Option<String> {
    match value {
        Value::String(code) if !code.trim().is_empty() => Some(code),
        Value::Number(code) => Some(code.to_string()),
        _ => None,
    }
}

pub async fn fetch_ollama_models(
    base_url: &str,
    model_name: Option<&str>,
    timeout_secs: u64,
) -> Result<Vec<String>, LlmServiceError> {
    let normalized_base_url = base_url.trim().trim_end_matches('/');
    let endpoint = format!("{normalized_base_url}/api/tags");

    let client = Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(LlmServiceError::Http)?;

    let response =
        client
            .get(&endpoint)
            .send()
            .await
            .map_err(|source| LlmServiceError::Connect {
                url: endpoint.clone(),
                source,
            })?;

    let status = response.status();
    let body = response.text().await.map_err(LlmServiceError::Http)?;
    if !status.is_success() {
        return Err(classify_provider_error(LlmProvider::Ollama, status, &body));
    }

    let parsed: OllamaTagsResponse =
        serde_json::from_str(&body).map_err(LlmServiceError::ResponseDecode)?;

    let requested_model = model_name.unwrap_or("").trim().to_string();
    let normalized_filter = requested_model.to_lowercase();

    let all_models = parsed
        .models
        .iter()
        .map(|item| item.name.trim())
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();

    if !requested_model.is_empty() {
        let exact_match_exists = all_models
            .iter()
            .any(|item| item.eq_ignore_ascii_case(&requested_model));

        if !exact_match_exists {
            return Err(LlmServiceError::ModelNotFound {
                model: requested_model,
            });
        }
    }

    let mut models = parsed
        .models
        .into_iter()
        .map(|item| item.name.trim().to_string())
        .filter(|item| !item.is_empty())
        .filter(|item| {
            if normalized_filter.is_empty() {
                return true;
            }

            item.to_lowercase().contains(&normalized_filter)
        })
        .collect::<Vec<_>>();

    models.sort_by_key(|item| item.to_lowercase());
    models.dedup();

    Ok(models)
}

impl LlmService {
    pub fn new(config: LlmConfig) -> Result<Self, reqwest::Error> {
        let profile = provider_profile(config.provider);
        let mut builder = Client::builder().timeout(Duration::from_secs(config.timeout_secs));
        if profile.bypass_proxy {
            // Ollama runs locally; bypass system proxies to avoid localhost routing issues.
            builder = builder.no_proxy();
        }
        let client = builder.build()?;

        Ok(Self { client, config })
    }

    pub fn from_env() -> Result<Self, LlmConfigError> {
        let config = LlmConfig::from_env()?;
        log::info!(
            "LLM config loaded (provider={}, base_url={}, model={}, timeout_secs={})",
            config.provider.as_str(),
            config.base_url,
            config.model,
            config.timeout_secs
        );
        Self::new(config).map_err(LlmConfigError::HttpClientBuild)
    }

    pub fn from_runtime_or_env() -> Result<Self, LlmConfigError> {
        let config = resolve_llm_config()?;
        log::info!(
            "LLM config resolved (provider={}, base_url={}, model={}, timeout_secs={})",
            config.provider.as_str(),
            config.base_url,
            config.model,
            config.timeout_secs
        );
        Self::new(config).map_err(LlmConfigError::HttpClientBuild)
    }

    pub fn model(&self) -> &str {
        &self.config.model
    }

    pub fn chat_endpoint(&self) -> String {
        self.config.chat_url()
    }

    pub(crate) async fn generate_json_with_retries<T, Parse>(
        &self,
        request: JsonGenerationRequest<'_>,
        parse: Parse,
    ) -> Result<(T, String, usize), LlmServiceError>
    where
        Parse: Fn(&str) -> Result<T, LlmServiceError>,
    {
        for attempt in 1..=GENERATION_MAX_ATTEMPTS {
            let raw_json = match self
                .chat_json(
                    request.system_prompt,
                    request.user_prompt,
                    request.format_schema.clone(),
                )
                .await
            {
                Ok(value) => value,
                Err(err) => {
                    if attempt >= GENERATION_MAX_ATTEMPTS || !is_retryable_error(&err) {
                        return Err(err);
                    }

                    let delay = retry_delay(attempt);
                    log::warn!(
                        "LLM {} request failed on attempt {}/{}: {}. Retrying in {} ms",
                        request.stage_label,
                        attempt,
                        GENERATION_MAX_ATTEMPTS,
                        err,
                        delay.as_millis()
                    );
                    sleep(delay).await;
                    continue;
                }
            };

            match parse(&raw_json) {
                Ok(value) => return Ok((value, raw_json, attempt)),
                Err(err) => {
                    if attempt >= GENERATION_MAX_ATTEMPTS || !is_retryable_error(&err) {
                        return Err(err);
                    }

                    let delay = retry_delay(attempt);
                    log::warn!(
                        "LLM {} output parsing failed on attempt {}/{}: {} | payload_preview={}. Retrying in {} ms",
                        request.stage_label,
                        attempt,
                        GENERATION_MAX_ATTEMPTS,
                        err,
                        log_preview(&raw_json, request.payload_preview_chars),
                        delay.as_millis()
                    );
                    sleep(delay).await;
                }
            };
        }

        unreachable!("retry loop must return success or error")
    }

    pub(crate) async fn chat_json(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        format_schema: Value,
    ) -> Result<String, LlmServiceError> {
        match self.config.provider {
            LlmProvider::Ollama => {
                self.chat_json_ollama(system_prompt, user_prompt, format_schema)
                    .await
            }
            LlmProvider::OpenRouter => {
                self.chat_json_openrouter(system_prompt, user_prompt, format_schema)
                    .await
            }
        }
    }

    async fn chat_json_ollama(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        format_schema: Value,
    ) -> Result<String, LlmServiceError> {
        let endpoint = self.chat_endpoint();
        let payload = OllamaChatRequest {
            model: self.model().to_string(),
            messages: vec![
                OllamaChatMessage {
                    role: String::from("system"),
                    content: system_prompt.to_string(),
                },
                OllamaChatMessage {
                    role: String::from("user"),
                    content: user_prompt.to_string(),
                },
            ],
            stream: false,
            format: format_schema,
        };

        let response = self
            .client
            .post(&endpoint)
            .json(&payload)
            .send()
            .await
            .map_err(|source| LlmServiceError::Connect {
                url: endpoint.clone(),
                source,
            })?;

        let status = response.status();
        let body = response.text().await.map_err(LlmServiceError::Http)?;
        if !status.is_success() {
            return Err(classify_provider_error(LlmProvider::Ollama, status, &body));
        }

        let parsed: OllamaChatResponse =
            serde_json::from_str(&body).map_err(LlmServiceError::ResponseDecode)?;
        let content = parsed
            .message
            .map(|message| message.content)
            .unwrap_or_default()
            .trim()
            .to_string();

        if content.is_empty() {
            return Err(LlmServiceError::EmptyModelResponse);
        }

        // Strip accidental markdown fences before strict schema parsing.
        Ok(strip_markdown_fences(&content))
    }

    async fn chat_json_openrouter(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        format_schema: Value,
    ) -> Result<String, LlmServiceError> {
        let endpoint = self.chat_endpoint();
        let api_key = self
            .config
            .api_key
            .as_deref()
            .ok_or(LlmServiceError::MissingApiKey)?;

        let payload = OpenRouterChatRequest {
            model: self.model().to_string(),
            messages: vec![
                OllamaChatMessage {
                    role: String::from("system"),
                    content: system_prompt.to_string(),
                },
                OllamaChatMessage {
                    role: String::from("user"),
                    content: user_prompt.to_string(),
                },
            ],
            response_format: json!({
                "type": "json_schema",
                "json_schema": {
                    "name": "active_recall_output",
                    "strict": true,
                    "schema": format_schema
                }
            }),
        };

        let response = self
            .client
            .post(&endpoint)
            .bearer_auth(api_key)
            .json(&payload)
            .send()
            .await
            .map_err(|source| LlmServiceError::Connect {
                url: endpoint.clone(),
                source,
            })?;

        let status = response.status();
        let body = response.text().await.map_err(LlmServiceError::Http)?;
        if !status.is_success() {
            return Err(classify_provider_error(
                LlmProvider::OpenRouter,
                status,
                &body,
            ));
        }

        let parsed: OpenRouterChatResponse =
            serde_json::from_str(&body).map_err(LlmServiceError::ResponseDecode)?;
        let content = parsed
            .choices
            .into_iter()
            .find_map(|choice| choice.message)
            .map(|message| extract_text_content(&message.content))
            .unwrap_or_default()
            .trim()
            .to_string();

        if content.is_empty() {
            return Err(LlmServiceError::EmptyModelResponse);
        }

        Ok(strip_markdown_fences(&content))
    }
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

/// Builds instances of the user-facing LLM error contract.
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

/// Maps detailed configuration failures to the public error contract.
impl LlmConfigError {
    /// Converts a detailed configuration error into the stable user-facing contract.
    ///
    /// This deliberately hides sensitive or overly technical details while keeping
    /// the original `LlmConfigError` available to backend logs and diagnostics.
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
    MissingApiKey,
    Serialize(serde_json::Error),
    EmptyModelResponse,
    Schema(LlmSchemaError),
    InvalidOutput(String),
}

/// Formats detailed request and response errors for backend logs and diagnostics.
impl fmt::Display for LlmServiceError {
    /// Writes a developer-facing description of the LLM operation failure.
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
            Self::ResponseDecode(err) => write!(f, "Failed to decode LLM response envelope: {err}"),
            Self::ModelNotFound { model } => write!(
                f,
                "Model '{model}' was not found on this provider URL. Try fetching all models first."
            ),
            Self::MissingApiKey => write!(
                f,
                "Missing API key for provider request. Set OPENROUTER_API_KEY in your environment."
            ),
            Self::Serialize(err) => write!(f, "Failed to serialize LLM request context: {err}"),
            Self::EmptyModelResponse => write!(f, "LLM returned an empty response message"),
            Self::Schema(err) => write!(f, "{err}"),
            Self::InvalidOutput(err) => write!(f, "LLM returned invalid output: {err}"),
        }
    }
}

/// Enables `LlmServiceError` to participate in standard Rust error handling.
impl Error for LlmServiceError {}

/// Maps detailed service failures to the public error contract.
impl LlmServiceError {
    /// Converts an LLM operation error into a safe, actionable frontend failure.
    ///
    /// Transport failures are handled first because they occur before an HTTP
    /// response exists. HTTP responses are already classified with provider,
    /// status, and body context by [`classify_provider_error`].
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
            Self::MissingApiKey => LlmFailure::new(
                LlmFailureCode::Account,
                "The OpenRouter API key is missing. Add it in Models and save your settings.",
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
///
/// HTTP status supplies the broad code and retry behavior. When the provider
/// body contains a recognized error envelope, its message replaces the generic
/// status message after normalization and truncation. Unrecognized bodies use
/// the safe status-based fallback and are never retained or returned verbatim.
///
/// When adding another LLM provider, also add its envelope parser to
/// [`parse_provider_error`] and route every non-success response through here.
fn classify_provider_error(
    provider: LlmProvider,
    status: StatusCode,
    body: &str,
) -> LlmServiceError {
    // Transport errors never reach this helper; they are classified first in
    // `LlmServiceError::to_failure` because no response metadata exists.
    let mut failure = failure_from_http_status(status);
    let provider_code = parse_provider_error(provider, body).map(|details| {
        failure.message = bounded_provider_message(&details.message);
        details.code
    });
    let provider_code = provider_code.flatten();
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

/// Produces one compact provider message for logs, `Display`, and the frontend.
///
/// Only the parsed provider `message` reaches this function. Whitespace is
/// normalized and output is limited to 300 Unicode characters.
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

fn parse_u64_env(key: &str, default_value: u64) -> Result<u64, LlmConfigError> {
    match env::var(key) {
        Ok(value) => value
            .parse::<u64>()
            .map_err(|_| LlmConfigError::InvalidInteger {
                key: key.to_string(),
                value,
            }),
        Err(_) => Ok(default_value),
    }
}

fn retry_delay(attempt: usize) -> Duration {
    let base_ms: u64 = 300;
    let exponent = attempt.saturating_sub(1).min(10) as u32;
    Duration::from_millis(base_ms.saturating_mul(2u64.saturating_pow(exponent)))
}

fn is_retryable_error(err: &LlmServiceError) -> bool {
    err.to_failure().retryable
}

fn strip_markdown_fences(raw: &str) -> String {
    let trimmed = raw.trim();
    if !trimmed.starts_with("```") {
        return trimmed.to_string();
    }

    let mut lines = trimmed.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return String::new();
    }

    if lines
        .first()
        .is_some_and(|line| line.trim_start().starts_with("```"))
    {
        lines.remove(0);
    }
    if lines
        .last()
        .is_some_and(|line| line.trim_start().starts_with("```"))
    {
        lines.pop();
    }

    lines.join("\n").trim().to_string()
}

// Normalizes OpenRouter message content into plain text.
//
// OpenRouter content can be:
// - a raw string
// - an array of content blocks (where text is usually in {"text": ...})
// - an object with a "text" field
//
// Returning a single string keeps downstream JSON parsing provider-agnostic.
fn extract_text_content(content: &Value) -> String {
    match content {
        // Some models return a direct string payload.
        Value::String(text) => text.clone(),
        // Many responses use block arrays; concatenate detected text blocks.
        Value::Array(items) => items
            .iter()
            .filter_map(|item| match item {
                Value::Object(map) => map.get("text").and_then(Value::as_str).map(String::from),
                Value::String(text) => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        // Some responses wrap text in an object.
        Value::Object(map) => map
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        // Unknown shapes are treated as empty to trigger existing empty-response handling.
        _ => String::new(),
    }
}

fn log_preview(raw: &str, max_chars: usize) -> String {
    let pretty = match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(value) => serde_json::to_string_pretty(&value).unwrap_or_else(|_| raw.to_string()),
        Err(_) => raw.to_string(),
    };

    let total_chars = pretty.chars().count();
    if total_chars <= max_chars {
        return pretty;
    }

    let mut preview = pretty.chars().take(max_chars).collect::<String>();
    preview.push_str("\n...<truncated>");
    preview
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
    fn rejects_unrecognized_or_empty_provider_error_envelopes() {
        assert!(parse_provider_error(LlmProvider::Ollama, r#"{"error":"  "}"#).is_none());
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
        assert!(!error.to_string().contains(r#"{"error""#));
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
