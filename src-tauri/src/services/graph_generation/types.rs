//! Phase 0 type system for graph-based generation.
//!
//! Serde-only — no rusqlite traits, no persistence logic. Graph state is
//! ephemeral and dropped after each run; only final MCQ rows hit SQLite.
//! Consolidation (Pass 3) is a pure function: Vec<ExtractedKnowledge> -> PropositionGraph.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeType {
    Definition,
    Fact,
    Procedural,
    Conceptual,
}

/// Closed set — must stay exactly in sync with stage_a_schema.rs and
/// stage_a_prompt.rs. Extend here first if a new relation type is needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationType {
    RelatedTo,
    Contrasts,
    Prerequisite,
    Consequence,
    Example,
    CounterExample,
}

/// Computed structurally from graph edges at bundle-assembly time
/// (Recall if a point has no related_points, Relational otherwise) —
/// never asked of the LLM directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionType {
    Recall,
    Relational,
}

// ---------------------------------------------------------------------
// Pre-consolidation (Stage A output, per chunk)
// ---------------------------------------------------------------------

/// A raw, un-deduplicated entity mention as extracted from a single chunk.
/// Pass 3 merges these by name across chunks into stable EntityNodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawEntityMention {
    pub name: String,
    pub chunk_id: String,
}

/// A relation as stated by Stage A, before the target has a stable entity ID.
/// No source field: the source is implicitly the owning KnowledgePoint's
/// primary entity, resolved via `KnowledgePoint.raw_entity_names`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationRef {
    pub target_entity_name: String,
    pub relation_type: RelationType,
    pub source_quote: Option<String>,
}

/// A testable proposition extracted from a chunk. Shared pre- and
/// post-consolidation: Stage A populates everything except `entity_ids`;
/// Pass 3 fills `entity_ids` in place once stable IDs exist.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgePoint {
    pub id: String,
    pub point: String,
    pub knowledge_type: KnowledgeType,
    pub chunk_id: String,

    /// Raw names of every entity this point is about, as written by Stage A.
    /// Broader than relation targets — e.g. "Chloroplasts contain chlorophyll"
    /// is about both entities even though only one may appear as a
    /// `raw_relations` target.
    pub raw_entity_names: Vec<String>,

    /// Stable entity IDs. Empty at Stage A time; Pass 3 resolves
    /// `raw_entity_names` + every `raw_relations[i].target_entity_name`
    /// against the deduplicated entity pool to populate this.
    #[serde(default)]
    pub entity_ids: Vec<String>,
    #[serde(default)]
    pub raw_relations: Vec<RelationRef>,        
}

/// Per-chunk Stage A output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedKnowledge {
    pub chunk_id: String,
    pub raw_entities: Vec<RawEntityMention>,
    pub knowledge_points: Vec<KnowledgePoint>,
}

// ---------------------------------------------------------------------
// Post-consolidation (Pass 3 output)
// ---------------------------------------------------------------------

/// A deduplicated entity with a stable ID. Pure connective tissue —
/// deliberately no back-references to KnowledgePoints. Consumers filter
/// `PropositionGraph.knowledge_points` by `entity_ids` at query time.
///
/// `canonical_name` is the first-seen raw form; display quality is preserved.
/// `aliases` accumulates every raw surface form that normalized to the same
/// comparison key — evidence preserved for future Layer 3 entity resolution
/// (e.g. embedding-based or LLM-based merging of "Calvin cycle" ↔
/// "light-independent reactions").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityNode {
    pub id: String,
    /// First-seen raw form of this entity's name (used for display).
    pub canonical_name: String,
    /// All raw surface forms that resolved to this node, in insertion order.
    /// Includes the canonical name as its first element.
    pub aliases: Vec<String>,
    pub chunk_ids: Vec<String>,
}

/// A resolved relation between two stable entity IDs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relation {
    pub source_id: String,
    pub target_id: String,
    pub relation_type: RelationType,
}

/// The consolidated, deduplicated output of Pass 3.
/// Ephemeral — dropped after each run, never persisted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropositionGraph {
    pub entities: Vec<EntityNode>,
    /// Same KnowledgePoints Stage A produced, with `entity_ids` now
    /// resolved instead of empty.
    pub knowledge_points: Vec<KnowledgePoint>,
    pub relations: Vec<Relation>,
}

// ---------------------------------------------------------------------
// Bundle assembly (consumed by Stage B — Phase 4/5)
// ---------------------------------------------------------------------

/// Everything Stage B needs to generate one question.
///
/// Structured data: relations and supporting entities are kept separate from
/// KnowledgePoints to enable flexible prompt formatting. Only immediately before
/// calling the LLM should these be serialized into text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphContextBundle {
    pub root_point: KnowledgePoint,
    pub related_points: Vec<KnowledgePoint>,
    pub question_type: QuestionType,
    /// Entities connected to root_point (for context, not necessarily mentioned in related_points)
    pub supporting_entities: Vec<EntityNode>,
    /// Relations involving root_point's entities (for context)
    pub supporting_relations: Vec<Relation>,
}
