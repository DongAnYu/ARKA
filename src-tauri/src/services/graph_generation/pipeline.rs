use std::error::Error;
use std::fmt;
use std::fs;

use serde::Serialize;

use crate::services::chunker::chunk_markdown;
use crate::services::llm::{JsonGenerationRequest, LlmService, LlmServiceError};

use super::bundle_builder::assemble_bundles;
use super::consolidator::{consolidate, validate_graph};
use super::graph_index::build_index;
use super::stage_a_prompt::format_stage_a_graph_user_prompt;
use super::stage_a_schema::{parse_stage_a_output, stage_a_format_schema};
use super::stage_b_generation::generate_mcq;
use super::stage_b_schema::GeneratedMCQ;
use super::types::{ExtractedKnowledge, GraphContextBundle, PropositionGraph};

const GRAPH_STAGE_A_SYSTEM_PROMPT: &str =
    "You are a knowledge graph extraction specialist. Output only valid JSON.";

#[derive(Debug, Serialize)]
pub struct GraphStageAResult {
    pub note_path: String,
    pub note_title: String,
    pub total_chunks: usize,
    pub successful_chunks: usize,
    pub failed_chunks: usize,
    pub chunks: Vec<GraphStageAChunkResult>,
    pub graph: PropositionGraph,
    pub validation_violations: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphStageAChunkResult {
    pub chunk_index: usize,
    pub heading: String,
    pub content_preview: String,
    pub status: String,
    pub raw_llm_response: Option<String>,
    pub attempts: Option<usize>,
    pub entity_count: Option<usize>,
    pub point_count: Option<usize>,
    pub relation_count: Option<usize>,
    pub entities: Option<Vec<String>>,
    pub knowledge_points: Option<Vec<GraphPointSummary>>,
    #[serde(skip_serializing)]
    pub extracted: Option<ExtractedKnowledge>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphPointSummary {
    pub point: String,
    pub knowledge_type: String,
    pub raw_entity_names: Vec<String>,
    pub relation_count: usize,
}

#[derive(Debug, Serialize)]
pub struct GraphStageBResult {
    pub total_bundles: usize,
    pub successful_mcqs: usize,
    pub failed_mcqs: usize,
    pub items: Vec<GraphStageBItemResult>,
}

#[derive(Debug, Serialize)]
pub struct GraphStageBItemResult {
    pub bundle_index: usize,
    pub bundle: GraphContextBundle,
    pub status: String,
    pub mcq: Option<GeneratedMCQ>,
    pub error: Option<String>,
}

#[derive(Debug)]
pub enum GraphPipelineError {
    Io(std::io::Error),
}

impl fmt::Display for GraphPipelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "Graph pipeline I/O failed: {err}"),
        }
    }
}

impl Error for GraphPipelineError {}

impl From<std::io::Error> for GraphPipelineError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

pub async fn run_graph_stage_a(
    input_path: &str,
    llm: &LlmService,
) -> Result<GraphStageAResult, GraphPipelineError> {
    let note_content = fs::read_to_string(input_path)?;
    let note_title = std::path::Path::new(input_path)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Untitled");
    let chunks = chunk_markdown(input_path, note_title, &note_content);
    let format_schema = stage_a_format_schema();

    let mut chunk_results = Vec::new();
    let mut extracted_chunks = Vec::new();

    for (idx, chunk) in chunks.iter().enumerate() {
        let user_prompt =
            format_stage_a_graph_user_prompt(&chunk.content, "(no index context in eval mode)");
        let chunk_id = format!("chunk-{}", idx);
        let request = JsonGenerationRequest {
            stage_label: "Graph Stage A",
            system_prompt: GRAPH_STAGE_A_SYSTEM_PROMPT,
            user_prompt: &user_prompt,
            format_schema: format_schema.clone(),
            payload_preview_chars: 800,
        };

        let llm_result = llm
            .generate_json_with_retries(request, |raw_json| {
                parse_stage_a_output(raw_json, chunk_id.clone())
                    .map_err(|err| LlmServiceError::InvalidOutput(err.to_string()))
            })
            .await;

        let chunk_result = match llm_result {
            Ok((extracted, raw_json, attempts)) => {
                let point_summaries = summarize_points(&extracted);
                let total_relations = relation_count(&extracted);
                let result = GraphStageAChunkResult {
                    chunk_index: idx,
                    heading: chunk.heading.clone(),
                    content_preview: truncate(&chunk.content, 200),
                    status: String::from("success"),
                    raw_llm_response: Some(raw_json),
                    attempts: Some(attempts),
                    entity_count: Some(extracted.raw_entities.len()),
                    point_count: Some(extracted.knowledge_points.len()),
                    relation_count: Some(total_relations),
                    entities: Some(
                        extracted
                            .raw_entities
                            .iter()
                            .map(|entity| entity.name.clone())
                            .collect(),
                    ),
                    knowledge_points: Some(point_summaries),
                    extracted: Some(extracted.clone()),
                    error: None,
                };
                extracted_chunks.push(extracted);
                result
            }
            Err(err) => GraphStageAChunkResult {
                chunk_index: idx,
                heading: chunk.heading.clone(),
                content_preview: truncate(&chunk.content, 200),
                status: String::from("llm_error"),
                raw_llm_response: None,
                attempts: None,
                entity_count: None,
                point_count: None,
                relation_count: None,
                entities: None,
                knowledge_points: None,
                extracted: None,
                error: Some(err.to_string()),
            },
        };

        chunk_results.push(chunk_result);
    }

    let successful_chunks = chunk_results
        .iter()
        .filter(|chunk| chunk.status == "success")
        .count();
    let failed_chunks = chunk_results.len().saturating_sub(successful_chunks);
    let graph = consolidate(extracted_chunks);
    let validation_violations = validate_graph(&graph);

    Ok(GraphStageAResult {
        note_path: input_path.to_string(),
        note_title: note_title.to_string(),
        total_chunks: chunks.len(),
        successful_chunks,
        failed_chunks,
        chunks: chunk_results,
        graph,
        validation_violations,
    })
}

pub async fn run_graph_stage_b(
    stage_a: &GraphStageAResult,
    llm: &LlmService,
) -> Result<GraphStageBResult, GraphPipelineError> {
    let index = build_index(&stage_a.graph);
    let bundles = assemble_bundles(&stage_a.graph, &index);
    let mut items = Vec::with_capacity(bundles.len());

    for (bundle_index, bundle) in bundles.into_iter().enumerate() {
        let result = match generate_mcq(&bundle, llm).await {
            Ok(mcq) => GraphStageBItemResult {
                bundle_index,
                bundle,
                status: String::from("success"),
                mcq: Some(mcq),
                error: None,
            },
            Err(err) => GraphStageBItemResult {
                bundle_index,
                bundle,
                status: String::from("llm_error"),
                mcq: None,
                error: Some(err.to_string()),
            },
        };
        items.push(result);
    }

    let successful_mcqs = items.iter().filter(|item| item.status == "success").count();
    let failed_mcqs = items.len().saturating_sub(successful_mcqs);

    Ok(GraphStageBResult {
        total_bundles: items.len(),
        successful_mcqs,
        failed_mcqs,
        items,
    })
}

fn summarize_points(extracted: &ExtractedKnowledge) -> Vec<GraphPointSummary> {
    extracted
        .knowledge_points
        .iter()
        .map(|kp| GraphPointSummary {
            point: kp.point.clone(),
            knowledge_type: format!("{:?}", kp.knowledge_type),
            raw_entity_names: kp.raw_entity_names.clone(),
            relation_count: kp.raw_relations.len(),
        })
        .collect()
}

fn relation_count(extracted: &ExtractedKnowledge) -> usize {
    extracted
        .knowledge_points
        .iter()
        .map(|kp| kp.raw_relations.len())
        .sum()
}

fn truncate(s: &str, max_chars: usize) -> String {
    let chars_count = s.chars().count();
    if chars_count <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars).collect();
        format!("{}...", truncated)
    }
}
