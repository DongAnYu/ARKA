use serde::Serialize;

use crate::models::note::Note;

use super::chunker::{self, MarkdownChunk};
use super::filesystem;
use super::llm::{LlmService, StageBMcq};

/// Lightweight chunk metadata returned to callers for observability.
///
/// This keeps UI payloads compact while still exposing enough context to
/// inspect what the chunker produced before an LLM is integrated.
#[derive(Debug, Clone, Serialize)]
pub struct ChunkPreview {
	pub note_path: String,
	pub note_title: String,
	pub heading: String,
	pub section_index: usize,
	pub chunk_index: usize,
	pub start_line: usize,
	pub end_line: usize,
	pub char_count: usize,
	pub preview_text: String,
	pub llm_result: ChunkLlmResult,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChunkLlmResult {
	pub status: String,
	pub key_points: Vec<String>,
	pub questions: Vec<ChunkLlmQuestionPreview>,
	pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChunkLlmQuestionPreview {
	pub question: String,
	pub option_a: String,
	pub option_b: String,
	pub option_c: String,
	pub option_d: String,
	pub correct_answer: String,
	pub explanation: String,
}

/// Per-note generation metrics.
#[derive(Debug, Clone, Serialize)]
pub struct NoteGenerationReport {
	pub note_path: String,
	pub note_title: String,
	pub total_chunks: usize,
}

/// Aggregated output for one orchestration run.
///
/// Includes per-chunk Stage A/B model output for preview-only inspection.
/// No DB writes are performed in this phase.
#[derive(Debug, Clone, Serialize)]
pub struct GenerationSummary {
	pub total_notes: usize,
	pub total_chunks: usize,
	pub notes_with_chunks: usize,
	pub note_reports: Vec<NoteGenerationReport>,
	pub chunk_previews: Vec<ChunkPreview>,
}

/// Runs the chunking pipeline for all notes in memory.
///
/// Orchestration flow:
/// 1. Iterate notes.
/// 2. Chunk each note with the markdown chunker.
/// 3. Collect note-level and chunk-level metrics.
pub async fn orchestrate_notes(notes: &[Note]) -> GenerationSummary {
	let mut note_reports = Vec::new();
	let mut chunk_previews = Vec::new();
	let mut notes_with_chunks = 0;
	let llm_service = LlmService::from_env().ok();

	for note in notes {
		let chunks = chunker::chunk_note(note);
		if !chunks.is_empty() {
			notes_with_chunks += 1;
		}

		note_reports.push(NoteGenerationReport {
			note_path: note.path.clone(),
			note_title: note.title.clone(),
			total_chunks: chunks.len(),
		});

		// Preserve chunk order for deterministic downstream processing.
		for chunk in &chunks {
			chunk_previews.push(chunk_to_preview(chunk, llm_service.as_ref()).await);
		}
	}

	GenerationSummary {
		total_notes: notes.len(),
		total_chunks: chunk_previews.len(),
		notes_with_chunks,
		note_reports,
		chunk_previews,
	}
}

/// Convenience entry point that loads notes from a vault path and then
/// delegates to `orchestrate_notes`.
pub async fn orchestrate_vault(vault_path: &str) -> Result<GenerationSummary, String> {
	let notes = filesystem::load_vault_notes(vault_path)?;
	Ok(orchestrate_notes(&notes).await)
}

/// Converts a full chunk into a compact preview record.
async fn chunk_to_preview(chunk: &MarkdownChunk, llm_service: Option<&LlmService>) -> ChunkPreview {
	ChunkPreview {
		note_path: chunk.note_path.clone(),
		note_title: chunk.note_title.clone(),
		heading: chunk.heading.clone(),
		section_index: chunk.section_index,
		chunk_index: chunk.chunk_index,
		start_line: chunk.start_line,
		end_line: chunk.end_line,
		char_count: chunk.content.chars().count(),
		preview_text: build_preview_text(&chunk.content, 220),
		llm_result: run_chunk_llm_pipeline(chunk, llm_service).await,
	}
}

/// Runs Stage A -> Stage B for a chunk through the configured LLM service.
async fn run_chunk_llm_pipeline(chunk: &MarkdownChunk, llm_service: Option<&LlmService>) -> ChunkLlmResult {
	let Some(service) = llm_service else {
		return ChunkLlmResult {
			status: String::from("error_init"),
			key_points: Vec::new(),
			questions: Vec::new(),
			error: Some(String::from("LLM service could not be initialized from env")),
		};
	};

	let stage_a = match service.generate_stage_a_key_points(&chunk.content).await {
		Ok(value) => value,
		Err(err) => {
			return ChunkLlmResult {
				status: String::from("error_stage_a"),
				key_points: Vec::new(),
				questions: Vec::new(),
				error: Some(err.to_string()),
			};
		}
	};

	let key_points = stage_a
		.key_points
		.into_iter()
		.map(|item| item.knowledge_point)
		.collect::<Vec<_>>();

	if key_points.is_empty() {
		return ChunkLlmResult {
			status: String::from("no_content"),
			key_points,
			questions: Vec::new(),
			error: None,
		};
	}

	let stage_b = match service.generate_stage_b_mcqs(&chunk.content, &key_points).await {
		Ok(value) => value,
		Err(err) => {
			return ChunkLlmResult {
				status: String::from("error_stage_b"),
				key_points,
				questions: Vec::new(),
				error: Some(err.to_string()),
			};
		}
	};

	ChunkLlmResult {
		status: String::from("ok"),
		key_points,
		questions: stage_b
			.questions
			.into_iter()
			.map(mcq_to_preview)
			.collect(),
		error: None,
	}
}

fn mcq_to_preview(item: StageBMcq) -> ChunkLlmQuestionPreview {
	ChunkLlmQuestionPreview {
		question: item.question,
		option_a: item.option_a,
		option_b: item.option_b,
		option_c: item.option_c,
		option_d: item.option_d,
		correct_answer: item.correct_answer,
		explanation: item.explanation,
	}
}

/// Builds a compact, single-line preview snippet for UI inspection.
fn build_preview_text(content: &str, max_chars: usize) -> String {
	let normalized = content.split_whitespace().collect::<Vec<_>>().join(" ");
	let total_chars = normalized.chars().count();

	if total_chars <= max_chars {
		return normalized;
	}

	if max_chars <= 3 {
		return String::from("...");
	}

	let head_chars = max_chars.saturating_mul(2) / 3;
	let tail_chars = max_chars.saturating_sub(head_chars);

	let head: String = normalized.chars().take(head_chars).collect();
	let tail: String = normalized
		.chars()
		.skip(total_chars.saturating_sub(tail_chars))
		.collect();

	format!("{} ... {}", head.trim_end(), tail.trim_start())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn orchestrator_counts_chunks_across_multiple_notes() {
		let notes = vec![
			Note {
				id: None,
				path: String::from("/vault/rust.md"),
				title: String::from("rust"),
				content: String::from(
					"## Ownership\nRust ownership rules govern borrowing and moves. Rust ownership rules govern borrowing and moves. Rust ownership rules govern borrowing and moves. Rust ownership rules govern borrowing and moves.",
				),
				last_modified: String::from("0"),
			},
			Note {
				id: None,
				path: String::from("/vault/short.md"),
				title: String::from("short"),
				content: String::from("tiny"),
				last_modified: String::from("0"),
			},
		];

		let summary = tauri::async_runtime::block_on(orchestrate_notes(&notes));

		assert_eq!(summary.total_notes, 2);
		assert_eq!(summary.notes_with_chunks, 2);
		assert!(summary.total_chunks >= 2);
		assert_eq!(summary.note_reports.len(), 2);
	}
}
