use std::error::Error;
use std::fmt;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::models::learning_item::{validate_generated_output, GeneratedItemsOutput};

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

#[derive(Debug)]
pub enum LlmSchemaError {
    Parse(serde_json::Error),
    Validation(LlmValidationError),
}

#[derive(Debug)]
pub enum LlmValidationError {
    EmptyKnowledgePoint { index: usize },
    InvalidGeneratedItems { reason: &'static str },
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
            Self::InvalidGeneratedItems { reason } => {
                write!(f, "generated learning items failed validation: {reason}")
            }
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

/// Parses Stage B output into the same contract used by LearningItem generation.
pub fn parse_stage_b_output(
    json_payload: &str,
    known_point_ids: &[&str],
) -> Result<GeneratedItemsOutput, LlmSchemaError> {
    let parsed: GeneratedItemsOutput =
        serde_json::from_str(json_payload).map_err(LlmSchemaError::Parse)?;
    validate_generated_output(&parsed, known_point_ids).map_err(|reason| {
        LlmSchemaError::Validation(LlmValidationError::InvalidGeneratedItems { reason })
    })?;
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
            "items": {
                "type": "array",
                "maxItems": 4,
                "items": {
                    "type": "object",
                    "properties": {
                        "knowledge_point_id": { "type": "string" },
                        "target": { "type": "string" },
                        "answer": { "type": "string" },
                        "explanation": { "type": ["string", "null"] },
                        "flashcard": {
                            "type": "object",
                            "properties": {
                                "prompt": { "type": "string" }
                            },
                            "required": ["prompt"],
                            "additionalProperties": false
                        },
                        "mcq": {
                            "anyOf": [
                                {
                                    "type": "object",
                                    "properties": {
                                        "prompt": { "type": ["string", "null"] },
                                        "distractors": {
                                            "type": "array",
                                            "minItems": 3,
                                            "maxItems": 3,
                                            "items": { "type": "string" }
                                        }
                                    },
                                    "required": ["prompt", "distractors"],
                                    "additionalProperties": false
                                },
                                { "type": "null" }
                            ]
                        },
                        "mcq_omission_reason": { "type": ["string", "null"] }
                    },
                    "required": [
                        "knowledge_point_id",
                        "target",
                        "answer",
                        "explanation",
                        "flashcard",
                        "mcq",
                        "mcq_omission_reason"
                    ],
                    "additionalProperties": false
                }
            }
        },
        "required": ["items"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_generation_schemas_are_strict_openai_compatible() {
        crate::services::llm::assert_strict_json_schema(&stage_a_format_schema());
        crate::services::llm::assert_strict_json_schema(&stage_b_format_schema());
    }

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
                    "items": [
                        {
                            "knowledge_point_id": "kp_1",
                            "target": "Rust ownership controls the lifecycle and access rules for values.",
                            "answer": "The lifecycle and access rules for values",
                            "explanation": "Ownership determines how values are managed and accessed.",
                            "flashcard": { "prompt": "What does ownership control in Rust?" },
                            "mcq": {
                                "prompt": null,
                                "distractors": ["UI rendering", "Network routing", "Audio mixing"]
                            },
                            "mcq_omission_reason": null
                        }
                    ]
                }
                "#;

        let parsed = parse_stage_b_output(payload, &["kp_1"]).expect("stage B JSON should parse");
        assert_eq!(parsed.items.len(), 1);
        assert_eq!(parsed.items[0].knowledge_point_id, "kp_1");
        assert!(parsed.items[0].mcq.is_some());
    }

    #[test]
    fn allows_stage_a_when_key_points_empty() {
        let payload = r#"{ "key_points": [] }"#;
        let parsed = parse_stage_a_output(payload).expect("stage A should allow empty key points");
        assert!(parsed.key_points.is_empty());
    }

    #[test]
    fn parses_stage_b_flashcard_without_mcq() {
        let payload = r#"
                {
                    "items": [
                        {
                            "knowledge_point_id": "kp_1",
                            "target": "A concept with no useful distractors.",
                            "answer": "The exact answer",
                            "explanation": null,
                            "flashcard": { "prompt": "What is the exact answer?" },
                            "mcq": null,
                            "mcq_omission_reason": "Could not create three plausible distractors without making the answer obvious."
                        }
                    ]
                }
                "#;

        let parsed = parse_stage_b_output(payload, &["kp_1"]).expect("stage B JSON should parse");
        assert!(parsed.items[0].mcq.is_none());
    }

    #[test]
    fn fails_stage_b_when_knowledge_point_id_is_invented() {
        let payload = r#"
                {
                    "items": [
                        {
                            "knowledge_point_id": "kp_99",
                            "target": "Target",
                            "answer": "Answer",
                            "explanation": null,
                            "flashcard": { "prompt": "Prompt?" },
                            "mcq": null,
                            "mcq_omission_reason": "The target does not translate cleanly into a single-answer multiple-choice question."
                        }
                    ]
                }
                "#;

        let err =
            parse_stage_b_output(payload, &["kp_1"]).expect_err("stage B should fail validation");
        assert!(matches!(
            err,
            LlmSchemaError::Validation(LlmValidationError::InvalidGeneratedItems {
                reason: "unknown_knowledge_point"
            })
        ));
    }
}
