//! Semantic entity-resolution support.
//!
//! This module operates on the deterministic `PropositionGraph` produced by
//! `consolidator` and prepares entity evidence for later embedding-based
//! candidate generation and LLM verification.

pub mod context_builder;
pub mod embedding_generator;
