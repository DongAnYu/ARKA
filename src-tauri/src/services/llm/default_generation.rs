use super::default_generation_schema::{
    parse_stage_a_output, parse_stage_b_output, stage_a_format_schema, stage_b_format_schema,
    StageAKeyPointsOutput, StageBMcqOutput,
};
use super::{LlmService, LlmServiceError, StructuredGenerationRequest};

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

        let key_points_json =
            serde_json::to_string(key_points).map_err(LlmServiceError::Serialize)?;
        let user_prompt = format_stage_b_user_prompt(chunk_markdown, &key_points_json);
        let request = StructuredGenerationRequest {
            stage_label: "Stage B",
            schema_name: "active_recall_questions",
            system_prompt: STAGE_B_SYSTEM_PROMPT,
            user_prompt: &user_prompt,
            schema: stage_b_format_schema(),
            payload_preview_chars: 800,
        };

        let (parsed, _raw_json, attempts) = self
            .generate_json_with_retries(request, |json_payload| {
                parse_stage_b_output(json_payload).map_err(LlmServiceError::Schema)
            })
            .await?;

        log::info!(
            "LLM Stage B generation finished (questions={}, attempts={})",
            parsed.questions.len(),
            attempts
        );
        Ok(parsed)
    }
}

const STAGE_A_SYSTEM_PROMPT: &str = "You are generating knowledge extraction output for active recall. Return strict JSON only, no markdown, no prose.";

const STAGE_B_SYSTEM_PROMPT: &str = "You are generating multiple-choice questions for active recall. Return strict JSON only, no markdown, no prose.";

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
            "Given this markdown chunk and extracted key points, create 1-4 MCQs for active recall. ",
            "Each question must test a different concept and must not paraphrase another question. ",
            "Use at most one question per key point and prioritize the strongest distinct concepts. ",
            "If concepts overlap, generate fewer questions instead of duplicates. ",
            "All questions must be grounded in the chunk content. ",
            "Every question stem must be self-contained and answerable without access to the source chunk. ",
            "Include any essential facts from an example, code block, image, diagram, figure, or table directly in the question stem. ",
            "Never refer to unspecified context with wording such as 'the example', 'shown above', 'shown below', or 'the following configuration'. ",
            "Do not rely on the answer options to supply context missing from the question stem. ",
            "Base questions only on source information that can be restated faithfully as text. ",
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
        assert!(prompt.contains("Do not rely on the answer options to supply context"));
        assert!(prompt.contains(
            "Base questions only on source information that can be restated faithfully as text"
        ));
        assert!(prompt.contains("name: php-apache"));
    }
}
