//! Pass 3 consolidation: Vec<ExtractedKnowledge> → PropositionGraph.
//!
//! Merges per-chunk Stage A output into a single deduplicated graph. Entity
//! deduplication is keyed on a normalized comparison string (Layer 1 below);
//! every raw surface form is preserved in `EntityNode.aliases` for future
//! Layer 3 semantic entity resolution.

use std::collections::{HashMap, HashSet};

use super::types::{
    EntityNode, ExtractedKnowledge, KnowledgePoint, PropositionGraph, Relation, RelationType,
};

// ----------------------------------------------------------------------------
// FUTURE WORK: Entity Resolution
//
// normalize_for_comparison() intentionally performs only representation
// normalization (Unicode, whitespace, case).
//
// It DOES NOT merge:
//
// - Calvin cycle ↔ light-independent reactions
// - ATP ↔ adenosine triphosphate
// - Photosystem II ↔ PSII
//
// Those require semantic entity resolution and belong in a later pass.
// Candidate approaches: embedding cosine similarity, LLM-based cluster merging.
// The `EntityNode.aliases` field exists specifically to feed that future pass:
// it records every surface form seen, so the resolver has full evidence from
// source material without needing to re-parse the original chunks.
// ----------------------------------------------------------------------------

// =====================================================================
// Public API
// =====================================================================

/// Consolidates multi-chunk Stage A output into a single PropositionGraph.
///
/// Three steps:
/// 1. Build a canonical entity map (Layer 1 normalization + Layer 2 dedup).
/// 2. Populate `entity_ids` on every KnowledgePoint.
/// 3. Resolve `RelationRef` values into typed `Relation` edges, then dedup.
///
/// Takes ownership of `chunks`; returns a fully resolved PropositionGraph.
pub fn consolidate(chunks: Vec<ExtractedKnowledge>) -> PropositionGraph {
    // ── Step 1: Build canonical entity map ───────────────────────────────
    //
    // entity_map:  normalized_key → EntityNode (mutable during build)
    // id_map:      normalized_key → entity_id  (for fast lookup in steps 2/3)
    //
    // Insertion order: first mention across all chunks, in chunk order.
    // canonical_name = first-seen raw form; aliases accumulate all raw forms.

    let mut entity_map: HashMap<String, EntityNode> = HashMap::new();
    // Different normalized keys can still collapse to the same slug. For
    // example, `partition` and `!partition` both prefer `entity-partition`.
    // Track allocated IDs so those keys remain distinct without breaking graph
    // references or relying on HashMap iteration order.
    let mut used_entity_ids: HashSet<String> = HashSet::new();
    let mut id_map: HashMap<String, String> = HashMap::new();
    let mut seen_chunk_ids: HashMap<String, HashSet<String>> = HashMap::new(); // key → chunk_ids

    for chunk in &chunks {
        for mention in &chunk.raw_entities {
            let key = normalize_for_comparison(&mention.name);

            if let Some(node) = entity_map.get_mut(&key) {
                // Known entity: accumulate alias if new, record chunk_id if new
                if !node.aliases.contains(&mention.name) {
                    node.aliases.push(mention.name.clone());
                }
                seen_chunk_ids
                    .entry(key.clone())
                    .or_default()
                    .insert(mention.chunk_id.clone());
            } else {
                // First encounter: canonical_name = raw form as written
                let id = allocate_entity_id(&key, &mut used_entity_ids);
                entity_map.insert(
                    key.clone(),
                    EntityNode {
                        id: id.clone(),
                        canonical_name: mention.name.clone(),
                        aliases: vec![mention.name.clone()],
                        chunk_ids: Vec::new(), // filled after the loop
                    },
                );
                id_map.insert(key.clone(), id);
                seen_chunk_ids
                    .entry(key.clone())
                    .or_default()
                    .insert(mention.chunk_id.clone());
            }
        }
    }

    // Flush chunk_ids from the tracking set into each EntityNode
    for (key, chunk_id_set) in &seen_chunk_ids {
        if let Some(node) = entity_map.get_mut(key) {
            let mut ids: Vec<String> = chunk_id_set.iter().cloned().collect();
            ids.sort(); // deterministic order
            node.chunk_ids = ids;
        }
    }

    // ── Step 2: Populate entity_ids on every KnowledgePoint ──────────────
    //
    // raw_entity_names → normalize → id_map lookup → push to entity_ids.
    // Unresolvable names (shouldn't happen post-auto-union hydration, but
    // handled defensively) are silently skipped: a partial entity_ids list
    // is better than a panic.

    let mut all_knowledge_points: Vec<KnowledgePoint> = Vec::new();

    for mut chunk in chunks {
        for kp in &mut chunk.knowledge_points {
            for raw_name in &kp.raw_entity_names {
                let key = normalize_for_comparison(raw_name);
                if let Some(entity_id) = id_map.get(&key) {
                    kp.entity_ids.push(entity_id.clone());
                }
                // else: silently skip — entity wasn't in any raw_entities list
            }
        }
        all_knowledge_points.extend(chunk.knowledge_points);
    }

    // ── Step 3: Resolve RelationRefs → Relations, then dedup ─────────────
    //
    // source_id = first entity_id of the owning KnowledgePoint
    //   (simplification: the first entity in raw_entity_names is the subject).
    // target_id = id_map lookup on normalized target_entity_name.
    // Relations where either endpoint can't be resolved are dropped.
    // Duplicate (source, target, type) triples are collapsed.

    let mut relation_set: HashSet<(String, String, RelationType)> = HashSet::new();

    for kp in &all_knowledge_points {
        let source_id = match kp.entity_ids.first() {
            Some(id) => id.clone(),
            None => continue, // no entity_ids → can't emit a source
        };

        for rel in &kp.raw_relations {
            let target_key = normalize_for_comparison(&rel.target_entity_name);
            let target_id = match id_map.get(&target_key) {
                Some(id) => id.clone(),
                None => continue, // target not in entity pool → skip
            };

            relation_set.insert((source_id.clone(), target_id, rel.relation_type));
        }
    }

    let mut relations: Vec<Relation> = relation_set
        .into_iter()
        .map(|(source_id, target_id, relation_type)| Relation {
            source_id,
            target_id,
            relation_type,
        })
        .collect();
    // Sort for deterministic output order (source, target, type)
    relations.sort_by(|a, b| {
        a.source_id
            .cmp(&b.source_id)
            .then(a.target_id.cmp(&b.target_id))
    });

    // ── Assemble ──────────────────────────────────────────────────────────

    let mut entities: Vec<EntityNode> = entity_map.into_values().collect();
    entities.sort_by(|a, b| a.id.cmp(&b.id)); // deterministic order

    let graph = PropositionGraph {
        entities,
        knowledge_points: all_knowledge_points,
        relations,
    };

    debug_assert!(
        validate_graph(&graph).is_empty(),
        "consolidate() produced an invalid graph:\n{}",
        validate_graph(&graph).join("\n")
    );

    graph
}

// =====================================================================
// Layer 1 — Representation normalization
// =====================================================================

// ----------------------------------------------------------------------------
// FUTURE WORK: Entity Resolution
//
// normalize_for_comparison() intentionally performs only representation
// normalization (Unicode, whitespace, case).
//
// It DOES NOT merge:
//
// - Calvin cycle ↔ light-independent reactions
// - ATP ↔ adenosine triphosphate
// - Photosystem II ↔ PSII
//
// Those require semantic entity resolution and belong in a later pass.
// ----------------------------------------------------------------------------

/// Produces a comparison key for entity deduplication.
///
/// Applies representation-only transformations — no semantic normalization.
/// Two entity names that differ only in casing, whitespace, or Unicode
/// notation will produce the same key. Everything else produces a distinct key.
///
/// The raw form is NEVER discarded: callers store the original string in
/// `EntityNode.canonical_name` (first-seen) and `EntityNode.aliases` (all
/// forms). This key is ephemeral — used only for HashMap lookup.
fn normalize_for_comparison(name: &str) -> String {
    // Step 1: Map Unicode subscript and superscript characters to ASCII.
    // Subscript digits (₀–₉) appear in chemical formulas (CO₂, H₂O).
    // Superscript signs (⁺ ⁻) appear in ion notation (H⁺, e⁻).
    let mapped: String = name
        .chars()
        .map(|c| match c {
            // Subscript digits
            '\u{2080}' => '0',
            '\u{2081}' => '1',
            '\u{2082}' => '2',
            '\u{2083}' => '3',
            '\u{2084}' => '4',
            '\u{2085}' => '5',
            '\u{2086}' => '6',
            '\u{2087}' => '7',
            '\u{2088}' => '8',
            '\u{2089}' => '9',
            // Superscript signs
            '\u{207A}' => '+',
            '\u{207B}' => '-',
            _ => c,
        })
        .collect();

    // Step 2: Trim leading/trailing whitespace.
    let trimmed = mapped.trim();

    // Step 3: Collapse internal runs of whitespace to a single space.
    let collapsed: String = trimmed.split_whitespace().collect::<Vec<&str>>().join(" ");

    // Step 4: Fold to lowercase.
    collapsed.to_lowercase()
}

// =====================================================================
// Layer 2 — Deterministic ID generation
// =====================================================================

/// Generates a stable, deterministic entity ID from a normalized comparison key.
///
/// Format: `"entity-{slug}"` where `+` and `-` are preserved (meaningful in
/// chemical notation), other non-alphanumeric characters become `_`, consecutive
/// `_` are collapsed, and leading/trailing `_` are trimmed.
///
/// Using `_` as the word-separator (not `-`) means chemistry signs at the
/// boundary of a name are never stripped by the trim step.
///
/// Examples:
///   "chloroplast"    → "entity-chloroplast"
///   "photosystem ii" → "entity-photosystem_ii"
///   "co2"            → "entity-co2"
///   "h+"             → "entity-h+"
///   "e-"             → "entity-e-"
fn to_entity_id(normalized_key: &str) -> String {
    let slug: String = normalized_key
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '+' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();

    // Collapse consecutive underscores and trim leading/trailing underscores
    let mut prev_sep = false;
    let collapsed: String = slug
        .chars()
        .filter_map(|c| {
            if c == '_' {
                if prev_sep {
                    None // skip consecutive separator
                } else {
                    prev_sep = true;
                    Some(c)
                }
            } else {
                prev_sep = false;
                Some(c)
            }
        })
        .collect();

    let trimmed = collapsed.trim_matches('_');
    let slug = if trimmed.is_empty() {
        "unnamed"
    } else {
        trimmed
    };
    format!("entity-{slug}")
}

/// Allocates a graph-unique entity ID while leaving ordinary IDs unchanged.
///
/// A comparison key may contain meaningful punctuation that the ID slug does
/// not preserve. If its preferred ID is occupied, append a stable hash of the
/// complete key. The counter is only a final guard against the extremely
/// unlikely case where two distinct keys also share the same hash.
fn allocate_entity_id(normalized_key: &str, used_ids: &mut HashSet<String>) -> String {
    let base_id = to_entity_id(normalized_key);
    if used_ids.insert(base_id.clone()) {
        return base_id;
    }

    let key_hash = stable_entity_key_hash(normalized_key);
    let hashed_id = format!("{base_id}_{key_hash:016x}");
    if used_ids.insert(hashed_id.clone()) {
        return hashed_id;
    }

    let mut discriminator = 2usize;
    loop {
        let candidate = format!("{hashed_id}_{discriminator}");
        if used_ids.insert(candidate.clone()) {
            return candidate;
        }
        discriminator += 1;
    }
}

/// Returns a stable FNV-1a hash for deterministic entity-ID disambiguation.
///
/// `DefaultHasher` is deliberately avoided because its output is not a stable
/// persistence contract across Rust versions.
fn stable_entity_key_hash(value: &str) -> u64 {
    const FNV_OFFSET_BASIS: u64 = 0xcbf_29ce4_8422_2325;
    const FNV_PRIME: u64 = 0x000_0100_0000_01b3;

    value
        .as_bytes()
        .iter()
        .fold(FNV_OFFSET_BASIS, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
        })
}

// =====================================================================
// Integrity validation
// =====================================================================

/// Checks structural invariants of a completed PropositionGraph.
///
/// Returns a list of violation strings. An empty Vec means the graph is valid.
/// Called via `debug_assert!` at the end of `consolidate()` — runs in test and
/// debug builds, compiled away in release. Add checks here freely; they cost
/// nothing in production and will catch regressions immediately.
///
/// Checks performed:
/// 1. Entity IDs are unique (no two EntityNodes share an ID).
/// 2. Every relation source_id and target_id exists in the entity set.
/// 3. Every entity_id in every KnowledgePoint exists in the entity set.
pub fn validate_graph(graph: &PropositionGraph) -> Vec<String> {
    let mut violations: Vec<String> = Vec::new();

    // Build the set of known entity IDs once — used by all checks below.
    let mut known_ids: HashSet<&str> = HashSet::new();
    for entity in &graph.entities {
        if !known_ids.insert(entity.id.as_str()) {
            violations.push(format!(
                "duplicate entity id: '{}' (canonical_name: '{}')",
                entity.id, entity.canonical_name
            ));
        }
    }

    // Check 2: relation endpoints must reference known entity IDs.
    for (i, rel) in graph.relations.iter().enumerate() {
        if !known_ids.contains(rel.source_id.as_str()) {
            violations.push(format!(
                "relation[{i}] source_id '{}' not found in entities",
                rel.source_id
            ));
        }
        if !known_ids.contains(rel.target_id.as_str()) {
            violations.push(format!(
                "relation[{i}] target_id '{}' not found in entities",
                rel.target_id
            ));
        }
    }

    // Check 3: entity_ids on KnowledgePoints must reference known entity IDs.
    for kp in &graph.knowledge_points {
        for id in &kp.entity_ids {
            if !known_ids.contains(id.as_str()) {
                violations.push(format!(
                    "knowledge_point '{}' references unknown entity_id '{}'",
                    kp.id, id
                ));
            }
        }
    }

    violations
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::graph_generation::types::{KnowledgeType, RawEntityMention, RelationRef};

    // ── normalize_for_comparison ─────────────────────────────────────────

    #[test]
    fn test_normalize_subscript_digits() {
        assert_eq!(normalize_for_comparison("CO₂"), "co2");
        assert_eq!(normalize_for_comparison("H₂O"), "h2o");
        assert_eq!(normalize_for_comparison("CO2"), "co2"); // already ASCII
    }

    #[test]
    fn test_normalize_superscript_signs() {
        assert_eq!(normalize_for_comparison("H⁺"), "h+");
        assert_eq!(normalize_for_comparison("e⁻"), "e-");
        assert_eq!(normalize_for_comparison("H+"), "h+"); // already ASCII
    }

    #[test]
    fn test_normalize_case_fold() {
        assert_eq!(normalize_for_comparison("Chloroplast"), "chloroplast");
        assert_eq!(normalize_for_comparison("PHOTOSYNTHESIS"), "photosynthesis");
    }

    #[test]
    fn test_normalize_whitespace() {
        assert_eq!(normalize_for_comparison("  Calvin cycle  "), "calvin cycle");
        assert_eq!(
            normalize_for_comparison("light  dependent  reactions"),
            "light dependent reactions"
        );
    }

    #[test]
    fn test_normalize_combined() {
        assert_eq!(
            normalize_for_comparison("CO₂ concentration"),
            "co2 concentration"
        );
        assert_eq!(normalize_for_comparison("H⁺ ions"), "h+ ions");
    }

    // ── to_entity_id ─────────────────────────────────────────────────────

    #[test]
    fn test_entity_id_simple() {
        assert_eq!(to_entity_id("chloroplast"), "entity-chloroplast");
    }

    #[test]
    fn test_entity_id_spaces_become_underscores() {
        assert_eq!(to_entity_id("photosystem ii"), "entity-photosystem_ii");
        assert_eq!(to_entity_id("calvin cycle"), "entity-calvin_cycle");
    }

    #[test]
    fn test_entity_id_preserves_plus_minus() {
        assert_eq!(to_entity_id("h+"), "entity-h+");
        assert_eq!(to_entity_id("e-"), "entity-e-");
    }

    #[test]
    fn test_entity_id_collapses_consecutive_underscores() {
        // internal spaces become underscores → collapse consecutive
        assert_eq!(
            to_entity_id("co2 concentration"),
            "entity-co2_concentration"
        );
    }

    #[test]
    fn test_entity_id_uses_fallback_for_punctuation_only_names() {
        assert_eq!(to_entity_id("!!!"), "entity-unnamed");
        assert_eq!(to_entity_id(""), "entity-unnamed");
    }

    // ── consolidate ───────────────────────────────────────────────────────

    fn make_chunk(
        chunk_id: &str,
        entities: &[&str],
        points: Vec<KnowledgePoint>,
    ) -> ExtractedKnowledge {
        ExtractedKnowledge {
            chunk_id: chunk_id.to_string(),
            raw_entities: entities
                .iter()
                .map(|name| RawEntityMention {
                    name: name.to_string(),
                    chunk_id: chunk_id.to_string(),
                })
                .collect(),
            knowledge_points: points,
        }
    }

    fn make_point(id: &str, chunk_id: &str, entity_names: &[&str]) -> KnowledgePoint {
        KnowledgePoint {
            id: id.to_string(),
            point: format!("A fact about {}", entity_names.join(", ")),
            knowledge_type: KnowledgeType::Fact,
            chunk_id: chunk_id.to_string(),
            raw_entity_names: entity_names.iter().map(|s| s.to_string()).collect(),
            entity_ids: Vec::new(),
            raw_relations: Vec::new(),
        }
    }

    fn make_point_with_relation(
        id: &str,
        chunk_id: &str,
        entity_names: &[&str],
        target: &str,
        relation_type: RelationType,
    ) -> KnowledgePoint {
        KnowledgePoint {
            id: id.to_string(),
            point: format!("A fact about {}", entity_names.join(", ")),
            knowledge_type: KnowledgeType::Fact,
            chunk_id: chunk_id.to_string(),
            raw_entity_names: entity_names.iter().map(|s| s.to_string()).collect(),
            entity_ids: Vec::new(),
            raw_relations: vec![RelationRef {
                target_entity_name: target.to_string(),
                relation_type,
                source_quote: None,
            }],
        }
    }

    #[test]
    fn test_empty_input() {
        let graph = consolidate(vec![]);
        assert!(graph.entities.is_empty());
        assert!(graph.knowledge_points.is_empty());
        assert!(graph.relations.is_empty());
    }

    #[test]
    fn test_single_chunk_basic() {
        let chunk = make_chunk(
            "c1",
            &["chloroplast", "chlorophyll"],
            vec![make_point("c1-kp-0", "c1", &["chloroplast", "chlorophyll"])],
        );
        let graph = consolidate(vec![chunk]);
        assert_eq!(graph.entities.len(), 2);
        assert_eq!(graph.knowledge_points.len(), 1);
        // entity_ids populated
        assert_eq!(graph.knowledge_points[0].entity_ids.len(), 2);
    }

    #[test]
    fn test_entity_dedup_across_chunks() {
        let c1 = make_chunk("c1", &["chloroplast"], vec![]);
        let c2 = make_chunk("c2", &["chloroplast"], vec![]);
        let graph = consolidate(vec![c1, c2]);
        // Same name in two chunks → one EntityNode with both chunk_ids
        assert_eq!(graph.entities.len(), 1);
        assert_eq!(graph.entities[0].chunk_ids.len(), 2);
        assert!(graph.entities[0].chunk_ids.contains(&"c1".to_string()));
        assert!(graph.entities[0].chunk_ids.contains(&"c2".to_string()));
    }

    #[test]
    fn test_case_insensitive_dedup() {
        let c1 = make_chunk("c1", &["Chloroplast"], vec![]);
        let c2 = make_chunk("c2", &["chloroplast"], vec![]);
        let graph = consolidate(vec![c1, c2]);
        assert_eq!(graph.entities.len(), 1);
        // First-seen raw form is preserved as canonical_name
        assert_eq!(graph.entities[0].canonical_name, "Chloroplast");
        // Both forms in aliases
        assert!(graph.entities[0]
            .aliases
            .contains(&"Chloroplast".to_string()));
        assert!(graph.entities[0]
            .aliases
            .contains(&"chloroplast".to_string()));
    }

    #[test]
    fn test_subscript_normalization_dedup() {
        // CO₂ and CO2 should resolve to the same EntityNode
        let c1 = make_chunk("c1", &["CO₂"], vec![]);
        let c2 = make_chunk("c2", &["CO2"], vec![]);
        let graph = consolidate(vec![c1, c2]);
        assert_eq!(graph.entities.len(), 1);
        assert_eq!(graph.entities[0].id, "entity-co2"); // no spaces → no underscores
        assert_eq!(graph.entities[0].canonical_name, "CO₂"); // first-seen form
        assert!(graph.entities[0].aliases.contains(&"CO2".to_string()));
    }

    #[test]
    fn test_slug_collisions_receive_unique_deterministic_ids() {
        fn collision_chunk() -> ExtractedKnowledge {
            make_chunk(
                "c1",
                &["partition", "!partition"],
                vec![make_point_with_relation(
                    "c1-kp-0",
                    "c1",
                    &["partition"],
                    "!partition",
                    RelationType::RelatedTo,
                )],
            )
        }

        let graph = consolidate(vec![collision_chunk()]);
        let partition = graph
            .entities
            .iter()
            .find(|entity| entity.canonical_name == "partition")
            .expect("partition entity should exist");
        let punctuated_partition = graph
            .entities
            .iter()
            .find(|entity| entity.canonical_name == "!partition")
            .expect("!partition entity should exist");

        assert_eq!(partition.id, "entity-partition");
        assert!(punctuated_partition.id.starts_with("entity-partition_"));
        assert_ne!(partition.id, punctuated_partition.id);
        assert_eq!(graph.relations.len(), 1);
        assert_eq!(graph.relations[0].source_id, partition.id);
        assert_eq!(graph.relations[0].target_id, punctuated_partition.id);
        assert!(validate_graph(&graph).is_empty());

        // Reprocessing the same ordered graph must allocate the same IDs.
        let repeated = consolidate(vec![collision_chunk()]);
        let entity_ids = graph
            .entities
            .iter()
            .map(|entity| (&entity.canonical_name, &entity.id))
            .collect::<Vec<_>>();
        let repeated_ids = repeated
            .entities
            .iter()
            .map(|entity| (&entity.canonical_name, &entity.id))
            .collect::<Vec<_>>();
        assert_eq!(entity_ids, repeated_ids);
    }

    #[test]
    fn test_entity_ids_populated() {
        let chunk = make_chunk(
            "c1",
            &["ATP", "NADPH"],
            vec![make_point("c1-kp-0", "c1", &["ATP", "NADPH"])],
        );
        let graph = consolidate(vec![chunk]);
        let kp = &graph.knowledge_points[0];
        assert_eq!(kp.entity_ids.len(), 2);
        // Verify IDs point to real entities
        let entity_ids: HashSet<&str> = graph.entities.iter().map(|e| e.id.as_str()).collect();
        for id in &kp.entity_ids {
            assert!(
                entity_ids.contains(id.as_str()),
                "entity_id {id} not found in graph"
            );
        }
    }

    #[test]
    fn test_relation_resolved() {
        let chunk = make_chunk(
            "c1",
            &["ATP", "Calvin cycle"],
            vec![make_point_with_relation(
                "c1-kp-0",
                "c1",
                &["ATP"],
                "Calvin cycle",
                RelationType::Prerequisite,
            )],
        );
        let graph = consolidate(vec![chunk]);
        assert_eq!(graph.relations.len(), 1);
        let rel = &graph.relations[0];
        assert_eq!(rel.relation_type, RelationType::Prerequisite);
        // source is "atp" entity, target is "calvin cycle" entity
        assert_eq!(rel.source_id, "entity-atp");
        assert_eq!(rel.target_id, "entity-calvin_cycle");
    }

    #[test]
    fn test_relation_dedup() {
        // Two chunks each contributing the same source→target→type
        let c1 = make_chunk(
            "c1",
            &["ATP", "Calvin cycle"],
            vec![make_point_with_relation(
                "c1-kp-0",
                "c1",
                &["ATP"],
                "Calvin cycle",
                RelationType::Prerequisite,
            )],
        );
        let c2 = make_chunk(
            "c2",
            &["ATP", "Calvin cycle"],
            vec![make_point_with_relation(
                "c2-kp-0",
                "c2",
                &["ATP"],
                "Calvin cycle",
                RelationType::Prerequisite,
            )],
        );
        let graph = consolidate(vec![c1, c2]);
        assert_eq!(
            graph.relations.len(),
            1,
            "duplicate relation should be collapsed"
        );
    }

    #[test]
    fn test_unresolvable_relation_skipped() {
        // Relation target "membrane" never declared in entities
        let chunk = make_chunk(
            "c1",
            &["chloroplast"],
            vec![make_point_with_relation(
                "c1-kp-0",
                "c1",
                &["chloroplast"],
                "membrane",
                RelationType::RelatedTo,
            )],
        );
        let graph = consolidate(vec![chunk]);
        // No panic; relation silently dropped
        assert_eq!(graph.relations.len(), 0);
        assert_eq!(graph.entities.len(), 1);
    }

    #[test]
    fn test_canonical_name_is_first_seen() {
        // Second chunk introduces a different casing — canonical should be first chunk's form
        let c1 = make_chunk("c1", &["Photosystem II"], vec![]);
        let c2 = make_chunk("c2", &["photosystem ii"], vec![]);
        let graph = consolidate(vec![c1, c2]);
        assert_eq!(graph.entities.len(), 1);
        assert_eq!(graph.entities[0].canonical_name, "Photosystem II");
    }

    // ── validate_graph ────────────────────────────────────────────────────

    #[test]
    fn test_validate_graph_clean_output_is_valid() {
        // Any graph produced by consolidate() must pass validate_graph.
        let chunk = make_chunk(
            "c1",
            &["ATP", "NADPH"],
            vec![make_point("c1-kp-0", "c1", &["ATP", "NADPH"])],
        );
        let graph = consolidate(vec![chunk]);
        assert!(validate_graph(&graph).is_empty());
    }

    #[test]
    fn test_validate_graph_duplicate_entity_id() {
        let entity = EntityNode {
            id: "entity-atp".to_string(),
            canonical_name: "ATP".to_string(),
            aliases: vec!["ATP".to_string()],
            chunk_ids: vec!["c1".to_string()],
        };
        let graph = PropositionGraph {
            entities: vec![entity.clone(), entity], // same ID twice
            knowledge_points: vec![],
            relations: vec![],
        };
        let v = validate_graph(&graph);
        assert!(!v.is_empty());
        assert!(v[0].contains("duplicate entity id"));
    }

    #[test]
    fn test_validate_graph_dangling_relation_source() {
        let graph = PropositionGraph {
            entities: vec![EntityNode {
                id: "entity-atp".to_string(),
                canonical_name: "ATP".to_string(),
                aliases: vec!["ATP".to_string()],
                chunk_ids: vec![],
            }],
            knowledge_points: vec![],
            relations: vec![Relation {
                source_id: "entity-ghost".to_string(), // does not exist
                target_id: "entity-atp".to_string(),
                relation_type: RelationType::RelatedTo,
            }],
        };
        let v = validate_graph(&graph);
        assert!(v
            .iter()
            .any(|s| s.contains("source_id") && s.contains("entity-ghost")));
    }

    #[test]
    fn test_validate_graph_dangling_relation_target() {
        let graph = PropositionGraph {
            entities: vec![EntityNode {
                id: "entity-atp".to_string(),
                canonical_name: "ATP".to_string(),
                aliases: vec!["ATP".to_string()],
                chunk_ids: vec![],
            }],
            knowledge_points: vec![],
            relations: vec![Relation {
                source_id: "entity-atp".to_string(),
                target_id: "entity-ghost".to_string(), // does not exist
                relation_type: RelationType::RelatedTo,
            }],
        };
        let v = validate_graph(&graph);
        assert!(v
            .iter()
            .any(|s| s.contains("target_id") && s.contains("entity-ghost")));
    }

    #[test]
    fn test_validate_graph_dangling_kp_entity_id() {
        use crate::services::graph_generation::types::KnowledgeType;
        let graph = PropositionGraph {
            entities: vec![],
            knowledge_points: vec![KnowledgePoint {
                id: "kp-0".to_string(),
                point: "some fact".to_string(),
                knowledge_type: KnowledgeType::Fact,
                chunk_id: "c1".to_string(),
                raw_entity_names: vec![],
                entity_ids: vec!["entity-ghost".to_string()], // not in entities
                raw_relations: vec![],
            }],
            relations: vec![],
        };
        let v = validate_graph(&graph);
        assert!(v.iter().any(|s| s.contains("entity-ghost")));
    }
}
