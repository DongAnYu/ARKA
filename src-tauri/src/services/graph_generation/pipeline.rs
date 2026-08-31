use std::error::Error;
use std::fmt;
use std::fs;
use std::time::Instant;

use serde::Serialize;
use serde_json::Value;
use tokio::task::JoinSet;

use crate::services::chunker::{chunk_markdown, MarkdownChunk};
use crate::services::llm::{LlmService, LlmServiceError, StructuredGenerationRequest};

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
pub const DEFAULT_STAGE_A_CONCURRENCY: usize = 10;
/// Maximum number of independent question-generation requests sent at once.
pub const DEFAULT_STAGE_B_CONCURRENCY: usize = 10;

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
    InvalidStageAConcurrency { value: usize },
    InvalidStageBConcurrency { value: usize },
    StageAWorkerJoin { message: String },
    StageBWorkerJoin { message: String },
}

impl fmt::Display for GraphPipelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "Graph pipeline I/O failed: {err}"),
            Self::InvalidStageAConcurrency { value } => write!(
                f,
                "Graph Stage A max concurrency must be greater than zero; received {value}"
            ),
            Self::InvalidStageBConcurrency { value } => write!(
                f,
                "Graph Stage B max concurrency must be greater than zero; received {value}"
            ),
            Self::StageAWorkerJoin { message } => {
                write!(f, "Graph Stage A worker failed: {message}")
            }
            Self::StageBWorkerJoin { message } => {
                write!(f, "Graph Stage B worker failed: {message}")
            }
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
    run_graph_stage_a_with_progress(input_path, llm, |_| {}).await
}

/// Runs Stage A while reporting per-chunk provider activity.
pub async fn run_graph_stage_a_with_progress<F>(
    input_path: &str,
    llm: &LlmService,
    on_progress: F,
) -> Result<GraphStageAResult, GraphPipelineError>
where
    F: FnMut(GraphStageAProgress),
{
    run_graph_stage_a_with_progress_and_concurrency(
        input_path,
        llm,
        DEFAULT_STAGE_A_CONCURRENCY,
        on_progress,
    )
    .await
}

/// Runs Stage A with an explicit bounded-concurrency limit.
///
/// Provider requests may complete in any order. Results are restored to source
/// chunk order before consolidation, so concurrency cannot change stable IDs,
/// graph order, or report ordering.
pub async fn run_graph_stage_a_with_progress_and_concurrency<F>(
    input_path: &str,
    llm: &LlmService,
    max_concurrency: usize,
    mut on_progress: F,
) -> Result<GraphStageAResult, GraphPipelineError>
where
    F: FnMut(GraphStageAProgress),
{
    if max_concurrency == 0 {
        return Err(GraphPipelineError::InvalidStageAConcurrency {
            value: max_concurrency,
        });
    }

    let note_content = fs::read_to_string(input_path)?;
    let note_title = std::path::Path::new(input_path)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Untitled");
    let chunks = chunk_markdown(input_path, note_title, &note_content);
    let format_schema = stage_a_format_schema();
    let total_chunks = chunks.len();
    let effective_concurrency = max_concurrency.min(total_chunks.max(1));
    on_progress(GraphStageAProgress::ChunksPrepared {
        total_chunks,
        max_concurrency: effective_concurrency,
    });

    let mut chunk_results = vec![None; total_chunks];
    let mut extracted_chunks = vec![None; total_chunks];
    let mut jobs = JoinSet::new();
    let mut next_index = 0usize;

    while next_index < total_chunks && jobs.len() < max_concurrency {
        let chunk = &chunks[next_index];
        on_progress(GraphStageAProgress::ChunkStarted {
            chunk_number: next_index + 1,
            total_chunks,
            heading: chunk.heading.clone(),
        });
        spawn_stage_a_task(&mut jobs, next_index, chunk, llm, &format_schema);
        next_index += 1;
    }

    while let Some(joined) = jobs.join_next().await {
        let (index, chunk_result, extracted, elapsed_ms) = match joined {
            Ok(result) => result,
            Err(error) => {
                jobs.abort_all();
                return Err(GraphPipelineError::StageAWorkerJoin {
                    message: error.to_string(),
                });
            }
        };
        on_progress(GraphStageAProgress::ChunkCompleted {
            chunk_number: index + 1,
            total_chunks,
            heading: chunk_result.heading.clone(),
            status: chunk_result.status.clone(),
            attempts: chunk_result.attempts,
            elapsed_ms,
        });
        chunk_results[index] = Some(chunk_result);
        extracted_chunks[index] = extracted;

        if next_index < total_chunks {
            let chunk = &chunks[next_index];
            on_progress(GraphStageAProgress::ChunkStarted {
                chunk_number: next_index + 1,
                total_chunks,
                heading: chunk.heading.clone(),
            });
            spawn_stage_a_task(&mut jobs, next_index, chunk, llm, &format_schema);
            next_index += 1;
        }
    }

    let chunk_results = chunk_results
        .into_iter()
        .map(|result| result.expect("every Stage A task should produce one ordered result"))
        .collect::<Vec<_>>();
    let extracted_chunks = extracted_chunks.into_iter().flatten().collect::<Vec<_>>();

    let successful_chunks = chunk_results
        .iter()
        .filter(|chunk| chunk.status == "success")
        .count();
    let failed_chunks = chunk_results.len().saturating_sub(successful_chunks);
    on_progress(GraphStageAProgress::Consolidating {
        successful_chunks,
        failed_chunks,
    });
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

type StageAWorkerResult = (
    usize,
    GraphStageAChunkResult,
    Option<ExtractedKnowledge>,
    u128,
);

fn spawn_stage_a_task(
    jobs: &mut JoinSet<StageAWorkerResult>,
    index: usize,
    chunk: &MarkdownChunk,
    llm: &LlmService,
    format_schema: &Value,
) {
    let chunk = chunk.clone();
    let llm = llm.clone();
    let format_schema = format_schema.clone();
    jobs.spawn(async move {
        let started = Instant::now();
        let (result, extracted) = extract_stage_a_chunk(index, chunk, llm, format_schema).await;
        (index, result, extracted, started.elapsed().as_millis())
    });
}

async fn extract_stage_a_chunk(
    index: usize,
    chunk: MarkdownChunk,
    llm: LlmService,
    format_schema: Value,
) -> (GraphStageAChunkResult, Option<ExtractedKnowledge>) {
    let user_prompt =
        format_stage_a_graph_user_prompt(&chunk.content, "(no index context in eval mode)");
    let chunk_id = format!("chunk-{index}");
    let request = StructuredGenerationRequest {
        stage_label: "Graph Stage A",
        schema_name: "graph_stage_a",
        system_prompt: GRAPH_STAGE_A_SYSTEM_PROMPT,
        user_prompt: &user_prompt,
        schema: format_schema,
        payload_preview_chars: 800,
    };

    match llm
        .generate_json_with_retries(request, |raw_json| {
            parse_stage_a_output(raw_json, chunk_id.clone())
                .map_err(|error| LlmServiceError::InvalidOutput(error.to_string()))
        })
        .await
    {
        Ok((extracted, raw_json, attempts)) => {
            let result = GraphStageAChunkResult {
                chunk_index: index,
                heading: chunk.heading,
                content_preview: truncate(&chunk.content, 200),
                status: String::from("success"),
                raw_llm_response: Some(raw_json),
                attempts: Some(attempts),
                entity_count: Some(extracted.raw_entities.len()),
                point_count: Some(extracted.knowledge_points.len()),
                relation_count: Some(relation_count(&extracted)),
                entities: Some(
                    extracted
                        .raw_entities
                        .iter()
                        .map(|entity| entity.name.clone())
                        .collect(),
                ),
                knowledge_points: Some(summarize_points(&extracted)),
                extracted: Some(extracted.clone()),
                error: None,
            };
            (result, Some(extracted))
        }
        Err(error) => (
            GraphStageAChunkResult {
                chunk_index: index,
                heading: chunk.heading,
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
                error: Some(error.to_string()),
            },
            None,
        ),
    }
}

pub async fn run_graph_stage_b(
    stage_a: &GraphStageAResult,
    llm: &LlmService,
) -> Result<GraphStageBResult, GraphPipelineError> {
    run_graph_stage_b_for_graph(&stage_a.graph, llm).await
}

/// Observable milestones for the bounded-concurrent Stage A extraction pass.
///
/// A provider request can legitimately take up to the configured timeout, so
/// callers should surface these events rather than displaying one apparently
/// idle "Running Stage A" message for the entire note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphStageAProgress {
    ChunksPrepared {
        total_chunks: usize,
        max_concurrency: usize,
    },
    ChunkStarted {
        chunk_number: usize,
        total_chunks: usize,
        heading: String,
    },
    ChunkCompleted {
        chunk_number: usize,
        total_chunks: usize,
        heading: String,
        status: String,
        attempts: Option<usize>,
        elapsed_ms: u128,
    },
    Consolidating {
        successful_chunks: usize,
        failed_chunks: usize,
    },
}

/// Observable milestones for bounded-concurrent question generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphStageBProgress {
    BundlesPrepared {
        total_bundles: usize,
        max_concurrency: usize,
    },
    BundleCompleted {
        bundle_number: usize,
        total_bundles: usize,
        status: String,
        in_flight_requests: usize,
        elapsed_ms: u128,
    },
}

/// Runs Stage B against the authoritative graph supplied by the caller.
///
/// The graph E2E evaluator uses this entry point after entity resolution so its
/// bundles match the app pipeline's `consolidate → resolve → build index` order.
pub async fn run_graph_stage_b_for_graph(
    graph: &PropositionGraph,
    llm: &LlmService,
) -> Result<GraphStageBResult, GraphPipelineError> {
    run_graph_stage_b_for_graph_with_progress_and_concurrency(
        graph,
        llm,
        DEFAULT_STAGE_B_CONCURRENCY,
        |_| {},
    )
    .await
}

/// Runs Stage B with bounded provider concurrency and stable output ordering.
///
/// Bundles are independent LLM requests, so they may safely execute in
/// parallel. Results are written back to their original bundle positions;
/// provider completion order therefore cannot change question or report order.
pub async fn run_graph_stage_b_for_graph_with_progress_and_concurrency<F>(
    graph: &PropositionGraph,
    llm: &LlmService,
    max_concurrency: usize,
    mut on_progress: F,
) -> Result<GraphStageBResult, GraphPipelineError>
where
    F: FnMut(GraphStageBProgress),
{
    if max_concurrency == 0 {
        return Err(GraphPipelineError::InvalidStageBConcurrency {
            value: max_concurrency,
        });
    }

    let index = build_index(graph);
    let bundles = assemble_bundles(graph, &index);
    let total_bundles = bundles.len();
    let effective_concurrency = max_concurrency.min(total_bundles.max(1));
    on_progress(GraphStageBProgress::BundlesPrepared {
        total_bundles,
        max_concurrency: effective_concurrency,
    });

    let mut ordered_items = std::iter::repeat_with(|| None)
        .take(total_bundles)
        .collect::<Vec<Option<GraphStageBItemResult>>>();
    let mut jobs = JoinSet::new();
    let mut next_index = 0usize;

    while next_index < total_bundles && jobs.len() < max_concurrency {
        spawn_stage_b_task(&mut jobs, next_index, &bundles[next_index], llm);
        next_index += 1;
    }

    while let Some(joined) = jobs.join_next().await {
        let (bundle_index, bundle, result, elapsed_ms) = match joined {
            Ok(result) => result,
            Err(error) => {
                jobs.abort_all();
                return Err(GraphPipelineError::StageBWorkerJoin {
                    message: error.to_string(),
                });
            }
        };
        let item = match result {
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
        let status = item.status.clone();
        ordered_items[bundle_index] = Some(item);

        if next_index < total_bundles {
            spawn_stage_b_task(&mut jobs, next_index, &bundles[next_index], llm);
            next_index += 1;
        }
        on_progress(GraphStageBProgress::BundleCompleted {
            bundle_number: bundle_index + 1,
            total_bundles,
            status,
            in_flight_requests: jobs.len(),
            elapsed_ms,
        });
    }

    let items = ordered_items
        .into_iter()
        .map(|item| item.expect("every Stage B task should produce one ordered result"))
        .collect::<Vec<_>>();

    let successful_mcqs = items.iter().filter(|item| item.status == "success").count();
    let failed_mcqs = items.len().saturating_sub(successful_mcqs);

    Ok(GraphStageBResult {
        total_bundles: items.len(),
        successful_mcqs,
        failed_mcqs,
        items,
    })
}

type StageBWorkerResult = (
    usize,
    GraphContextBundle,
    Result<GeneratedMCQ, LlmServiceError>,
    u128,
);

fn spawn_stage_b_task(
    jobs: &mut JoinSet<StageBWorkerResult>,
    bundle_index: usize,
    bundle: &GraphContextBundle,
    llm: &LlmService,
) {
    let bundle = bundle.clone();
    let llm = llm.clone();
    jobs.spawn(async move {
        let started = Instant::now();
        let result = generate_mcq(&bundle, &llm).await;
        (bundle_index, bundle, result, started.elapsed().as_millis())
    });
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::time::{sleep, Duration};

    use super::*;
    use crate::services::graph_generation::types::{
        EntityNode, KnowledgePoint, KnowledgeType, PropositionGraph,
    };
    use crate::services::llm::{LlmConfig, LlmProvider};

    #[tokio::test]
    async fn stage_a_bounds_concurrency_and_restores_source_order() {
        let note_path = temporary_note_path();
        std::fs::write(&note_path, test_note()).expect("temporary note should write");
        let (base_url, maximum_active) = delayed_ollama_server(3).await;
        let llm = LlmService::new(LlmConfig {
            provider: LlmProvider::Ollama,
            base_url,
            model: String::from("stage-a-test-model"),
            timeout_secs: 5,
            api_key: None,
        })
        .expect("test LLM should build");

        let result = run_graph_stage_a_with_progress_and_concurrency(
            note_path.to_str().expect("temporary path should be UTF-8"),
            &llm,
            2,
            |_| {},
        )
        .await
        .expect("concurrent Stage A should succeed");
        let _ = std::fs::remove_file(&note_path);

        assert_eq!(maximum_active.load(Ordering::SeqCst), 2);
        assert_eq!(result.successful_chunks, 3);
        assert_eq!(
            result
                .chunks
                .iter()
                .map(|chunk| chunk.chunk_index)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(
            result
                .chunks
                .iter()
                .map(|chunk| chunk.heading.as_str())
                .collect::<Vec<_>>(),
            vec!["First", "Second", "Third"]
        );
    }

    #[tokio::test]
    async fn stage_b_bounds_concurrency_and_restores_bundle_order() {
        let graph = test_stage_b_graph();
        let (base_url, maximum_active) = delayed_stage_b_server(3).await;
        let llm = LlmService::new(LlmConfig {
            provider: LlmProvider::Ollama,
            base_url,
            model: String::from("stage-b-test-model"),
            timeout_secs: 5,
            api_key: None,
        })
        .expect("test LLM should build");
        let mut progress = Vec::new();

        let result =
            run_graph_stage_b_for_graph_with_progress_and_concurrency(&graph, &llm, 2, |event| {
                progress.push(event)
            })
            .await
            .expect("concurrent Stage B should succeed");

        assert_eq!(maximum_active.load(Ordering::SeqCst), 2);
        assert_eq!(result.successful_mcqs, 3);
        assert_eq!(
            result
                .items
                .iter()
                .map(|item| item.bundle_index)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(
            result
                .items
                .iter()
                .map(|item| item.bundle.root_point.id.as_str())
                .collect::<Vec<_>>(),
            vec!["point-first", "point-second", "point-third"]
        );
        assert!(matches!(
            progress.first(),
            Some(GraphStageBProgress::BundlesPrepared {
                total_bundles: 3,
                max_concurrency: 2
            })
        ));
        assert!(matches!(
            progress.last(),
            Some(GraphStageBProgress::BundleCompleted {
                in_flight_requests: 0,
                ..
            })
        ));
    }

    fn temporary_note_path() -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should follow Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("arka-stage-a-{}-{nonce}.md", std::process::id()))
    }

    fn test_note() -> String {
        ["First", "Second", "Third"]
            .into_iter()
            .map(|heading| {
                format!(
                    "## {heading}\n\n{heading} marker. {}",
                    "This section contains independent extraction evidence. ".repeat(5)
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    fn test_stage_b_graph() -> PropositionGraph {
        let markers = ["First", "Second", "Third"];
        PropositionGraph {
            entities: markers
                .iter()
                .enumerate()
                .map(|(index, marker)| EntityNode {
                    id: format!("entity-{}", marker.to_ascii_lowercase()),
                    canonical_name: format!("{marker} entity"),
                    aliases: vec![format!("{marker} entity")],
                    chunk_ids: vec![format!("chunk-{index}")],
                })
                .collect(),
            knowledge_points: markers
                .iter()
                .enumerate()
                .map(|(index, marker)| KnowledgePoint {
                    id: format!("point-{}", marker.to_ascii_lowercase()),
                    point: format!("{marker} Stage B marker has enough evidence for a question."),
                    knowledge_type: KnowledgeType::Fact,
                    chunk_id: format!("chunk-{index}"),
                    raw_entity_names: vec![format!("{marker} entity")],
                    entity_ids: vec![format!("entity-{}", marker.to_ascii_lowercase())],
                    raw_relations: Vec::new(),
                })
                .collect(),
            relations: Vec::new(),
        }
    }

    async fn delayed_ollama_server(expected_requests: usize) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener should bind");
        let address = listener.local_addr().expect("listener should have address");
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let active_for_server = Arc::clone(&active);
        let maximum_for_server = Arc::clone(&maximum);

        tokio::spawn(async move {
            for _ in 0..expected_requests {
                let (socket, _) = listener.accept().await.expect("request should connect");
                let active = Arc::clone(&active_for_server);
                let maximum = Arc::clone(&maximum_for_server);
                tokio::spawn(async move {
                    let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                    maximum.fetch_max(current, Ordering::SeqCst);
                    serve_stage_a_response(socket, &active).await;
                });
            }
        });

        (format!("http://{address}"), maximum)
    }

    async fn delayed_stage_b_server(expected_requests: usize) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener should bind");
        let address = listener.local_addr().expect("listener should have address");
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let active_for_server = Arc::clone(&active);
        let maximum_for_server = Arc::clone(&maximum);

        tokio::spawn(async move {
            for _ in 0..expected_requests {
                let (socket, _) = listener.accept().await.expect("request should connect");
                let active = Arc::clone(&active_for_server);
                let maximum = Arc::clone(&maximum_for_server);
                tokio::spawn(async move {
                    let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                    maximum.fetch_max(current, Ordering::SeqCst);
                    serve_stage_b_response(socket, &active).await;
                });
            }
        });

        (format!("http://{address}"), maximum)
    }

    async fn serve_stage_a_response(mut socket: TcpStream, active: &AtomicUsize) {
        let request = read_http_request(&mut socket).await;
        if request.contains("First marker") {
            sleep(Duration::from_millis(150)).await;
        } else {
            sleep(Duration::from_millis(25)).await;
        }

        let extraction = json!({
            "entities": ["Test entity"],
            "knowledge_points": [{
                "point": "The test entity has extraction evidence.",
                "knowledge_type": "fact",
                "raw_entity_names": ["Test entity"],
                "raw_relations": []
            }]
        })
        .to_string();
        let body = json!({
            "message": {
                "role": "assistant",
                "content": extraction
            }
        })
        .to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        socket
            .write_all(response.as_bytes())
            .await
            .expect("test response should write");
        active.fetch_sub(1, Ordering::SeqCst);
    }

    async fn serve_stage_b_response(mut socket: TcpStream, active: &AtomicUsize) {
        let request = read_http_request(&mut socket).await;
        if request.contains("First Stage B marker") {
            sleep(Duration::from_millis(150)).await;
        } else {
            sleep(Duration::from_millis(25)).await;
        }

        let mcq = json!({
            "question": "Which statement matches the supplied knowledge point?",
            "options": [
                "The supplied statement is correct",
                "The first distractor is correct",
                "The second distractor is correct",
                "The third distractor is correct"
            ],
            "correct_index": 0,
            "explanation": "The first option directly restates the evidence supplied in the graph bundle."
        })
        .to_string();
        let body = json!({
            "message": {
                "role": "assistant",
                "content": mcq
            }
        })
        .to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        socket
            .write_all(response.as_bytes())
            .await
            .expect("test response should write");
        active.fetch_sub(1, Ordering::SeqCst);
    }

    async fn read_http_request(socket: &mut TcpStream) -> String {
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1024];

        loop {
            let bytes_read = socket.read(&mut buffer).await.expect("request should read");
            if bytes_read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..bytes_read]);

            let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            if request.len() >= header_end + 4 + content_length {
                break;
            }
        }

        String::from_utf8_lossy(&request).into_owned()
    }
}
