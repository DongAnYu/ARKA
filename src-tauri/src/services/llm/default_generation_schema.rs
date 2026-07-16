use std::collections::HashSet;
use std::error::Error;
use std::fmt;

use serde::Deserialize;
use serde_json::{json, Value};

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
                write!(
                    f,
                    "field '{field}' at question index {index} must be non-empty"
                )
            }
            Self::DuplicateOptions { index } => {
                write!(f, "question at index {index} has duplicate options")
            }
            Self::DuplicateQuestion { index } => {
                write!(
                    f,
                    "question at index {index} duplicates a previous question"
                )
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
    let parsed: StageAKeyPointsOutput =
        serde_json::from_str(json_payload).map_err(LlmSchemaError::Parse)?;
    validate_stage_a_output(&parsed)?;
    Ok(parsed)
}

/// Parses Stage B LLM output using a deterministic wrapper object.
pub fn parse_stage_b_output(json_payload: &str) -> Result<StageBMcqOutput, LlmSchemaError> {
    let parsed: StageBMcqOutput =
        serde_json::from_str(json_payload).map_err(LlmSchemaError::Parse)?;
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
        return Err(LlmSchemaError::Validation(
            LlmValidationError::EmptyQuestions,
        ));
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

fn validate_non_empty(
    index: usize,
    field: &'static str,
    value: &str,
) -> Result<(), LlmSchemaError> {
    if value.trim().is_empty() {
        return Err(LlmSchemaError::Validation(LlmValidationError::EmptyField {
            index,
            field,
        }));
    }

    Ok(())
}

pub fn stage_a_format_schema() -> Value {
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

pub fn stage_b_format_schema() -> Value {
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
