//! Graph-based generation pipeline (Phase 0+).
//!
//! Ephemeral type system and schema definitions. No business logic in this module;
//! orchestration and consolidation logic move into separate phase-specific modules.

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

// Re-exports available for consumers of this module
// (Types are primarily used internally during consolidation and generation phases)
