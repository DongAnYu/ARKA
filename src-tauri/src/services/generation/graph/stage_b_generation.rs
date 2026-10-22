//! Stage B MCQ generation: LLM orchestration for active recall questions.
//!
//! Orchestrates the complete MCQ generation pipeline:
//! 1. Build structured prompt from GraphContextBundle
//! 2. Call LLM via StructuredGenerationRequest (with built-in retry logic)
//! 3. Assemble options and their answer index from the model's answer/distractors
//! 4. Validate structural correctness
//! 5. Return or error
//!
//! Error handling: Retries are managed by LlmService.generate_json_with_retries().
//! This module focuses on orchestration, not retry logic.

use crate::services::llm::{LlmService, LlmServiceError, StructuredGenerationRequest};
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

    // The provider writes content; option positions and their key belong to us.
    let format_schema = schema_for_generated_mcq();

    // Prepare LLM request
    let request = StructuredGenerationRequest {
        stage_label: "stage_b_mcq",
        schema_name: "graph_stage_b_mcq",
        system_prompt: &system_prompt,
        user_prompt: &user_prompt,
        schema: format_schema,
        payload_preview_chars: 200,
    };

    let (mcq, _raw_json, attempt) = llm_client
        .generate_json_with_retries(request, |json_payload| {
            parse_and_validate_mcq(json_payload, bundle.question_type, &bundle.root_point.id)
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

/// Builds the schema for model-owned question content.
///
/// This schema is passed to the LLM to constrain its output to valid MCQ structure:
/// - question: string (10-500 chars)
/// - answer: the correct answer text (1-500 chars)
/// - distractors: exactly 3 incorrect answers (1-500 chars each)
/// - explanation: string (20-1000 chars)
///
/// The backend supplies option positions, correct_index, and question_type when
/// converting this response to the existing GeneratedMCQ representation.
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
            "answer": {
                "type": "string",
                "description": "The correct answer, supported by the source and explanation",
                "minLength": 1,
                "maxLength": 500
            },
            "distractors": {
                "type": "array",
                "description": "Exactly 3 distinct, plausible but incorrect answers",
                "items": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 500
                },
                "minItems": 3,
                "maxItems": 3
            },
            "explanation": {
                "type": "string",
                "description": "Explanation of why the answer is correct (20-1000 characters)",
                "minLength": 20,
                "maxLength": 1000
            }
        },
        "required": ["question", "answer", "distractors", "explanation"],
        "additionalProperties": false
    })
}

// =====================================================================
// JSON Parsing and Validation
// =====================================================================

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireGeneratedMCQ {
    question: String,
    answer: String,
    distractors: [String; 3],
    explanation: String,
}

/// Parses JSON response from LLM into GeneratedMCQ and validates it.
///
/// This closure is called by LlmService.generate_json_with_retries() as the
/// parsing function. On each attempt, we:
/// 1. Deserialize the answer and exactly three distractors
/// 2. Insert the answer at a stable backend-owned position
/// 3. Run structural and duplicate-choice validation
/// 4. Return parsed MCQ or error
///
/// # Arguments
/// - `json_payload` — Raw JSON string from LLM response
/// # Returns
/// - Ok(GeneratedMCQ) if parse and validation succeed
/// - Err(LlmServiceError) if parse or validation fails
fn parse_and_validate_mcq(
    json_payload: &str,
    question_type: QuestionType,
    root_point_id: &str,
) -> Result<GeneratedMCQ, LlmServiceError> {
    // Reject model-supplied keys and positions rather than trusting them.
    let wire_mcq: WireGeneratedMCQ = serde_json::from_str(json_payload).map_err(|err| {
        LlmServiceError::Schema(
            crate::services::llm::default_generation_schema::LlmSchemaError::Parse(err),
        )
    })?;

    // A stable root-ID checksum spreads answers across A-D without depending on
    // task completion order or changing positions between provider retries.
    let correct_index = root_point_id
        .bytes()
        .fold(0, |position, byte| (position + usize::from(byte)) % 4);
    let mut options = Vec::from(wire_mcq.distractors);
    options.insert(correct_index, wire_mcq.answer);

    let mcq = GeneratedMCQ {
        question: wire_mcq.question,
        options,
        correct_index,
        explanation: wire_mcq.explanation,
        question_type,
    };

    // Validate structural correctness
    let mut validation_errors = validate_mcq(&mcq);
    let mut seen = std::collections::HashSet::new();
    for option in &mcq.options {
        let normalized = option
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        if !seen.insert(normalized) {
            validation_errors.push("Answer and distractors must all be distinct".into());
            break;
        }
    }
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
    use crate::services::generation::graph::types::{
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
        assert!(schema["properties"]["answer"].is_object());
        assert_eq!(schema["properties"]["distractors"]["minItems"], 3);
        assert_eq!(schema["properties"]["distractors"]["maxItems"], 3);
        assert!(schema["properties"].get("options").is_none());
        assert!(schema["properties"].get("correct_index").is_none());
        assert!(schema["properties"]["explanation"].is_object());
    }

    #[test]
    fn generated_mcq_schema_is_strict_openai_compatible() {
        crate::services::llm::assert_strict_json_schema(&schema_for_generated_mcq());
    }

    #[test]
    fn test_schema_requires_all_fields() {
        let schema = schema_for_generated_mcq();
        let required = schema["required"].as_array().unwrap();
        assert_eq!(required.len(), 4);
        assert!(required.iter().any(|r| r.as_str() == Some("question")));
        assert!(required.iter().any(|r| r.as_str() == Some("answer")));
        assert!(required.iter().any(|r| r.as_str() == Some("distractors")));
        assert!(required.iter().any(|r| r.as_str() == Some("explanation")));
    }

    fn valid_content() -> Value {
        json!({
            "question": "What is JWT used for?",
            "answer": "Stateless auth",
            "distractors": ["Session storage", "Compression", "Encryption"],
            "explanation": "JWT tokens are self-contained and used for stateless authentication"
        })
    }

    #[test]
    fn backend_positions_preserve_answer_and_downstream_format() {
        let content = valid_content().to_string();
        let mut positions = std::collections::HashSet::new();
        for root_id in [
            "chunk-0-kp-0",
            "chunk-0-kp-1",
            "chunk-0-kp-2",
            "chunk-0-kp-3",
        ] {
            let mcq = parse_and_validate_mcq(&content, QuestionType::Relational, root_id).unwrap();
            positions.insert(mcq.correct_index);
            assert_eq!(mcq.options.len(), 4);
            assert_eq!(mcq.options[mcq.correct_index], "Stateless auth");
            let distractors = mcq
                .options
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != mcq.correct_index)
                .map(|(_, text)| text.as_str())
                .collect::<Vec<_>>();
            assert_eq!(
                distractors,
                ["Session storage", "Compression", "Encryption"]
            );
            assert_eq!(mcq.question_type, QuestionType::Relational);
            // Replaying a request must preserve both the options and their key.
            assert_eq!(
                mcq,
                parse_and_validate_mcq(&content, QuestionType::Relational, root_id).unwrap()
            );
            let stored = serde_json::to_value(&mcq).unwrap();
            assert!(stored.get("answer").is_none());
            assert!(stored.get("distractors").is_none());
            assert_eq!(stored["options"][mcq.correct_index], "Stateless auth");
            assert_eq!(serde_json::from_value::<GeneratedMCQ>(stored).unwrap(), mcq);
        }
        assert_eq!(positions.len(), 4);
    }

    #[test]
    fn test_parse_and_validate_invalid_json() {
        let json = r#"{ "question": "What is JWT?" }"#;
        let result = parse_and_validate_mcq(json, QuestionType::Recall, "p1");
        assert!(result.is_err());
        match result.unwrap_err() {
            LlmServiceError::Schema(_) => {}
            other => panic!("Expected Schema error, got {:?}", other),
        }
    }

    #[test]
    fn rejects_wrong_distractor_counts() {
        for distractors in [json!([]), json!(["A", "B"]), json!(["A", "B", "C", "D"])] {
            let mut content = valid_content();
            content["distractors"] = distractors;
            assert!(
                parse_and_validate_mcq(&content.to_string(), QuestionType::Recall, "p1").is_err()
            );
        }
    }

    #[test]
    fn rejects_model_owned_positions_and_classification() {
        for (field, value) in [
            ("correct_index", json!(2)),
            ("options", json!(["A", "B", "C", "D"])),
            ("question_type", json!("recall")),
        ] {
            let mut content = valid_content();
            content[field] = value;
            assert!(
                parse_and_validate_mcq(&content.to_string(), QuestionType::Recall, "p1").is_err()
            );
        }
    }

    #[test]
    fn rejects_invalid_content_lengths_and_blank_choices() {
        for (field, value) in [
            ("question", json!("JWT?")),
            ("question", json!("x".repeat(501))),
            ("explanation", json!("Stateless")),
            ("explanation", json!("x".repeat(1001))),
            ("answer", json!("   ")),
            ("answer", json!("x".repeat(501))),
            (
                "distractors",
                json!(["Session storage", "\n", "Encryption"]),
            ),
            (
                "distractors",
                json!(["Session storage", "x".repeat(501), "Encryption"]),
            ),
        ] {
            let mut content = valid_content();
            content[field] = value;
            assert!(
                parse_and_validate_mcq(&content.to_string(), QuestionType::Recall, "p1").is_err(),
                "accepted {field}"
            );
        }
    }

    #[test]
    fn rejects_answer_distractor_and_distractor_distractor_duplicates() {
        for distractors in [
            json!([" stateless  AUTH\n", "Compression", "Encryption"]),
            json!(["Session storage", " session\tSTORAGE ", "Encryption"]),
        ] {
            let mut content = valid_content();
            content["distractors"] = distractors;
            let error = parse_and_validate_mcq(&content.to_string(), QuestionType::Recall, "p1")
                .unwrap_err();
            assert!(matches!(error, LlmServiceError::InvalidOutput(_)));
        }
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
