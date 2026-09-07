//! Stage B prompt engineering: system prompts and context rendering.
//!
//! Prompt quality directly dominates MCQ quality. This module separates:
//! - `system_prompt()`: High-level MCQ task definition and guidelines
//! - `render_bundle_context()`: Structured formatting of bundle data for context
//!
//! By keeping prompts separate from generation logic, we can iterate on prompt
//! quality without touching code. Future improvements (Recall vs Relational
//! specific prompts, difficulty tuning, etc.) are straightforward.

use super::types::{GraphContextBundle, QuestionType};
use std::collections::HashMap;

// =====================================================================
// System Prompt
// =====================================================================

/// System prompt for MCQ generation.
///
/// Sets tone, task clarity, and quality expectations.
/// Emphasizes distinction between Recall and Relational questions.
pub fn system_prompt() -> String {
    r#"You are an expert educational MCQ (multiple-choice question) generator for active recall learning.

Your task: Generate exactly ONE high-quality MCQ from the provided knowledge point and supporting context.

REQUIREMENTS:
1. Return valid JSON only. No markdown, no prose, no explanations outside JSON.
2. The question must primarily assess understanding of the ROOT KNOWLEDGE POINT.
3. Use related knowledge only as supporting context. Do not ask about unrelated supporting facts.
4. All four options must be grammatically parallel and plausible.
5. Exactly one option is correct; the other three are convincing distractors.
6. Avoid "none of the above" or "all of the above" options.
7. The question stem must be self-contained and answerable without access to the source note.
8. Include any essential facts from an example, code block, image, diagram, figure, or table directly in the question stem.
9. Never refer to unspecified context with wording such as "the example", "shown above", "shown below", or "the following configuration".
10. Do not rely on the answer options to supply context missing from the question stem.
11. Base the question only on source information that can be restated faithfully as text.

QUESTION TYPES:

Recall Question:
- Tests direct factual knowledge of a single concept
- Example: "What is the primary role of JWT?"
- Options present different facts or definitions
- These should be straightforward but require accurate retrieval from memory
- Assume the learner has studied the material but may have forgotten details

Relational Question:
- Tests understanding of connections between multiple concepts
- Example: "How does JWT compare to Cookies in stateless authentication?"
- Requires reasoning about at least TWO connected concepts
- The correct answer depends on understanding the relationship, not recalling a single isolated fact
- Do not reduce to asking about a single concept even if related points are provided

QUALITY GUIDELINES:
- Make distractors plausible (common misconceptions are ideal)
- Ensure the question is unambiguous
- Explanation should justify why the correct answer is right
- Keep question and explanation concise but complete
- The question should require retrieval and understanding, not guessing

OUTPUT JSON SCHEMA:
{
  "question": "string (10-500 chars)",
  "options": ["option A", "option B", "option C", "option D"],
  "correct_index": 0-3,
  "explanation": "string (20-1000 chars, justify correctness)"
}
"#.to_string()
}

// =====================================================================
// Bundle Context Rendering
// =====================================================================

/// Renders a GraphContextBundle into context text for the LLM.
///
/// Formats knowledge point, related points, and relationships into a structured,
/// information-rich format optimized for LLM reasoning.
///
/// Order of information is intentional:
/// 1. ROOT KNOWLEDGE POINT (what we're testing)
/// 2. RELATED KNOWLEDGE POINTS (semantic context from the graph)
/// 3. KEY RELATIONSHIPS (how concepts connect)
/// 4. SUPPORTING ENTITIES (supplementary reference)
///
/// This order matches LLM reasoning patterns: complete semantic information
/// before isolated entity lists.
pub fn render_bundle_context(bundle: &GraphContextBundle) -> String {
    let mut context = String::new();

    // ── Root Knowledge Point ────────────────────────────────────────
    context.push_str("ROOT KNOWLEDGE POINT:\n");
    context.push_str(&format!("{}\n\n", bundle.root_point.point));

    // ── Question Type Guidance ───────────────────────────────────────
    let question_type_hint = match bundle.question_type {
        QuestionType::Recall => {
            "Question type: RECALL — focus on testing direct knowledge of this concept."
        }
        QuestionType::Relational => {
            "Question type: RELATIONAL — focus on connections between this concept and others below."
        }
    };
    context.push_str(&format!("{}\n\n", question_type_hint));

    // ── Related Points (Complete Semantic Context) ──────────────────
    if !bundle.related_points.is_empty() {
        context.push_str("RELATED KNOWLEDGE POINTS (for context):\n");
        for related in &bundle.related_points {
            context.push_str(&format!("- {}\n", related.point));
        }
        context.push('\n');
    }

    // ── Supporting Relations (Conceptual Connections) ───────────────
    if !bundle.supporting_relations.is_empty() {
        // Build lookup: entity_id → canonical_name
        let entity_lookup: HashMap<String, String> = bundle
            .supporting_entities
            .iter()
            .map(|e| (e.id.clone(), e.canonical_name.clone()))
            .collect();

        context.push_str("KEY RELATIONSHIPS:\n");
        for rel in &bundle.supporting_relations {
            // Render human-readable relation with canonical names, no internal IDs
            let rel_text = match rel.relation_type {
                crate::services::graph_generation::types::RelationType::RelatedTo => {
                    "is related to"
                }
                crate::services::graph_generation::types::RelationType::Contrasts => {
                    "contrasts with"
                }
                crate::services::graph_generation::types::RelationType::Prerequisite => {
                    "is a prerequisite for"
                }
                crate::services::graph_generation::types::RelationType::Consequence => "leads to",
                crate::services::graph_generation::types::RelationType::Example => {
                    "is an example of"
                }
                crate::services::graph_generation::types::RelationType::CounterExample => {
                    "is a counter-example to"
                }
            };
            let source_name = entity_lookup
                .get(&rel.source_id)
                .map(|s| s.as_str())
                .unwrap_or(&rel.source_id);
            let target_name = entity_lookup
                .get(&rel.target_id)
                .map(|s| s.as_str())
                .unwrap_or(&rel.target_id);
            context.push_str(&format!("- {} {} {}\n", source_name, rel_text, target_name));
        }
        context.push('\n');
    }

    // ── Supporting Entities (Supplementary Reference) ───────────────
    if !bundle.supporting_entities.is_empty() {
        context.push_str("SUPPORTING ENTITIES:\n");
        for entity in &bundle.supporting_entities {
            context.push_str(&format!("- {}\n", entity.canonical_name));
        }
        context.push('\n');
    }

    context.push_str("Generate exactly ONE multiple-choice question.\n");
    context.push_str(
        "The question must primarily assess understanding of the ROOT KNOWLEDGE POINT.\n",
    );
    context.push_str("Use the related knowledge only as supporting context.\n");
    context.push_str("Do not ask about unrelated supporting facts.\n");

    context
}

// =====================================================================
// User Prompt Builder
// =====================================================================

/// Builds the complete user prompt from a bundle.
///
/// Combines context rendering with JSON schema expectation.
pub fn build_user_prompt(bundle: &GraphContextBundle) -> String {
    let context = render_bundle_context(bundle);

    format!(
        "{}Respond with valid JSON matching this schema:\n{{\n  \"question\": \"...\",\n  \"options\": [\"A\", \"B\", \"C\", \"D\"],\n  \"correct_index\": 0-3,\n  \"explanation\": \"...\"\n}}\n",
        context
    )
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::graph_generation::types::{
        EntityNode, KnowledgePoint, KnowledgeType, Relation, RelationType,
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
            point: text.to_string(),
            knowledge_type: KnowledgeType::Fact,
            chunk_id: "c1".to_string(),
            raw_entity_names: vec![],
            entity_ids: vec![],
            raw_relations: vec![],
        }
    }

    fn make_bundle(root_type: QuestionType) -> GraphContextBundle {
        GraphContextBundle {
            root_point: make_point("kp1", "JWT is used for stateless authentication"),
            related_points: vec![
                make_point("kp2", "Cookies are used to store session state"),
                make_point("kp3", "JWTs can be stored in localStorage or cookies"),
            ],
            question_type: root_type,
            supporting_entities: vec![
                make_entity("e1", "JWT"),
                make_entity("e2", "Cookies"),
                make_entity("e3", "Authentication"),
            ],
            supporting_relations: vec![
                Relation {
                    source_id: "e1".to_string(),
                    target_id: "e3".to_string(),
                    relation_type: RelationType::RelatedTo,
                },
                Relation {
                    source_id: "e2".to_string(),
                    target_id: "e3".to_string(),
                    relation_type: RelationType::RelatedTo,
                },
            ],
        }
    }

    #[test]
    fn test_system_prompt_specifies_mcq_generation_task() {
        let prompt = system_prompt();
        assert!(prompt.contains("exactly ONE"));
        assert!(prompt.contains("primarily assess"));
        assert!(prompt.contains("ROOT KNOWLEDGE POINT"));
    }

    #[test]
    fn test_system_prompt_relational_requires_two_concepts() {
        let prompt = system_prompt();
        assert!(prompt.contains("at least TWO connected concepts"));
        assert!(prompt.contains("reasoning about"));
    }

    #[test]
    fn test_system_prompt_recall_framing_for_retrieval() {
        let prompt = system_prompt();
        assert!(prompt.contains("studied the material"));
        assert!(prompt.contains("retrieval"));
        assert!(prompt.contains("not guessing"));
    }

    #[test]
    fn test_system_prompt_requires_source_examples_to_be_self_contained() {
        let prompt = system_prompt();

        assert!(prompt.contains("self-contained and answerable without access to the source note"));
        assert!(prompt.contains(
            "essential facts from an example, code block, image, diagram, figure, or table"
        ));
        assert!(prompt.contains("Never refer to unspecified context"));
        assert!(prompt.contains("Do not rely on the answer options to supply context"));
        assert!(prompt.contains(
            "Base the question only on source information that can be restated faithfully as text"
        ));
    }

    #[test]
    fn test_render_recall_context_has_proper_structure() {
        let bundle = make_bundle(QuestionType::Recall);
        let context = render_bundle_context(&bundle);

        // Check new section order: ROOT → RELATED → RELATIONSHIPS → ENTITIES
        let root_pos = context.find("ROOT KNOWLEDGE POINT").unwrap();
        let related_pos = context
            .find("RELATED KNOWLEDGE POINTS")
            .unwrap_or(usize::MAX);
        let relations_pos = context.find("KEY RELATIONSHIPS").unwrap_or(usize::MAX);
        let entities_pos = context.find("SUPPORTING ENTITIES").unwrap_or(usize::MAX);

        // Verify order
        assert!(root_pos < related_pos || related_pos == usize::MAX);
        assert!(related_pos < relations_pos || relations_pos == usize::MAX);
        assert!(relations_pos < entities_pos || entities_pos == usize::MAX);
    }

    #[test]
    fn test_render_relational_context_emphasizes_connections() {
        let bundle = make_bundle(QuestionType::Relational);
        let context = render_bundle_context(&bundle);

        assert!(context.contains("RELATIONAL"));
        assert!(context.contains("connections"));
    }

    #[test]
    fn test_render_no_entity_ids_exposed() {
        let bundle = make_bundle(QuestionType::Recall);
        let context = render_bundle_context(&bundle);

        // Should have entity names but NOT the internal IDs
        assert!(context.contains("JWT"));
        assert!(context.contains("Cookies"));
        // IDs should not appear in the final context
        assert!(!context.contains("entity-jwt"));
        assert!(!context.contains("entity-cookies"));
    }

    #[test]
    fn test_render_includes_related_points() {
        let bundle = make_bundle(QuestionType::Recall);
        let context = render_bundle_context(&bundle);

        assert!(context.contains("RELATED KNOWLEDGE POINTS"));
        assert!(context.contains("Cookies are used to store session state"));
    }

    #[test]
    fn test_render_includes_relations() {
        let bundle = make_bundle(QuestionType::Recall);
        let context = render_bundle_context(&bundle);

        assert!(context.contains("KEY RELATIONSHIPS"));
        assert!(context.contains("is related to"));
    }

    #[test]
    fn test_render_explicit_instruction_about_root_point() {
        let bundle = make_bundle(QuestionType::Recall);
        let context = render_bundle_context(&bundle);

        assert!(context.contains("Generate exactly ONE"));
        assert!(context.contains("primarily assess"));
        assert!(context.contains("ROOT KNOWLEDGE POINT"));
        assert!(context.contains("Do not ask about unrelated"));
    }

    #[test]
    fn test_build_user_prompt_combines_context_and_schema() {
        let bundle = make_bundle(QuestionType::Recall);
        let prompt = build_user_prompt(&bundle);

        // Should contain context
        assert!(prompt.contains("ROOT KNOWLEDGE POINT"));
        assert!(prompt.contains("JWT is used"));

        // Should contain JSON schema
        assert!(prompt.contains("\"question\""));
        assert!(prompt.contains("\"options\""));
        assert!(prompt.contains("\"correct_index\""));
        assert!(prompt.contains("\"explanation\""));
    }

    #[test]
    fn test_render_empty_relations_skips_section() {
        let mut bundle = make_bundle(QuestionType::Recall);
        bundle.supporting_relations.clear();
        let context = render_bundle_context(&bundle);

        // Should not have KEY RELATIONSHIPS section
        assert!(!context.contains("KEY RELATIONSHIPS"));
    }

    #[test]
    fn test_render_empty_entities_skips_section() {
        let mut bundle = make_bundle(QuestionType::Recall);
        bundle.supporting_entities.clear();
        let context = render_bundle_context(&bundle);

        // Should not have SUPPORTING ENTITIES section
        assert!(!context.contains("SUPPORTING ENTITIES"));
    }
}
