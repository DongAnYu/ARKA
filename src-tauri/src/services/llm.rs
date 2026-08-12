use std::collections::HashMap;
use std::env;
use std::sync::{OnceLock, RwLock};
use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::time::sleep;

pub mod default_generation;
pub mod default_generation_schema;
mod error;
pub use default_generation_schema::{LlmSchemaError, StageBMcq};
pub use error::{LlmConfigError, LlmFailure, LlmFailureCode, LlmServiceError};

use error::{classify_provider_error, is_retryable_error};

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
