//! Stage B MCQ schema: generated question structures and validation.
//!
//! Represents a complete multiple-choice question generated from a GraphContextBundle.
//! Includes structural validation to ensure consistency with downstream storage.

use serde::{Deserialize, Serialize};

use super::types::QuestionType;

// =====================================================================
// Generated MCQ Structure
// =====================================================================

/// A complete MCQ generated from a bundle (Stage B output).
///
/// This is the final product before database storage. Includes all necessary
/// information for active recall questions plus metadata for evaluation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GeneratedMCQ {
    /// The question text (e.g., "What is the primary role of JWT in web authentication?")
    pub question: String,

    /// Exactly 4 answer options
    pub options: Vec<String>,

    /// Index of the correct answer: 0-3
    pub correct_index: usize,

    /// Explanation of why the answer is correct
    pub explanation: String,

    /// Question classification: Recall or Relational
    ///
    /// Used during evaluation to compare actual vs expected question types.
    /// Recall: single-entity factual questions (e.g., "Define JWT")
    /// Relational: cross-entity connection questions (e.g., "How does JWT relate to Cookies?")
    pub question_type: QuestionType,
}

// =====================================================================
// Validation
// =====================================================================

/// Validates a GeneratedMCQ for structural correctness.
///
/// Returns a Vec of validation errors. Empty vec means valid.
///
/// Checks:
/// - correct_index in [0, 3]
/// - options.len() == 4 and all non-empty
/// - question non-empty and reasonable length
/// - explanation non-empty
pub fn validate_mcq(mcq: &GeneratedMCQ) -> Vec<String> {
    let mut errors = Vec::new();

    // Validate correct_index
    if mcq.correct_index > 3 {
        errors.push(format!(
            "correct_index {} out of range [0, 3]",
            mcq.correct_index
        ));
    }

    // Validate options
    if mcq.options.len() != 4 {
        errors.push(format!("expected 4 options, got {}", mcq.options.len()));
    }

    for (i, option) in mcq.options.iter().enumerate() {
        if option.trim().is_empty() {
            errors.push(format!("option {} is empty", i));
        }
        if option.len() > 500 {
            errors.push(format!("option {} exceeds 500 chars ({})", i, option.len()));
        }
    }

    // Validate question
    if mcq.question.trim().is_empty() {
        errors.push("question is empty".to_string());
    }
    if mcq.question.len() < 10 {
        errors.push(format!(
            "question too short ({} chars, min 10)",
            mcq.question.len()
        ));
    }
    if mcq.question.len() > 500 {
        errors.push(format!(
            "question too long ({} chars, max 500)",
            mcq.question.len()
        ));
    }

    // Validate explanation
    if mcq.explanation.trim().is_empty() {
        errors.push("explanation is empty".to_string());
    }
    if mcq.explanation.len() < 20 {
        errors.push(format!(
            "explanation too short ({} chars, min 20)",
            mcq.explanation.len()
        ));
    }
    if mcq.explanation.len() > 1000 {
        errors.push(format!(
            "explanation too long ({} chars, max 1000)",
            mcq.explanation.len()
        ));
    }

    errors
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_valid_mcq() -> GeneratedMCQ {
        GeneratedMCQ {
            question: "What is the primary role of JWT in web authentication?".to_string(),
            options: vec![
                "To encrypt passwords on the server".to_string(),
                "To create stateless authentication tokens".to_string(),
                "To store session data in the database".to_string(),
                "To hash user passwords".to_string(),
            ],
            correct_index: 1,
            explanation: "JWT (JSON Web Tokens) are used to create stateless authentication tokens that can be verified without accessing the server's database.".to_string(),
            question_type: QuestionType::Recall,
        }
    }

    #[test]
    fn test_valid_mcq() {
        let mcq = make_valid_mcq();
        let errors = validate_mcq(&mcq);
        assert!(errors.is_empty(), "Expected no errors, got: {:?}", errors);
    }

    #[test]
    fn test_correct_index_out_of_range() {
        let mut mcq = make_valid_mcq();
        mcq.correct_index = 4;
        let errors = validate_mcq(&mcq);
        assert!(!errors.is_empty());
        assert!(errors.iter().any(|e| e.contains("correct_index")));
    }

    #[test]
    fn test_wrong_option_count_too_few() {
        let mut mcq = make_valid_mcq();
        mcq.options.pop();
        let errors = validate_mcq(&mcq);
        assert!(!errors.is_empty());
        assert!(errors.iter().any(|e| e.contains("expected 4 options")));
    }

    #[test]
    fn test_wrong_option_count_too_many() {
        let mut mcq = make_valid_mcq();
        mcq.options.push("Extra option".to_string());
        let errors = validate_mcq(&mcq);
        assert!(!errors.is_empty());
        assert!(errors.iter().any(|e| e.contains("expected 4 options")));
    }

    #[test]
    fn test_empty_option() {
        let mut mcq = make_valid_mcq();
        mcq.options[2] = "".to_string();
        let errors = validate_mcq(&mcq);
        assert!(!errors.is_empty());
        assert!(errors.iter().any(|e| e.contains("option") && e.contains("empty")));
    }

    #[test]
    fn test_option_too_long() {
        let mut mcq = make_valid_mcq();
        mcq.options[0] = "x".repeat(501);
        let errors = validate_mcq(&mcq);
        assert!(!errors.is_empty());
        assert!(errors.iter().any(|e| e.contains("exceeds 500 chars")));
    }

    #[test]
    fn test_empty_question() {
        let mut mcq = make_valid_mcq();
        mcq.question = "".to_string();
        let errors = validate_mcq(&mcq);
        assert!(!errors.is_empty());
        assert!(errors.iter().any(|e| e.contains("question is empty")));
    }

    #[test]
    fn test_question_too_short() {
        let mut mcq = make_valid_mcq();
        mcq.question = "Short?".to_string();
        let errors = validate_mcq(&mcq);
        assert!(!errors.is_empty());
        assert!(errors.iter().any(|e| e.contains("question too short")));
    }

    #[test]
    fn test_question_too_long() {
        let mut mcq = make_valid_mcq();
        mcq.question = "x".repeat(501);
        let errors = validate_mcq(&mcq);
        assert!(!errors.is_empty());
        assert!(errors.iter().any(|e| e.contains("question too long")));
    }

    #[test]
    fn test_empty_explanation() {
        let mut mcq = make_valid_mcq();
        mcq.explanation = "".to_string();
        let errors = validate_mcq(&mcq);
        assert!(!errors.is_empty());
        assert!(errors.iter().any(|e| e.contains("explanation is empty")));
    }

    #[test]
    fn test_explanation_too_short() {
        let mut mcq = make_valid_mcq();
        mcq.explanation = "Too short!".to_string();
        let errors = validate_mcq(&mcq);
        assert!(!errors.is_empty());
        assert!(errors.iter().any(|e| e.contains("explanation too short")));
    }

    #[test]
    fn test_explanation_too_long() {
        let mut mcq = make_valid_mcq();
        mcq.explanation = "x".repeat(1001);
        let errors = validate_mcq(&mcq);
        assert!(!errors.is_empty());
        assert!(errors.iter().any(|e| e.contains("explanation too long")));
    }

    #[test]
    fn test_relational_question_type() {
        let mut mcq = make_valid_mcq();
        mcq.question_type = QuestionType::Relational;
        let errors = validate_mcq(&mcq);
        assert!(errors.is_empty());
        assert_eq!(mcq.question_type, QuestionType::Relational);
    }

    #[test]
    fn test_multiple_validation_errors() {
        let mut mcq = make_valid_mcq();
        mcq.correct_index = 5;
        mcq.options.pop();
        mcq.question = "Short".to_string();
        mcq.explanation = "Bad".to_string();
        let errors = validate_mcq(&mcq);
        assert!(errors.len() > 1, "Expected multiple errors, got: {:?}", errors);
    }
}
