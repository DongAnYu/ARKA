//! Stage B MCQ generation: LLM orchestration for active recall questions.
//!
//! Orchestrates the complete MCQ generation pipeline:
//! 1. Build structured prompt from GraphContextBundle
//! 2. Call LLM via JsonGenerationRequest (with built-in retry logic)
//! 3. Parse JSON response into GeneratedMCQ struct
//! 4. Validate structural correctness
//! 5. Return or error
//!
//! Error handling: Retries are managed by LlmService.generate_json_with_retries().
//! This module focuses on orchestration, not retry logic.

use crate::services::llm::{JsonGenerationRequest, LlmService, LlmServiceError};
use serde::Deserialize;
use serde_json::{json, Value};

use super::stage_b_prompt;
use super::stage_b_schema::{validate_mcq, GeneratedMCQ};
use super::types::{GraphContextBundle, QuestionType};

// =====================================================================
// Main Generation Function
// =====================================================================

/// Generates a single MCQ from a GraphContextBundle via LLM.
///
/// Orchestrates the full pipeline:
/// - Builds system and user prompts from bundle
/// - Calls LLM with JSON schema for GeneratedMCQ
/// - Parses JSON response into structured format
/// - Validates against MCQ requirements
/// - Returns GeneratedMCQ or LlmServiceError
///
/// Retry logic with exponential backoff is handled internally
/// by LlmService.generate_json_with_retries().
///
/// # Arguments
/// - `bundle` — GraphContextBundle with root point, related points, and relationships
/// - `llm_client` — Configured LLM service (Ollama or OpenRouter)
///
/// # Returns
/// - Ok(GeneratedMCQ) on success
/// - Err(LlmServiceError) if LLM call fails, JSON parsing fails, or validation fails
///
/// # Errors
/// - LlmServiceError::Connect — Cannot reach LLM endpoint
/// - LlmServiceError::HttpStatus — LLM server error
/// - LlmServiceError::ResponseDecode — Malformed JSON response envelope
/// - LlmServiceError::Schema — Parse error or validation error
/// - LlmServiceError::EmptyModelResponse — LLM returned no content
pub async fn generate_mcq(
    bundle: &GraphContextBundle,
    llm_client: &LlmService,
) -> Result<GeneratedMCQ, LlmServiceError> {
    // Build prompts
    let system_prompt = stage_b_prompt::system_prompt();
    let user_prompt = stage_b_prompt::build_user_prompt(bundle);

    // Build JSON schema that describes GeneratedMCQ output format
    let format_schema = schema_for_generated_mcq();

    // Prepare LLM request
    let request = JsonGenerationRequest {
        stage_label: "stage_b_mcq",
        system_prompt: &system_prompt,
        user_prompt: &user_prompt,
        format_schema,
        payload_preview_chars: 200,
    };

    let (mcq, _raw_json, attempt) = llm_client
        .generate_json_with_retries(request, |json_payload| {
            parse_and_validate_mcq(json_payload, bundle.question_type)
        })
        .await?;

    // Log success
    let question_type_label = match mcq.question_type {
        super::types::QuestionType::Recall => "RECALL",
        super::types::QuestionType::Relational => "RELATIONAL",
    };
    log::info!(
        "Generated {} MCQ for kp:{} (attempt {}, {} chars in question)",
        question_type_label,
        bundle.root_point.id,
        attempt,
        mcq.question.len()
    );

    Ok(mcq)
}

// =====================================================================
// JSON Schema for GeneratedMCQ
// =====================================================================

/// Builds the JSON schema that describes GeneratedMCQ output format.
///
/// This schema is passed to the LLM to constrain its output to valid MCQ structure:
/// - question: string (10-500 chars)
/// - options: array of exactly 4 strings (1-500 chars each)
/// - correct_index: integer in [0, 3]
/// - explanation: string (20-1000 chars)
///
/// The schema matches GeneratedMCQ struct but without the question_type field
/// (that's computed from supporting_relations presence during bundle assembly).
fn schema_for_generated_mcq() -> Value {
    json!({
        "type": "object",
        "properties": {
            "question": {
                "type": "string",
                "description": "The multiple-choice question (10-500 characters)",
                "minLength": 10,
                "maxLength": 500
            },
            "options": {
                "type": "array",
                "description": "Exactly 4 answer options (each 1-500 characters)",
                "items": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 500
                },
                "minItems": 4,
                "maxItems": 4
            },
            "correct_index": {
                "type": "integer",
                "description": "Index of correct answer (0-3)",
                "minimum": 0,
                "maximum": 3
            },
            "explanation": {
                "type": "string",
                "description": "Explanation of why the answer is correct (20-1000 characters)",
                "minLength": 20,
                "maxLength": 1000
            }
        },
        "required": ["question", "options", "correct_index", "explanation"],
        "additionalProperties": false
    })
}

// =====================================================================
// JSON Parsing and Validation
// =====================================================================

#[derive(Debug, Deserialize)]
struct WireGeneratedMCQ {
    question: String,
    options: Vec<String>,
    correct_index: usize,
    explanation: String,
}

/// Parses JSON response from LLM into GeneratedMCQ and validates it.
///
/// This closure is called by LlmService.generate_json_with_retries() as the
/// parsing function. On each attempt, we:
/// 1. Deserialize JSON string into GeneratedMCQ struct
/// 2. Run structural validation via validate_mcq()
/// 3. Return parsed MCQ or error
///
/// # Arguments
/// - `json_payload` — Raw JSON string from LLM response
/// # Returns
/// - Ok(GeneratedMCQ) if parse and validation succeed
/// - Err(LlmServiceError) if parse or validation fails
fn parse_and_validate_mcq(
    json_payload: &str,
    question_type: QuestionType,
) -> Result<GeneratedMCQ, LlmServiceError> {
    // Deserialize only the LLM-owned fields. `question_type` is computed from
    // the bundle and should never be supplied by the model.
    let wire_mcq: WireGeneratedMCQ = serde_json::from_str(json_payload).map_err(|err| {
        LlmServiceError::Schema(
            crate::services::llm::default_generation_schema::LlmSchemaError::Parse(err),
        )
    })?;

    let mcq = GeneratedMCQ {
        question: wire_mcq.question,
        options: wire_mcq.options,
        correct_index: wire_mcq.correct_index,
        explanation: wire_mcq.explanation,
        question_type,
    };

    // Validate structural correctness
    let validation_errors = validate_mcq(&mcq);
    if !validation_errors.is_empty() {
        let error_msg = validation_errors.join("; ");
        return Err(LlmServiceError::InvalidOutput(error_msg));
    }

    Ok(mcq)
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::graph_generation::types::{
        EntityNode, KnowledgePoint, KnowledgeType, QuestionType, Relation, RelationType,
    };

    fn make_entity(id: &str, name: &str) -> EntityNode {
        EntityNode {
            id: id.to_string(),
            canonical_name: name.to_string(),
            aliases: vec![name.to_string()],
            chunk_ids: vec!["c1".to_string()],
        }
    }

    fn make_point(id: &str, text: &str) -> KnowledgePoint {
        KnowledgePoint {
            id: id.to_string(),
            chunk_id: "c1".to_string(),
            point: text.to_string(),
            knowledge_type: KnowledgeType::Fact,
            raw_entity_names: vec![],
            raw_relations: vec![],
            entity_ids: vec![],
        }
    }

    fn make_bundle_recall() -> GraphContextBundle {
        GraphContextBundle {
            root_point: make_point(
                "p1",
                "JWT is a self-contained token for stateless authentication",
            ),
            related_points: vec![make_point(
                "p2",
                "Tokens eliminate server-side session storage",
            )],
            question_type: QuestionType::Recall,
            supporting_entities: vec![
                make_entity("e1", "JWT"),
                make_entity("e2", "Authentication"),
            ],
            supporting_relations: vec![],
        }
    }

    fn make_bundle_relational() -> GraphContextBundle {
        GraphContextBundle {
            root_point: make_point(
                "p1",
                "JWT is a self-contained token for stateless authentication",
            ),
            related_points: vec![make_point(
                "p2",
                "Cookies require server-side session storage",
            )],
            question_type: QuestionType::Relational,
            supporting_entities: vec![make_entity("e1", "JWT"), make_entity("e2", "Cookies")],
            supporting_relations: vec![Relation {
                source_id: "e1".to_string(),
                target_id: "e2".to_string(),
                relation_type: RelationType::Contrasts,
            }],
        }
    }

    #[test]
    fn test_schema_for_generated_mcq_is_valid_json() {
        let schema = schema_for_generated_mcq();
        // Should deserialize to Value without errors
        assert!(schema.is_object());
        assert!(schema["properties"].is_object());
        assert!(schema["properties"]["question"].is_object());
        assert!(schema["properties"]["options"].is_object());
        assert!(schema["properties"]["correct_index"].is_object());
        assert!(schema["properties"]["explanation"].is_object());
    }

    #[test]
    fn test_schema_requires_all_fields() {
        let schema = schema_for_generated_mcq();
        let required = schema["required"].as_array().unwrap();
        assert_eq!(required.len(), 4);
        assert!(required.iter().any(|r| r.as_str() == Some("question")));
        assert!(required.iter().any(|r| r.as_str() == Some("options")));
        assert!(required.iter().any(|r| r.as_str() == Some("correct_index")));
        assert!(required.iter().any(|r| r.as_str() == Some("explanation")));
    }

    #[test]
    fn test_parse_and_validate_valid_mcq() {
        let json = r#"{
            "question": "What is JWT used for?",
            "options": ["Stateless auth", "Session storage", "Compression", "Encryption"],
            "correct_index": 0,
            "explanation": "JWT tokens are self-contained and used for stateless authentication"
        }"#;

        let result = parse_and_validate_mcq(json, QuestionType::Recall);
        assert!(result.is_ok());
        let mcq = result.unwrap();
        assert_eq!(mcq.question, "What is JWT used for?");
        assert_eq!(mcq.options.len(), 4);
        assert_eq!(mcq.correct_index, 0);
    }

    #[test]
    fn test_parse_and_validate_invalid_json() {
        let json = r#"{ "question": "What is JWT?" }"#;
        let result = parse_and_validate_mcq(json, QuestionType::Recall);
        assert!(result.is_err());
        match result.unwrap_err() {
            LlmServiceError::Schema(_) => {}
            other => panic!("Expected Schema error, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_and_validate_wrong_option_count() {
        let json = r#"{
            "question": "What is JWT?",
            "options": ["A", "B", "C"],
            "correct_index": 0,
            "explanation": "JWT is a token format for stateless authentication"
        }"#;

        let result = parse_and_validate_mcq(json, QuestionType::Recall);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_and_validate_correct_index_out_of_range() {
        let json = r#"{
            "question": "What is JWT?",
            "options": ["A", "B", "C", "D"],
            "correct_index": 5,
            "explanation": "JWT is a token format for stateless authentication"
        }"#;

        let result = parse_and_validate_mcq(json, QuestionType::Recall);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_and_validate_question_too_short() {
        let json = r#"{
            "question": "JWT?",
            "options": ["A", "B", "C", "D"],
            "correct_index": 0,
            "explanation": "JWT is a token format for stateless authentication"
        }"#;

        let result = parse_and_validate_mcq(json, QuestionType::Recall);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_and_validate_explanation_too_short() {
        let json = r#"{
            "question": "What is JWT used for in web applications?",
            "options": ["A", "B", "C", "D"],
            "correct_index": 0,
            "explanation": "Stateless"
        }"#;

        let result = parse_and_validate_mcq(json, QuestionType::Recall);
        assert!(result.is_err());
    }

    #[test]
    fn test_bundle_recall_has_empty_relations() {
        let bundle = make_bundle_recall();
        assert!(bundle.supporting_relations.is_empty());
        assert_eq!(bundle.question_type, QuestionType::Recall);
    }

    #[test]
    fn test_bundle_relational_has_relations() {
        let bundle = make_bundle_relational();
        assert!(!bundle.supporting_relations.is_empty());
        assert_eq!(bundle.question_type, QuestionType::Relational);
    }
}
