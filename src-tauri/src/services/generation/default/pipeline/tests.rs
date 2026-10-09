use super::*;
use crate::models::learning_item::GenerationPipeline;
use crate::services::generation::job::next_preview_job_id;
use crate::services::llm::{LlmConfig, LlmProvider};
use serde_json::{json, Value};
use std::sync::atomic::AtomicUsize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{sleep, Duration};

struct Reply {
    payload: Value,
    status: u16,
    delay: Duration,
}
impl Reply {
    fn json(payload: Value) -> Self {
        Self {
            payload,
            status: 200,
            delay: Duration::ZERO,
        }
    }
}

struct MockProvider {
    llm: Arc<LlmService>,
    requests: Arc<Mutex<Vec<Value>>>,
    maximum_active: Arc<AtomicUsize>,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for MockProvider {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn read_request(stream: &mut TcpStream) -> Value {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    let (start, length) = loop {
        let count = stream.read(&mut buffer).await.unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..end]);
            let length = headers
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .unwrap()
                .1
                .trim()
                .parse::<usize>()
                .unwrap();
            break (end + 4, length);
        }
    };
    while bytes.len() < start + length {
        let count = stream.read(&mut buffer).await.unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&buffer[..count]);
    }
    serde_json::from_slice(&bytes[start..start + length]).unwrap()
}

impl MockProvider {
    async fn new(handler: impl Fn(&Value) -> Reply + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        let active = Arc::new(AtomicUsize::new(0));
        let maximum_active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::clone(&maximum_active);
        let handler = Arc::new(handler);
        let server = tokio::spawn(async move {
            let mut workers = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (mut stream, _) = accepted.unwrap();
                        let handler = Arc::clone(&handler); let recorded = Arc::clone(&recorded);
                        let active = Arc::clone(&active); let maximum = Arc::clone(&maximum);
                        workers.spawn(async move {
                            let request = read_request(&mut stream).await;
                            recorded.lock().unwrap().push(request.clone());
                            let count = active.fetch_add(1, Ordering::Relaxed) + 1; maximum.fetch_max(count, Ordering::Relaxed);
                            let reply = handler(&request); sleep(reply.delay).await;
                            let body = if reply.status == 200 {
                                json!({"message":{"role":"assistant","content":reply.payload.to_string()}})
                            } else { json!({"error":"Fixture request rejected"}) }.to_string();
                            let response = format!("HTTP/1.1 {} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", reply.status,body.len(),body);
                            let _ = stream.write_all(response.as_bytes()).await;
                            active.fetch_sub(1, Ordering::Relaxed);
                        });
                    }
                    joined = workers.join_next(), if !workers.is_empty() => { joined.unwrap().unwrap(); }
                }
            }
        });
        Self {
            llm: Arc::new(
                LlmService::new(LlmConfig {
                    provider: LlmProvider::Ollama,
                    base_url: format!("http://{address}"),
                    model: "fixture".into(),
                    timeout_secs: 5,
                    api_key: None,
                })
                .unwrap(),
            ),
            requests,
            maximum_active,
            server,
        }
    }
}

fn stage(request: &Value) -> &'static str {
    let properties = &request["format"]["properties"];
    if properties.get("key_points").is_some() {
        "extract"
    } else if properties.get("centrality").is_some() {
        "assess"
    } else if properties.get("items").is_some() {
        "generate"
    } else {
        panic!("Unexpected request contract");
    }
}
fn prompt(request: &Value) -> &str {
    request["messages"][1]["content"].as_str().unwrap()
}
fn evidence(request: &Value) -> Value {
    serde_json::from_str(prompt(request).split_once("Evidence JSON:\n").unwrap().1).unwrap()
}
fn keyed(request: &Value) -> Vec<Value> {
    serde_json::from_str(
        prompt(request)
            .split_once("Key points with IDs JSON:\n")
            .unwrap()
            .1,
    )
    .unwrap()
}
fn point(chunk: usize, position: usize) -> String {
    if position == 1 {
        format!("  point-{chunk}-{position}\nwith `code`  ")
    } else {
        format!("point-{chunk}-{position}")
    }
}
fn chunk_index(request: &Value) -> usize {
    for index in 0..10 {
        if prompt(request).contains(&format!("original-chunk-{index}")) {
            return index;
        }
    }
    panic!("Original chunk missing from request");
}
fn extracted(index: usize, count: usize) -> Value {
    json!({"key_points":(0..count).map(|position| json!({"knowledge_point":point(index,position)})).collect::<Vec<_>>()})
}
fn assessed(score: u8, suitable: bool, needs_source: bool) -> Value {
    json!({"centrality":score,"explanatory_value":score,"foundational_value":score,"distinct_contribution":score,"application_value":score,
        "suitable_learning_target":suitable,"requires_source_context":needs_source,"justification":"Grounded fixture assessment."})
}
fn generated(points: &[Value]) -> Value {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../models/learning_item/fixtures/generated-pair.json"
    ))
    .unwrap();
    json!({"items":points.iter().map(|point| {
        let mut item = fixture["items"][0].clone();
        item["knowledge_point_id"] = point["knowledge_point_id"].clone();
        item["target"] = point["knowledge_point"].clone();
        item
    }).collect::<Vec<_>>()})
}
fn job(chunk_count: usize, maximum: usize) -> (Arc<PreviewJob>, Vec<NoteGenerationReport>) {
    let options = serde_json::from_value(json!({"max_learning_items":maximum})).unwrap();
    let (mut job, _) = prepare_job(
        next_preview_job_id(),
        &[],
        options,
        GenerationMetadata {
            model: Some("fixture".into()),
            provider: Some("ollama".into()),
            pipeline: Some(GenerationPipeline::Chunk),
            generated_at: None,
        },
    );
    let chunks = (0..chunk_count)
        .map(|index| MarkdownChunk {
            note_path: "note.md".into(),
            note_title: "Search".into(),
            heading: "Repeated section heading".into(),
            section_index: index,
            chunk_index: 0,
            start_line: index * 10 + 1,
            end_line: index * 10 + 10,
            content: format!("Original source original-chunk-{index}."),
        })
        .collect();
    Arc::get_mut(&mut job).unwrap().default_generation =
        Some(Arc::new(DefaultGenerationJob::new(options, chunks)));
    {
        let mut snapshot = job.snapshot.lock().unwrap();
        snapshot.total_notes = 1;
        snapshot.notes_with_chunks = 1;
        snapshot.total_chunks = chunk_count;
    }
    (
        job,
        vec![NoteGenerationReport {
            note_path: "note.md".into(),
            note_title: "Search".into(),
            total_chunks: chunk_count,
        }],
    )
}

#[tokio::test]
async fn complete_note_is_assessed_before_selected_only_generation_with_original_source_mapping() {
    for maximum in [1, 2] {
        let provider = MockProvider::new(|request| match stage(request) {
            "extract" => Reply::json(extracted(chunk_index(request), 3)),
            "assess" => {
                let text = evidence(request)["candidate_knowledge_point"]
                    .as_str()
                    .unwrap()
                    .to_string();
                let score = if text.contains("point-0-1") {
                    5
                } else if text.contains("point-1-2") {
                    4
                } else {
                    1
                };
                let mut reply = Reply::json(assessed(score, true, false));
                if score == 5 {
                    reply.delay = Duration::from_millis(60);
                }
                reply
            }
            "generate" => Reply::json(generated(&keyed(request))),
            _ => unreachable!(),
        })
        .await;
        let (job, reports) = job(2, maximum);
        run(Arc::clone(&job), Arc::clone(&provider.llm), 2, reports).await;
        let snapshot = job.snapshot.lock().unwrap().clone();
        assert!(snapshot.error.is_none());
        assert_eq!(snapshot.progress_percent, 100);
        let report = snapshot.default_selection.unwrap();
        assert_eq!(report.assessed_count, 6);
        assert_eq!(report.selected_count, maximum);
        assert_eq!(report.generated_count, maximum);
        assert_eq!(snapshot.mcq_generated, maximum); // A flashcard/MCQ pair counts once.
        let state = job.default_generation.as_ref().unwrap();
        assert_eq!(state.candidates().unwrap().len(), 6);
        let outcomes = state.assessments().unwrap();
        let strongest = state
            .candidates()
            .unwrap()
            .into_iter()
            .find(|point| point.source_chunk_index == 0 && point.point_position == 1)
            .unwrap();
        assert_eq!(
            outcomes[&strongest.id].result.as_ref().unwrap().centrality,
            5
        );
        let summary = snapshot.summary.unwrap();
        assert_eq!(
            summary.chunk_previews[0].llm_result.key_points[1],
            point(0, 1)
        );
        let drafts = summary
            .chunk_previews
            .iter()
            .flat_map(|preview| preview.llm_result.items.iter())
            .collect::<Vec<_>>();
        assert_eq!(drafts[0].source.knowledge_point, point(0, 1));
        assert_eq!(drafts[0].source.start_line, 1);
        assert_eq!(drafts[0].content.knowledge_point_id, "kp_1");
        if maximum == 2 {
            assert_eq!(drafts[1].source.knowledge_point, point(1, 2));
            assert_eq!(drafts[1].source.start_line, 11);
        }
        let requests = provider.requests.lock().unwrap();
        assert_eq!(requests.len(), 2 + 6 + maximum);
        assert!(requests[..2]
            .iter()
            .all(|request| stage(request) == "extract"));
        assert!(requests[2..8]
            .iter()
            .all(|request| stage(request) == "assess"));
        for request in requests
            .iter()
            .filter(|request| stage(request) == "generate")
        {
            assert_eq!(keyed(request).len(), 1);
            assert_eq!(request["format"]["properties"]["items"]["maxItems"], 1);
            assert!(keyed(request)[0]["knowledge_point"]
                .as_str()
                .unwrap()
                .contains(if chunk_index(request) == 0 {
                    "point-0-1"
                } else {
                    "point-1-2"
                }));
        }
        assert!(provider.maximum_active.load(Ordering::Relaxed) <= 2);
    }
}

#[tokio::test]
async fn source_recovery_is_bounded_and_ambiguous_or_unsuitable_candidates_are_excluded() {
    let provider = MockProvider::new(|request| match stage(request) {
        "extract" => Reply::json(extracted(0, 3)),
        "assess" => {
            let evidence = evidence(request);
            let text = evidence["candidate_knowledge_point"].as_str().unwrap();
            let unresolved = text.contains("point-0-1");
            let suitable = !text.contains("point-0-2");
            Reply::json(assessed(
                4,
                suitable,
                suitable && (unresolved || evidence.get("original_chunk").is_none()),
            ))
        }
        "generate" => Reply::json(generated(&keyed(request))),
        _ => unreachable!(),
    })
    .await;
    let (job, reports) = job(1, 3);
    run(Arc::clone(&job), Arc::clone(&provider.llm), 2, reports).await;
    let snapshot = job.snapshot.lock().unwrap();
    let report = snapshot.default_selection.as_ref().unwrap();
    assert_eq!(report.assessed_count, 3);
    assert_eq!(report.source_context_reassessments, 2);
    assert_eq!(report.unresolved_assessments, 1);
    assert_eq!(report.unsuitable_count, 1);
    assert_eq!(report.selected_count, 1);
    assert_eq!(report.generated_count, 1);
    assert_eq!(snapshot.warnings.len(), 1);
    assert!(report
        .shortfall
        .iter()
        .any(|detail| detail.reason == "unresolved_assessments"));
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 7);
    let recovered = requests
        .iter()
        .filter(|request| {
            stage(request) == "assess" && evidence(request).get("original_chunk").is_some()
        })
        .collect::<Vec<_>>();
    assert_eq!(recovered.len(), 2);
    for request in recovered {
        assert!(evidence(request)["original_chunk"]
            .as_str()
            .unwrap()
            .contains("original-chunk-0"));
    }
}

#[tokio::test]
async fn assessment_failures_and_empty_extraction_never_generate_unranked_items() {
    for empty in [false, true] {
        let provider = MockProvider::new(move |request| match stage(request) {
            "extract" => Reply::json(extracted(0, if empty { 0 } else { 3 })),
            "assess" => Reply {
                payload: Value::Null,
                status: 400,
                delay: Duration::ZERO,
            },
            "generate" => panic!("Unranked generation must not run"),
            _ => unreachable!(),
        })
        .await;
        let (job, reports) = job(1, 2);
        run(Arc::clone(&job), Arc::clone(&provider.llm), 2, reports).await;
        let snapshot = job.snapshot.lock().unwrap();
        let report = snapshot.default_selection.as_ref().unwrap();
        assert!(snapshot.is_finished);
        assert!(snapshot.error.is_none());
        assert_eq!(report.selected_count, 0);
        assert_eq!(report.generated_count, 0);
        assert_eq!(report.unresolved_assessments, if empty { 0 } else { 3 });
        assert!(provider
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|request| stage(request) != "generate"));
    }
}

#[tokio::test]
async fn capacity_and_stage_b_failures_or_omissions_are_reported_without_replenishment() {
    let provider = MockProvider::new(|request| match stage(request) {
        "extract" => Reply::json(extracted(chunk_index(request), 6)),
        "assess" => Reply::json(assessed(4, true, false)),
        "generate" if chunk_index(request) == 0 => Reply::json(generated(&keyed(request)[..1])),
        "generate" => Reply {
            payload: Value::Null,
            status: 400,
            delay: Duration::ZERO,
        },
        _ => unreachable!(),
    })
    .await;
    let (job, reports) = job(2, 20);
    run(Arc::clone(&job), Arc::clone(&provider.llm), 2, reports).await;
    let snapshot = job.snapshot.lock().unwrap();
    let report = snapshot.default_selection.as_ref().unwrap();
    assert_eq!(report.selected_count, 8);
    assert_eq!(report.chunk_capacity_excluded, 4);
    assert_eq!(report.generated_count, 1);
    assert_eq!(report.generation_omitted_count, 3);
    assert_eq!(report.generation_failed_targets, 4);
    let reasons = report
        .shortfall
        .iter()
        .map(|detail| detail.reason)
        .collect::<Vec<_>>();
    for reason in [
        "chunk_capacity",
        "generation_failures",
        "generation_omissions",
    ] {
        assert!(reasons.contains(&reason));
    }
    assert_eq!(snapshot.ready_previews.len(), 1);
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 16);
    for request in requests
        .iter()
        .filter(|request| stage(request) == "generate")
    {
        assert_eq!(keyed(request).len(), 4);
    }
}

#[tokio::test]
async fn invalid_model_score_uses_structured_retry_instead_of_default_scores() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&attempts);
    let provider = MockProvider::new(move |request| match stage(request) {
        "extract" => Reply::json(extracted(0, 1)),
        "assess" => Reply::json(assessed(
            if observed.fetch_add(1, Ordering::Relaxed) == 0 {
                0
            } else {
                5
            },
            true,
            false,
        )),
        "generate" => Reply::json(generated(&keyed(request))),
        _ => unreachable!(),
    })
    .await;
    let (job, reports) = job(1, 1);
    run(Arc::clone(&job), Arc::clone(&provider.llm), 1, reports).await;
    assert_eq!(attempts.load(Ordering::Relaxed), 2);
    let state = job.default_generation.as_ref().unwrap();
    let outcomes = state.assessments().unwrap();
    assert_eq!(
        outcomes
            .values()
            .next()
            .unwrap()
            .result
            .as_ref()
            .unwrap()
            .centrality,
        5
    );
    assert_eq!(
        job.snapshot
            .lock()
            .unwrap()
            .default_selection
            .as_ref()
            .unwrap()
            .generated_count,
        1
    );
}

#[tokio::test]
async fn stage_b_rejects_unselected_or_repeated_ids_before_exposing_a_preview() {
    for repeated in [false, true] {
        let attempts = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&attempts);
        let provider = MockProvider::new(move |request| match stage(request) {
            "extract" => Reply::json(extracted(0, 3)),
            "assess" => Reply::json(assessed(4, true, false)),
            "generate" => {
                let mut output = generated(&keyed(request));
                if observed.fetch_add(1, Ordering::Relaxed) == 0 {
                    if repeated {
                        let mut duplicate = output["items"][0].clone();
                        duplicate["target"] = json!("Another target");
                        output["items"].as_array_mut().unwrap().push(duplicate);
                    } else {
                        output["items"][0]["knowledge_point_id"] = json!("kp_2");
                    }
                }
                Reply::json(output)
            }
            _ => unreachable!(),
        })
        .await;
        let (job, reports) = job(1, 1);
        run(Arc::clone(&job), Arc::clone(&provider.llm), 2, reports).await;
        assert_eq!(attempts.load(Ordering::Relaxed), 2);
        let snapshot = job.snapshot.lock().unwrap();
        assert_eq!(
            snapshot.default_selection.as_ref().unwrap().generated_count,
            1
        );
        assert_eq!(snapshot.ready_previews[0].llm_result.items.len(), 1);
        assert_eq!(
            snapshot.ready_previews[0].llm_result.items[0]
                .source
                .knowledge_point,
            point(0, 0)
        );
    }
}

#[tokio::test]
async fn extraction_failure_preserves_other_sections_and_reports_its_actual_shortfall() {
    let provider = MockProvider::new(|request| match stage(request) {
        "extract" if chunk_index(request) == 0 => Reply {
            payload: Value::Null,
            status: 400,
            delay: Duration::ZERO,
        },
        "extract" => Reply::json(extracted(1, 1)),
        "assess" => Reply::json(assessed(4, true, false)),
        "generate" => Reply::json(generated(&keyed(request))),
        _ => unreachable!(),
    })
    .await;
    let (job, reports) = job(2, 2);
    run(Arc::clone(&job), Arc::clone(&provider.llm), 2, reports).await;
    let snapshot = job.snapshot.lock().unwrap();
    let report = snapshot.default_selection.as_ref().unwrap();
    assert_eq!(report.extracted_chunks, 1);
    assert_eq!(report.extraction_failed_chunks, 1);
    assert_eq!(report.assessed_count, 1);
    assert_eq!(report.generated_count, 1);
    assert_eq!(snapshot.completed_chunks, 2);
    assert!(report
        .shortfall
        .iter()
        .any(|detail| detail.reason == "extraction_failures"));
    assert_eq!(
        snapshot.ready_previews[0].llm_result.items[0]
            .source
            .start_line,
        11
    );
}

#[tokio::test]
async fn pause_and_cancellation_control_each_asynchronous_default_phase() {
    for phase in ["extract", "assess", "generate"] {
        for cancel in [false, true] {
            let entered = Arc::new(Notify::new());
            let notified = Arc::clone(&entered);
            let provider = MockProvider::new(move |request| {
                let mut reply = match stage(request) {
                    "extract" => Reply::json(extracted(0, 3)),
                    "assess" => Reply::json(assessed(4, true, false)),
                    "generate" => Reply::json(generated(&keyed(request))),
                    _ => unreachable!(),
                };
                if stage(request) == phase {
                    notified.notify_one();
                    reply.delay = Duration::from_millis(100);
                }
                reply
            })
            .await;
            let (job, reports) = job(1, 2);
            let worker = Arc::clone(&job);
            let llm = Arc::clone(&provider.llm);
            let task = tokio::spawn(async move { run(worker, llm, 2, reports).await });
            tokio::time::timeout(Duration::from_secs(2), entered.notified())
                .await
                .unwrap();
            if cancel {
                job.cancelled.store(true, Ordering::Relaxed);
                job.control_changed.notify_waiters();
                tokio::time::timeout(Duration::from_millis(250), task)
                    .await
                    .unwrap()
                    .unwrap();
                let snapshot = job.snapshot.lock().unwrap();
                assert!(snapshot.is_cancelled);
                assert!(snapshot.is_finished);
                assert!(snapshot.summary.is_none());
            } else {
                job.paused.store(true, Ordering::Relaxed);
                job.control_changed.notify_waiters();
                let before = job.snapshot.lock().unwrap().progress_percent;
                sleep(Duration::from_millis(140)).await;
                assert_eq!(job.snapshot.lock().unwrap().progress_percent, before);
                assert!(!task.is_finished());
                job.paused.store(false, Ordering::Relaxed);
                job.control_changed.notify_waiters();
                tokio::time::timeout(Duration::from_secs(2), task)
                    .await
                    .unwrap()
                    .unwrap();
                let snapshot = job.snapshot.lock().unwrap();
                assert_eq!(snapshot.progress_percent, 100);
                assert_eq!(
                    snapshot.default_selection.as_ref().unwrap().generated_count,
                    2
                );
            }
        }
    }
}
