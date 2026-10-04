//! Graph extraction, consolidation, entity resolution, and question generation.
//!
//! Desktop jobs use shared generation state; evaluation tools call the pipeline
//! stages directly. Graph data stays ephemeral throughout generation.

mod desktop;

pub mod bundle_builder;
pub mod consolidator;
pub mod entity_resolution;
pub mod graph_index;
pub mod pipeline;
pub mod stage_a_prompt;
pub mod stage_a_schema;
pub mod stage_b_generation;
pub mod stage_b_prompt;
pub mod stage_b_schema;
pub mod types;

pub use desktop::start_graph_generation_job;
