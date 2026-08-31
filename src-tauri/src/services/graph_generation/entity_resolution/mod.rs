//! Semantic entity-resolution support.
//!
//! This module operates on the deterministic `PropositionGraph` produced by
//! `consolidator` and prepares entity evidence for later embedding-based
//! candidate generation and LLM verification.

pub mod candidate_generator;
pub mod context_builder;
pub mod embedding_generator;
pub mod graph_rewriter;
pub mod merge_planner;
pub mod pipeline;
pub mod semantic_verifier;
