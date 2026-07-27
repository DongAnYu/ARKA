//! Graph-based generation pipeline (Phase 0+).
//!
//! Ephemeral type system and schema definitions. No business logic in this module;
//! orchestration and consolidation logic move into separate phase-specific modules.

pub mod types;
pub mod stage_a_schema;
pub mod stage_a_prompt;
pub mod consolidator;

// Re-exports available for consumers of this module
// (Types are primarily used internally during consolidation and generation phases)
