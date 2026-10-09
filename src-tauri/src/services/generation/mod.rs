//! Generation entry points shared by Tauri commands and evaluation tools.
//!
//! Default and graph orchestration live in their respective child modules.

pub(crate) mod default;
pub(crate) mod graph;
mod job;
mod progress;
mod save;

// Desktop and evaluation targets consume different parts of this public facade.
#[allow(unused_imports)]
pub use default::{
    orchestrate_notes, orchestrate_notes_for_evaluation, orchestrate_vault,
    start_preview_generation_job,
};
#[allow(unused_imports)]
pub use graph::start_graph_generation_job;
#[allow(unused_imports)]
pub use job::{
    cancel_preview_generation, generation_metadata_for_job, set_preview_generation_paused,
};
#[allow(unused_imports)]
pub use progress::{
    get_preview_generation_progress, ChunkLlmQuestionPreview, ChunkLlmResult, ChunkPreview,
    GenerationProgressSnapshot, GenerationSummary, NoteGenerationReport,
};
#[allow(unused_imports)]
pub use save::save_generated_learning_items;

#[cfg(test)]
mod tests;
