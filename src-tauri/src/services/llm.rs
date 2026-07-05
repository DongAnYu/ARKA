use std::env;
use std::error::Error;
use std::fmt;
use std::future::Future;
use std::collections::HashSet;
use std::sync::{OnceLock, RwLock};
use std::time::Duration;

use reqwest::Client;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::time::sleep;

const DEFAULT_OLLAMA_BASE_URL: &str = "http://127.0.0.1:11434";
const DEFAULT_MODEL: &str = "qwen3:4b";
const DEFAULT_TIMEOUT_SECS: u64 = 60;
const GENERATION_MAX_ATTEMPTS: usize = 3;

static RUNTIME_LLM_CONFIG: OnceLock<RwLock<Option<LlmConfig>>> = OnceLock::new();

#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub base_url: String,
    pub model: String,
    pub timeout_secs: u64,
}

impl LlmConfig {
    /// Builds config from environment variables for Ollama.
    ///
    /// Supported env vars:
    /// - LLM_BASE_URL: e.g. http://127.0.0.1:11434
    /// - LLM_MODEL: e.g. qwen3:4b
    /// - LLM_TIMEOUT_SECS: request timeout in seconds
    pub fn from_env() -> Result<Self, LlmConfigError> {
        let base_url = env::var("LLM_BASE_URL").unwrap_or_else(|_| DEFAULT_OLLAMA_BASE_URL.to_string());
        let model = env::var("LLM_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
        let timeout_secs = parse_u64_env("LLM_TIMEOUT_SECS", DEFAULT_TIMEOUT_SECS)?;

        Ok(Self {
            base_url,
            model,
            timeout_secs,
        })
    }

    pub fn chat_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        format!("{base}/api/chat")
    }
}

fn runtime_config_lock() -> &'static RwLock<Option<LlmConfig>> {
    RUNTIME_LLM_CONFIG.get_or_init(|| RwLock::new(None))
}

pub fn set_runtime_llm_config(
    base_url: &str,
    model: &str,
    timeout_secs: u64,
) -> Result<(), LlmConfigError> {
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

    let config = LlmConfig {
        base_url: normalized_base_url.to_string(),
        model: normalized_model.to_string(),
        timeout_secs,
    };

    let lock = runtime_config_lock();
    let mut guard = lock.write().map_err(|_| LlmConfigError::RuntimeConfigPoisoned)?;
    *guard = Some(config);

    Ok(())
}

pub fn resolve_llm_config() -> Result<LlmConfig, LlmConfigError> {
    let lock = runtime_config_lock();
    let guard = lock.read().map_err(|_| LlmConfigError::RuntimeConfigPoisoned)?;
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

    let response = client
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
        return Err(LlmServiceError::HttpStatus { status, body });
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

/// Wrapper for Stage A output so parsing is deterministic.
///
/// Expected JSON shape:
/// {
///   "key_points": [
///     { "knowledge_point": "..." }
///   ]
/// }
#[derive(Debug, Clone, Deserialize)]
pub struct StageAKeyPointsOutput {
    pub key_points: Vec<StageAKeyPoint>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StageAKeyPoint {
    pub knowledge_point: String,
}

/// Wrapper for Stage B output so parsing is deterministic.
///
/// Expected JSON shape:
/// {
///   "questions": [
///     {
///       "question": "...",
///       "option_a": "...",
///       "option_b": "...",
///       "option_c": "...",
///       "option_d": "...",
///       "correct_answer": "A",
///       "explanation": "..."
///     }
///   ]
/// }
#[derive(Debug, Clone, Deserialize)]
pub struct StageBMcqOutput {
    pub questions: Vec<StageBMcq>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StageBMcq {
    pub question: String,
    pub option_a: String,
    pub option_b: String,
    pub option_c: String,
    pub option_d: String,
    pub correct_answer: String,
    pub explanation: String,
}

#[derive(Debug)]
pub enum LlmSchemaError {
    Parse(serde_json::Error),
    Validation(LlmValidationError),
}

#[derive(Debug)]
pub enum LlmValidationError {
    EmptyKnowledgePoint { index: usize },
    EmptyQuestions,
    EmptyField { index: usize, field: &'static str },
    DuplicateOptions { index: usize },
    DuplicateQuestion { index: usize },
    InvalidCorrectAnswer { index: usize, value: String },
}

impl fmt::Display for LlmSchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(err) => write!(f, "Invalid LLM JSON payload: {err}"),
            Self::Validation(err) => write!(f, "LLM JSON validation failed: {err}"),
        }
    }
}

impl Error for LlmSchemaError {}

impl fmt::Display for LlmValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyKnowledgePoint { index } => {
                write!(f, "knowledge_point at index {index} must be non-empty")
            }
            Self::EmptyQuestions => write!(f, "questions must contain at least one item"),
            Self::EmptyField { index, field } => {
                write!(f, "field '{field}' at question index {index} must be non-empty")
            }
            Self::DuplicateOptions { index } => {
                write!(f, "question at index {index} has duplicate options")
            }
            Self::DuplicateQuestion { index } => {
                write!(f, "question at index {index} duplicates a previous question")
            }
            Self::InvalidCorrectAnswer { index, value } => write!(
                f,
                "question at index {index} has invalid correct_answer '{value}' (expected A/B/C/D)"
            ),
        }
    }
}

/// Parses Stage A LLM output using a deterministic wrapper object.
pub fn parse_stage_a_output(json_payload: &str) -> Result<StageAKeyPointsOutput, LlmSchemaError> {
    let parsed: StageAKeyPointsOutput = serde_json::from_str(json_payload).map_err(LlmSchemaError::Parse)?;
    validate_stage_a_output(&parsed)?;
    Ok(parsed)
}

/// Parses Stage B LLM output using a deterministic wrapper object.
pub fn parse_stage_b_output(json_payload: &str) -> Result<StageBMcqOutput, LlmSchemaError> {
    let parsed: StageBMcqOutput = serde_json::from_str(json_payload).map_err(LlmSchemaError::Parse)?;
    validate_stage_b_output(&parsed)?;
    Ok(parsed)
}

fn validate_stage_a_output(parsed: &StageAKeyPointsOutput) -> Result<(), LlmSchemaError> {
    for (index, item) in parsed.key_points.iter().enumerate() {
        if item.knowledge_point.trim().is_empty() {
            return Err(LlmSchemaError::Validation(
                LlmValidationError::EmptyKnowledgePoint { index },
            ));
        }
    }

    Ok(())
}

fn validate_stage_b_output(parsed: &StageBMcqOutput) -> Result<(), LlmSchemaError> {
    if parsed.questions.is_empty() {
        return Err(LlmSchemaError::Validation(LlmValidationError::EmptyQuestions));
    }

    let mut seen_questions = HashSet::new();

    for (index, question) in parsed.questions.iter().enumerate() {
        validate_non_empty(index, "question", &question.question)?;
        validate_non_empty(index, "option_a", &question.option_a)?;
        validate_non_empty(index, "option_b", &question.option_b)?;
        validate_non_empty(index, "option_c", &question.option_c)?;
        validate_non_empty(index, "option_d", &question.option_d)?;
        validate_non_empty(index, "correct_answer", &question.correct_answer)?;
        validate_non_empty(index, "explanation", &question.explanation)?;

        let answer = question.correct_answer.trim();
        if answer != "A" && answer != "B" && answer != "C" && answer != "D" {
            return Err(LlmSchemaError::Validation(
                LlmValidationError::InvalidCorrectAnswer {
                    index,
                    value: question.correct_answer.clone(),
                },
            ));
        }

        let a = question.option_a.trim();
        let b = question.option_b.trim();
        let c = question.option_c.trim();
        let d = question.option_d.trim();
        if a == b || a == c || a == d || b == c || b == d || c == d {
            return Err(LlmSchemaError::Validation(
                LlmValidationError::DuplicateOptions { index },
            ));
        }

        let question_key = normalize_for_dedup(&question.question);
        if !seen_questions.insert(question_key) {
            return Err(LlmSchemaError::Validation(
                LlmValidationError::DuplicateQuestion { index },
            ));
        }
    }

    Ok(())
}

fn validate_non_empty(index: usize, field: &'static str, value: &str) -> Result<(), LlmSchemaError> {
    if value.trim().is_empty() {
        return Err(LlmSchemaError::Validation(LlmValidationError::EmptyField {
            index,
            field,
        }));
    }

    Ok(())
}

impl LlmService {
    pub fn new(config: LlmConfig) -> Result<Self, reqwest::Error> {
        let client = Client::builder()
            // Ollama runs locally; bypass system proxies to avoid localhost routing issues.
            .no_proxy()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()?;

        Ok(Self { client, config })
    }

    pub fn from_env() -> Result<Self, LlmConfigError> {
        let config = LlmConfig::from_env()?;
        log::info!(
            "LLM config loaded (base_url={}, model={}, timeout_secs={})",
            config.base_url,
            config.model,
            config.timeout_secs
        );
        Self::new(config).map_err(LlmConfigError::HttpClientBuild)
    }

    pub fn from_runtime_or_env() -> Result<Self, LlmConfigError> {
        let config = resolve_llm_config()?;
        log::info!(
            "LLM config resolved (base_url={}, model={}, timeout_secs={})",
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

    pub async fn generate_stage_a_key_points(
        &self,
        chunk_markdown: &str,
    ) -> Result<StageAKeyPointsOutput, LlmServiceError> {
        log::info!(
            "LLM Stage A generation started (chunk_chars={})",
            chunk_markdown.chars().count()
        );
        let system_prompt = "You are generating knowledge extraction output for active recall. Return strict JSON only, no markdown, no prose.";
        let user_prompt = format!(
            concat!(
                "Extract concrete knowledge points from this markdown chunk for study review. ",
                "Only include points that represent stable, testable knowledge. ",
                "Ignore navigation text, filler, personal journaling, and weak context that does not stand on its own. ",
                "If the chunk does not contain enough real knowledge to study, return an empty array. ",
                "Return exactly this JSON shape and nothing else: {{\"key_points\":[{{\"knowledge_point\":\"...\"}}]}}. ",
                "Examples: {{\"key_points\":[]}} or {{\"key_points\":[{{\"knowledge_point\":\"...\"}}]}}.\n\n",
                "Chunk:\n{}"
            ),
            chunk_markdown,
        );

        let (parsed, attempts) = self
            .retry_with_backoff("Stage A", |attempt| {
                let user_prompt = user_prompt.clone();
                async move {
                let json_payload = self
                    .chat_json(system_prompt, &user_prompt, stage_a_format_schema())
                    .await?;

                parse_stage_a_output(&json_payload).map_err(|err| {
                    log::warn!(
                        "LLM Stage A schema validation failed on attempt {}/{}: {} | payload_preview={}",
                        attempt,
                        GENERATION_MAX_ATTEMPTS,
                        err,
                        log_preview(&json_payload, 600)
                    );
                    LlmServiceError::Schema(err)
                })
                }
            })
            .await?;

        log::info!(
            "LLM Stage A generation finished (key_points={}, attempts={})",
            parsed.key_points.len(),
            attempts
        );
        Ok(parsed)
    }

    pub async fn generate_stage_b_mcqs(
        &self,
        chunk_markdown: &str,
        key_points: &[String],
    ) -> Result<StageBMcqOutput, LlmServiceError> {
        log::info!(
            "LLM Stage B generation started (chunk_chars={}, key_points={})",
            chunk_markdown.chars().count(),
            key_points.len()
        );
        let key_points_json = serde_json::to_string(key_points).map_err(LlmServiceError::Serialize)?;
        let system_prompt = "You are generating multiple-choice questions for active recall. Return strict JSON only, no markdown, no prose.";
        let user_prompt = format!(
            concat!(
                "Given this markdown chunk and extracted key points, create 1-4 MCQs for active recall. ",
                "Each question must test a different concept and must not paraphrase another question. ",
                "Use at most one question per key point and prioritize the strongest distinct concepts. ",
                "If concepts overlap, generate fewer questions instead of duplicates. ",
                "All questions must be grounded in the chunk content. ",
                "Each question must have exactly four options (A-D), exactly one correct answer, and no duplicate options. ",
                "Avoid 'all of the above', 'none of the above', and trick wording. ",
                "Do not always use A as correct; distribute correct answers across A/B/C/D when reasonable. ",
                "Before returning, self-check for schema compliance, uniqueness, and non-empty fields. ",
                "Return exactly this JSON shape and nothing else: {{\"questions\":[{{\"question\":\"...\",\"option_a\":\"...\",\"option_b\":\"...\",\"option_c\":\"...\",\"option_d\":\"...\",\"correct_answer\":\"A\",\"explanation\":\"...\"}}]}}\n\n",
                "Chunk:\n{}\n\n",
                "Key points JSON:\n{}"
            ),
            chunk_markdown,
            key_points_json,
        );

        let (parsed, attempts) = self
            .retry_with_backoff("Stage B", |attempt| {
                let user_prompt = user_prompt.clone();
                async move {
                let json_payload = self
                    .chat_json(system_prompt, &user_prompt, stage_b_format_schema())
                    .await?;

                parse_stage_b_output(&json_payload).map_err(|err| {
                    log::warn!(
                        "LLM Stage B schema validation failed on attempt {}/{}: {} | payload_preview={}",
                        attempt,
                        GENERATION_MAX_ATTEMPTS,
                        err,
                        log_preview(&json_payload, 800)
                    );
                    LlmServiceError::Schema(err)
                })
                }
            })
            .await?;

        log::info!(
            "LLM Stage B generation finished (questions={}, attempts={})",
            parsed.questions.len(),
            attempts
        );
        Ok(parsed)
    }

    async fn retry_with_backoff<T, Op, Fut>(
        &self,
        stage_label: &str,
        mut operation: Op,
    ) -> Result<(T, usize), LlmServiceError>
    where
        Op: FnMut(usize) -> Fut,
        Fut: Future<Output = Result<T, LlmServiceError>>,
    {
        for attempt in 1..=GENERATION_MAX_ATTEMPTS {
            match operation(attempt).await {
                Ok(value) => return Ok((value, attempt)),
                Err(err) => {
                    if attempt >= GENERATION_MAX_ATTEMPTS {
                        return Err(err);
                    }

                    let delay = retry_delay(attempt);
                    log::warn!(
                        "LLM {} attempt {}/{} failed: {}. Retrying in {} ms",
                        stage_label,
                        attempt,
                        GENERATION_MAX_ATTEMPTS,
                        err,
                        delay.as_millis()
                    );
                    sleep(delay).await;
                }
            }
        }

        unreachable!("retry loop must return success or error")
    }

    async fn chat_json(
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
            return Err(LlmServiceError::HttpStatus { status, body });
        }

        let parsed: OllamaChatResponse = serde_json::from_str(&body).map_err(LlmServiceError::ResponseDecode)?;
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
}

#[derive(Debug)]
pub enum LlmConfigError {
    InvalidInteger { key: String, value: String },
    InvalidValue {
        key: String,
        value: String,
        reason: String,
    },
    RuntimeConfigPoisoned,
    HttpClientBuild(reqwest::Error),
}

impl fmt::Display for LlmConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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

impl Error for LlmConfigError {}

#[derive(Debug)]
pub enum LlmServiceError {
    Connect { url: String, source: reqwest::Error },
    Http(reqwest::Error),
    HttpStatus { status: StatusCode, body: String },
    ResponseDecode(serde_json::Error),
    ModelNotFound { model: String },
    Serialize(serde_json::Error),
    EmptyModelResponse,
    Schema(LlmSchemaError),
}

impl fmt::Display for LlmServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect { url, source } => {
                write!(
                    f,
                    "Cannot reach LLM endpoint at {url}: {source}. Ensure Ollama is running and listening on that URL."
                )
            }
            Self::Http(err) => write!(f, "LLM request failed: {err}"),
            Self::HttpStatus { status, body } => {
                write!(f, "LLM endpoint returned HTTP {status}: {body}")
            }
            Self::ResponseDecode(err) => write!(f, "Failed to decode LLM response envelope: {err}"),
            Self::ModelNotFound { model } => write!(
                f,
                "Model '{model}' was not found on this provider URL. Try fetching all models first."
            ),
            Self::Serialize(err) => write!(f, "Failed to serialize LLM request context: {err}"),
            Self::EmptyModelResponse => write!(f, "LLM returned an empty response message"),
            Self::Schema(err) => write!(f, "{err}"),
        }
    }
}

impl Error for LlmServiceError {}

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

    if lines.first().is_some_and(|line| line.trim_start().starts_with("```")) {
        lines.remove(0);
    }
    if lines.last().is_some_and(|line| line.trim_start().starts_with("```")) {
        lines.pop();
    }

    lines.join("\n").trim().to_string()
}

fn normalize_for_dedup(input: &str) -> String {
    input
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
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

fn stage_a_format_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "key_points": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "knowledge_point": { "type": "string" }
                    },
                    "required": ["knowledge_point"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["key_points"],
        "additionalProperties": false
    })
}

fn stage_b_format_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "questions": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "question": { "type": "string" },
                        "option_a": { "type": "string" },
                        "option_b": { "type": "string" },
                        "option_c": { "type": "string" },
                        "option_d": { "type": "string" },
                        "correct_answer": { "type": "string", "enum": ["A", "B", "C", "D"] },
                        "explanation": { "type": "string" }
                    },
                    "required": [
                        "question",
                        "option_a",
                        "option_b",
                        "option_c",
                        "option_d",
                        "correct_answer",
                        "explanation"
                    ],
                    "additionalProperties": false
                }
            }
        },
        "required": ["questions"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
        use super::*;

        #[test]
        fn parses_stage_a_wrapper() {
                let payload = r#"
                {
                    "key_points": [
                        { "knowledge_point": "Ownership defines who can access data." },
                        { "knowledge_point": "Borrowing avoids moving ownership." }
                    ]
                }
                "#;

                let parsed = parse_stage_a_output(payload).expect("stage A JSON should parse");
                assert_eq!(parsed.key_points.len(), 2);
                assert_eq!(
                        parsed.key_points[0].knowledge_point,
                        "Ownership defines who can access data."
                );
        }

        #[test]
        fn parses_stage_b_wrapper() {
                let payload = r#"
                {
                    "questions": [
                        {
                            "question": "What does ownership control in Rust?",
                            "option_a": "Memory and access",
                            "option_b": "UI rendering",
                            "option_c": "Network routing",
                            "option_d": "Audio mixing",
                            "correct_answer": "A",
                            "explanation": "Ownership defines lifecycle and access rules for values."
                        }
                    ]
                }
                "#;

                let parsed = parse_stage_b_output(payload).expect("stage B JSON should parse");
                assert_eq!(parsed.questions.len(), 1);
                assert_eq!(parsed.questions[0].correct_answer, "A");
        }

                #[test]
                fn allows_stage_a_when_key_points_empty() {
                    let payload = r#"{ "key_points": [] }"#;
                    let parsed = parse_stage_a_output(payload).expect("stage A should allow empty key points");
                    assert!(parsed.key_points.is_empty());
                }

                #[test]
                fn fails_stage_b_when_correct_answer_invalid() {
                    let payload = r#"
                    {
                        "questions": [
                        {
                            "question": "Q?",
                            "option_a": "A1",
                            "option_b": "B1",
                            "option_c": "C1",
                            "option_d": "D1",
                            "correct_answer": "E",
                            "explanation": "Because"
                        }
                        ]
                    }
                    "#;

                    let err = parse_stage_b_output(payload).expect_err("stage B should fail validation");
                    assert!(matches!(
                        err,
                        LlmSchemaError::Validation(LlmValidationError::InvalidCorrectAnswer { .. })
                    ));
                }

                #[test]
                fn fails_stage_b_when_options_duplicate() {
                    let payload = r#"
                    {
                        "questions": [
                        {
                            "question": "Q?",
                            "option_a": "Same",
                            "option_b": "Same",
                            "option_c": "C1",
                            "option_d": "D1",
                            "correct_answer": "A",
                            "explanation": "Because"
                        }
                        ]
                    }
                    "#;

                    let err = parse_stage_b_output(payload).expect_err("stage B should fail validation");
                    assert!(matches!(
                        err,
                        LlmSchemaError::Validation(LlmValidationError::DuplicateOptions { .. })
                    ));
                }

                #[test]
                fn fails_stage_b_when_questions_duplicate() {
                    let payload = r#"
                    {
                        "questions": [
                        {
                            "question": "What is Kubernetes scheduler?",
                            "option_a": "A1",
                            "option_b": "B1",
                            "option_c": "C1",
                            "option_d": "D1",
                            "correct_answer": "A",
                            "explanation": "Because"
                        },
                        {
                            "question": "What is kubernetes scheduler",
                            "option_a": "A2",
                            "option_b": "B2",
                            "option_c": "C2",
                            "option_d": "D2",
                            "correct_answer": "B",
                            "explanation": "Because 2"
                        }
                        ]
                    }
                    "#;

                    let err = parse_stage_b_output(payload).expect_err("stage B should fail validation");
                    assert!(matches!(
                        err,
                        LlmSchemaError::Validation(LlmValidationError::DuplicateQuestion { .. })
                    ));
                }
}
