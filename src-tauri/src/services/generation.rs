use serde::Serialize;

use crate::models::note::Note;

use super::chunker::{self, MarkdownChunk};
use super::filesystem;

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
/// For now this is chunk-only output (no model calls, no DB writes). The same
/// shape can be extended in the next phase with fields like generated_count,
/// parse_failures, and persisted_count.
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
pub fn orchestrate_notes(notes: &[Note]) -> GenerationSummary {
	let mut note_reports = Vec::new();
	let mut chunk_previews = Vec::new();
	let mut notes_with_chunks = 0;

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
		chunk_previews.extend(chunks.iter().map(chunk_to_preview));
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
pub fn orchestrate_vault(vault_path: &str) -> Result<GenerationSummary, String> {
	let notes = filesystem::load_vault_notes(vault_path)?;
	Ok(orchestrate_notes(&notes))
}

/// Converts a full chunk into a compact preview record.
fn chunk_to_preview(chunk: &MarkdownChunk) -> ChunkPreview {
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

		let summary = orchestrate_notes(&notes);

		assert_eq!(summary.total_notes, 2);
		assert_eq!(summary.notes_with_chunks, 2);
		assert!(summary.total_chunks >= 2);
		assert_eq!(summary.note_reports.len(), 2);
	}
}
