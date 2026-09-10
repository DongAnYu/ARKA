use super::default_generation_schema::{
    parse_stage_a_output, parse_stage_b_output, stage_a_format_schema, stage_b_format_schema,
    StageAKeyPointsOutput,
};
use super::{LlmService, LlmServiceError, StructuredGenerationRequest};
use crate::models::learning_item::GeneratedItemsOutput;

impl LlmService {
    pub async fn generate_stage_a_key_points(
        &self,
        chunk_markdown: &str,
    ) -> Result<StageAKeyPointsOutput, LlmServiceError> {
        log::info!(
            "LLM Stage A generation started (chunk_chars={})",
            chunk_markdown.chars().count()
        );

        let user_prompt = format_stage_a_user_prompt(chunk_markdown);
        let request = StructuredGenerationRequest {
            stage_label: "Stage A",
            schema_name: "active_recall_key_points",
            system_prompt: STAGE_A_SYSTEM_PROMPT,
            user_prompt: &user_prompt,
            schema: stage_a_format_schema(),
            payload_preview_chars: 600,
        };

        let (parsed, _raw_json, attempts) = self
            .generate_json_with_retries(request, |json_payload| {
                parse_stage_a_output(json_payload).map_err(LlmServiceError::Schema)
            })
            .await?;

        log::info!(
            "LLM Stage A generation finished (key_points={}, attempts={})",
            parsed.key_points.len(),
            attempts
        );
        Ok(parsed)
    }

    pub async fn generate_stage_b_learning_items(
        &self,
        chunk_markdown: &str,
        key_points: &[String],
    ) -> Result<GeneratedItemsOutput, LlmServiceError> {
        log::info!(
            "LLM Stage B generation started (chunk_chars={}, key_points={})",
            chunk_markdown.chars().count(),
            key_points.len()
        );

        let keyed_points = key_points
            .iter()
            .enumerate()
            .map(|(index, knowledge_point)| {
                serde_json::json!({
                    "knowledge_point_id": format!("kp_{}", index + 1),
                    "knowledge_point": knowledge_point,
                })
            })
            .collect::<Vec<_>>();
        let known_point_ids = keyed_points
            .iter()
            .filter_map(|item| item["knowledge_point_id"].as_str())
            .collect::<Vec<_>>();
        let key_points_json =
            serde_json::to_string(&keyed_points).map_err(LlmServiceError::Serialize)?;
        let user_prompt = format_stage_b_user_prompt(chunk_markdown, &key_points_json);
        let request = StructuredGenerationRequest {
            stage_label: "Stage B",
            schema_name: "active_recall_learning_items",
            system_prompt: STAGE_B_SYSTEM_PROMPT,
            user_prompt: &user_prompt,
            schema: stage_b_format_schema(),
            payload_preview_chars: 800,
        };

        let (parsed, _raw_json, attempts) = self
            .generate_json_with_retries(request, |json_payload| {
                parse_stage_b_output(json_payload, &known_point_ids)
                    .map_err(LlmServiceError::Schema)
            })
            .await?;

        log::info!(
            "LLM Stage B generation finished (learning_items={}, attempts={})",
            parsed.items.len(),
            attempts
        );
        Ok(parsed)
    }
}

const STAGE_A_SYSTEM_PROMPT: &str = "You are generating knowledge extraction output for active recall. Return strict JSON only, no markdown, no prose.";

const STAGE_B_SYSTEM_PROMPT: &str = "You are generating atomic learning items for active recall. Each item has one shared answer, a required flashcard, and an optional MCQ variant. Return strict JSON only, no markdown, no prose.";

fn format_stage_a_user_prompt(chunk_markdown: &str) -> String {
    format!(
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
    )
}

fn format_stage_b_user_prompt(chunk_markdown: &str, key_points_json: &str) -> String {
    format!(
        concat!(
            "Given this markdown chunk and extracted key points, create up to 4 atomic learning items for active recall. ",
            "Use at most one learning item per key point and prioritize the strongest distinct concepts. ",
            "Copy knowledge_point_id exactly from the supplied key points; never invent an ID. ",
            "Each item must contain one target, one concise canonical answer, and one required flashcard prompt. ",
            "The target should state the specific knowledge the learner is expected to retain, not merely repeat the question. ",
            "The flashcard prompt must be self-contained and answerable without access to the source chunk. ",
            "The canonical answer must directly and grammatically answer the flashcard prompt, contain enough information to satisfy the target, and remain understandable when read on its own. ",
            "Do not begin or depend on context-only pronouns such as 'it', 'this', 'that', or 'they' when the referenced subject would be unclear without the prompt; briefly restate the subject instead. ",
            "If the prompt asks how, why, or for an explanation, include the essential mechanism, causal link, or reasoning needed to answer that request; do not merely name the outcome, process, or concept. ",
            "Keep the canonical answer concise after completeness is satisfied; do not add unrelated facts or turn it into a paragraph. ",
            "The root answer is shared by both variants; do not put a second correct answer inside the MCQ. ",
            "Add an MCQ only when you can produce exactly 3 plausible, distinct distractors with exactly one defensible answer. ",
            "When the flashcard wording also works for the MCQ, set mcq.prompt to null. Otherwise provide a self-contained MCQ prompt that tests the same target and has the same root answer. ",
            "When an MCQ is unsuitable, set mcq to null and set mcq_omission_reason to a short plain-language explanation of why a good MCQ could not be produced. ",
            "When an MCQ is present, set mcq_omission_reason to null. ",
            "If concepts overlap, generate fewer items instead of duplicates. All content must be grounded in the chunk. ",
            "Include any essential facts from an example, code block, image, diagram, figure, or table directly in the prompt. ",
            "Never refer to unspecified context with wording such as 'the example', 'shown above', 'shown below', or 'the following configuration'. ",
            "Do not rely on distractors to supply context missing from the prompt. Base items only on source information that can be restated faithfully as text. ",
            "Avoid trick wording and semantic duplicates. Before returning, self-check schema compliance, grounding, uniqueness, and non-empty core fields. ",
            "Return exactly this JSON shape and nothing else: {{\"items\":[{{\"knowledge_point_id\":\"kp_1\",\"target\":\"...\",\"answer\":\"...\",\"explanation\":\"... or null\",\"flashcard\":{{\"prompt\":\"...\"}},\"mcq\":{{\"prompt\":null,\"distractors\":[\"...\",\"...\",\"...\"]}},\"mcq_omission_reason\":null}}]}}. ",
            "For a flashcard-only item, use \"mcq\":null and a non-null omission reason.\n\n",
            "Chunk:\n{}\n\n",
            "Key points with IDs JSON:\n{}"
        ),
        chunk_markdown,
        key_points_json,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_b_prompt_requires_source_examples_to_be_self_contained() {
        let chunk = r#"#### HPA Example (`hpa.yaml`)
```yaml
scaleTargetRef:
  kind: Deployment
  name: php-apache
minReplicas: 1
maxReplicas: 10
```"#;
        let prompt = format_stage_b_user_prompt(
            chunk,
            r#"["The HPA scales the php-apache Deployment between 1 and 10 replicas."]"#,
        );

        assert!(prompt.contains("self-contained and answerable without access to the source chunk"));
        assert!(prompt.contains(
            "essential facts from an example, code block, image, diagram, figure, or table"
        ));
        assert!(prompt.contains("Never refer to unspecified context"));
        assert!(prompt.contains("Do not rely on distractors to supply context"));
        assert!(prompt.contains(
            "Base items only on source information that can be restated faithfully as text"
        ));
        assert!(prompt.contains("mcq_omission_reason"));
        assert!(prompt.contains("name: php-apache"));
    }
}
