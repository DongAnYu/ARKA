//! Graph indexing for efficient point lookup and relation traversal.
//!
//! After consolidation, the PropositionGraph is read-only. The GraphIndex
//! precomputes two fast lookup tables:
//! - entity_id → list of knowledge_point IDs mentioning it
//! - entity_id → list of outgoing edges (relation_type, target_entity_id)
//!
//! This enables efficient bundle assembly: given a root point, quickly find
//! all related points without scanning the full relation list.

use std::collections::HashMap;

use super::types::{PropositionGraph, RelationType};

// =====================================================================
// Index structure
// =====================================================================

/// Fast lookup tables over a consolidated PropositionGraph.
///
/// Precomputed from the graph at bundle-assembly time. Immutable thereafter.
#[derive(Debug, Clone)]
pub struct GraphIndex {
    /// entity_id → sorted list of point IDs mentioning this entity
    entity_to_points: HashMap<String, Vec<String>>,

    /// entity_id → sorted list of (RelationType, target_entity_id) edges
    entity_edges: HashMap<String, Vec<(RelationType, String)>>,

    /// entity_id → EntityNode (O(1) lookup, avoids linear scans at scale)
    entity_lookup: HashMap<String, crate::services::graph_generation::types::EntityNode>,

    /// point_id → KnowledgePoint (O(1) lookup, avoids linear scans at scale)
    point_lookup: HashMap<String, crate::services::graph_generation::types::KnowledgePoint>,
}

// =====================================================================
// Public API
// =====================================================================

/// Builds an index from a consolidated graph.
///
/// Three passes:
/// 1. Map each entity → all points mentioning it (via point.entity_ids)
/// 2. Map each entity → all outgoing edges (from graph.relations)
/// 3. Build O(1) lookup maps for entities and points (performance at scale)
pub fn build_index(graph: &PropositionGraph) -> GraphIndex {
    let mut entity_to_points: HashMap<String, Vec<String>> = HashMap::new();
    let mut entity_edges: HashMap<String, Vec<(RelationType, String)>> = HashMap::new();

    // ── Pass 1: Entity → Points ───────────────────────────────────────
    for point in &graph.knowledge_points {
        for entity_id in &point.entity_ids {
            entity_to_points
                .entry(entity_id.clone())
                .or_insert_with(Vec::<String>::new)
                .push(point.id.clone());
        }
    }

    // Sort each point list for deterministic iteration order
    for points in entity_to_points.values_mut() {
        points.sort();
        points.dedup(); // shouldn't be needed but defensive
    }

    // ── Pass 2: Entity → Edges ────────────────────────────────────────
    for relation in &graph.relations {
        entity_edges
            .entry(relation.source_id.clone())
            .or_insert_with(Vec::<(RelationType, String)>::new)
            .push((relation.relation_type, relation.target_id.clone()));
    }

    // Sort each edge list for deterministic iteration order
    for edges in entity_edges.values_mut() {
        edges.sort_by(|a, b| {
            a.1.cmp(&b.1)
                .then_with(|| format!("{:?}", a.0).cmp(&format!("{:?}", b.0)))
        });
    }

    // ── Pass 3: Entity and Point lookups (O(1) access) ────────────────
    let entity_lookup: HashMap<String, crate::services::graph_generation::types::EntityNode> =
        graph
            .entities
            .iter()
            .map(|e| (e.id.clone(), e.clone()))
            .collect();

    let point_lookup: HashMap<String, crate::services::graph_generation::types::KnowledgePoint> =
        graph
            .knowledge_points
            .iter()
            .map(|p| (p.id.clone(), p.clone()))
            .collect();

    GraphIndex {
        entity_to_points,
        entity_edges,
        entity_lookup,
        point_lookup,
    }
}

impl GraphIndex {
    /// Returns all point IDs that mention the given entity.
    ///
    /// Empty vec if entity not found.
    pub fn points_for_entity(&self, entity_id: &str) -> Vec<String> {
        self.entity_to_points
            .get(entity_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Returns all outgoing edges from the given entity.
    ///
    /// Each edge is (RelationType, target_entity_id).
    /// Empty vec if entity not found.
    pub fn edges_from_entity(&self, entity_id: &str) -> Vec<(RelationType, String)> {
        self.entity_edges
            .get(entity_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Returns all entities referenced in this index.
    pub fn all_entity_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .entity_to_points
            .keys()
            .chain(self.entity_edges.keys())
            .cloned()
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }

    /// O(1) lookup for an entity by ID. Returns None if not found.
    pub fn entity(
        &self,
        entity_id: &str,
    ) -> Option<&crate::services::graph_generation::types::EntityNode> {
        self.entity_lookup.get(entity_id)
    }

    /// O(1) lookup for a knowledge point by ID. Returns None if not found.
    pub fn point(
        &self,
        point_id: &str,
    ) -> Option<&crate::services::graph_generation::types::KnowledgePoint> {
        self.point_lookup.get(point_id)
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::graph_generation::types::{EntityNode, KnowledgePoint, Relation};

    fn make_entity(id: &str, name: &str) -> EntityNode {
        EntityNode {
            id: id.to_string(),
            canonical_name: name.to_string(),
            aliases: vec![name.to_string()],
            chunk_ids: vec!["c1".to_string()],
        }
    }

    fn make_point(id: &str, entity_ids: &[&str]) -> KnowledgePoint {
        use crate::services::graph_generation::types::KnowledgeType;
        KnowledgePoint {
            id: id.to_string(),
            point: "test point".to_string(),
            knowledge_type: KnowledgeType::Fact,
            chunk_id: "c1".to_string(),
            raw_entity_names: vec![],
            entity_ids: entity_ids.iter().map(|s| s.to_string()).collect(),
            raw_relations: vec![],
        }
    }

    fn make_relation(source_id: &str, target_id: &str, relation_type: RelationType) -> Relation {
        Relation {
            source_id: source_id.to_string(),
            target_id: target_id.to_string(),
            relation_type,
        }
    }

    #[test]
    fn test_empty_graph() {
        let graph = PropositionGraph {
            entities: vec![],
            knowledge_points: vec![],
            relations: vec![],
        };
        let index = build_index(&graph);
        assert!(index.all_entity_ids().is_empty());
    }

    #[test]
    fn test_entity_to_points_single() {
        let graph = PropositionGraph {
            entities: vec![make_entity("e1", "ATP")],
            knowledge_points: vec![make_point("p1", &["e1"])],
            relations: vec![],
        };
        let index = build_index(&graph);
        assert_eq!(index.points_for_entity("e1"), vec!["p1"]);
        assert_eq!(
            index.points_for_entity("e_nonexistent"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn test_entity_to_points_multiple() {
        let graph = PropositionGraph {
            entities: vec![make_entity("e1", "ATP"), make_entity("e2", "Chloroplast")],
            knowledge_points: vec![
                make_point("p1", &["e1", "e2"]), // mentions both
                make_point("p2", &["e1"]),       // mentions e1 only
                make_point("p3", &["e2"]),       // mentions e2 only
            ],
            relations: vec![],
        };
        let index = build_index(&graph);
        assert_eq!(index.points_for_entity("e1"), vec!["p1", "p2"]); // sorted
        assert_eq!(index.points_for_entity("e2"), vec!["p1", "p3"]);
    }

    #[test]
    fn test_edges_from_entity_single() {
        let graph = PropositionGraph {
            entities: vec![make_entity("e1", "ATP"), make_entity("e2", "Energy")],
            knowledge_points: vec![],
            relations: vec![make_relation("e1", "e2", RelationType::RelatedTo)],
        };
        let index = build_index(&graph);
        let edges = index.edges_from_entity("e1");
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0], (RelationType::RelatedTo, "e2".to_string()));
    }

    #[test]
    fn test_edges_from_entity_multiple() {
        let graph = PropositionGraph {
            entities: vec![
                make_entity("e1", "ATP"),
                make_entity("e2", "Calvin Cycle"),
                make_entity("e3", "Energy"),
            ],
            knowledge_points: vec![],
            relations: vec![
                make_relation("e1", "e2", RelationType::Prerequisite),
                make_relation("e1", "e3", RelationType::RelatedTo),
            ],
        };
        let index = build_index(&graph);
        let edges = index.edges_from_entity("e1");
        assert_eq!(edges.len(), 2);
        // Should be sorted by target_id
        assert_eq!(edges[0].1, "e2");
        assert_eq!(edges[1].1, "e3");
    }

    #[test]
    fn test_edges_nonexistent_entity() {
        let graph = PropositionGraph {
            entities: vec![],
            knowledge_points: vec![],
            relations: vec![],
        };
        let index = build_index(&graph);
        assert_eq!(index.edges_from_entity("e_nonexistent"), vec![]);
    }

    #[test]
    fn test_all_entity_ids() {
        let graph = PropositionGraph {
            entities: vec![
                make_entity("e1", "ATP"),
                make_entity("e2", "Chloroplast"),
                make_entity("e3", "Light"),
            ],
            knowledge_points: vec![make_point("p1", &["e1", "e2"]), make_point("p2", &["e3"])],
            relations: vec![make_relation("e1", "e2", RelationType::RelatedTo)],
        };
        let index = build_index(&graph);
        let all_ids = index.all_entity_ids();
        assert_eq!(all_ids, vec!["e1", "e2", "e3"]);
    }

    #[test]
    fn test_dedup_points_for_same_entity() {
        // A point shouldn't list the same entity twice, but be defensive
        let graph = PropositionGraph {
            entities: vec![make_entity("e1", "ATP")],
            knowledge_points: vec![KnowledgePoint {
                id: "p1".to_string(),
                point: "test".to_string(),
                knowledge_type: crate::services::graph_generation::types::KnowledgeType::Fact,
                chunk_id: "c1".to_string(),
                raw_entity_names: vec![],
                entity_ids: vec!["e1".to_string(), "e1".to_string()], // duplicate
                raw_relations: vec![],
            }],
            relations: vec![],
        };
        let index = build_index(&graph);
        assert_eq!(index.points_for_entity("e1"), vec!["p1"]); // p1 appears once
    }

    #[test]
    fn test_edge_type_distribution() {
        let graph = PropositionGraph {
            entities: vec![
                make_entity("e1", "ATP"),
                make_entity("e2", "Calvin Cycle"),
                make_entity("e3", "Light"),
                make_entity("e4", "Membrane"),
            ],
            knowledge_points: vec![],
            relations: vec![
                make_relation("e1", "e2", RelationType::Prerequisite),
                make_relation("e1", "e3", RelationType::RelatedTo),
                make_relation("e1", "e4", RelationType::Consequence),
            ],
        };
        let index = build_index(&graph);
        let edges = index.edges_from_entity("e1");
        assert_eq!(edges.len(), 3);
        // Verify types are present
        let types: Vec<_> = edges.iter().map(|(t, _)| t).collect();
        assert!(types.contains(&&RelationType::Prerequisite));
        assert!(types.contains(&&RelationType::RelatedTo));
        assert!(types.contains(&&RelationType::Consequence));
    }

    #[test]
    fn test_deterministic_order() {
        // Build index twice from same graph, verify deterministic ordering
        let graph = PropositionGraph {
            entities: vec![
                make_entity("e1", "ATP"),
                make_entity("e2", "Energy"),
                make_entity("e3", "Chloroplast"),
            ],
            knowledge_points: vec![
                make_point("p3", &["e1", "e2"]),
                make_point("p1", &["e1"]),
                make_point("p2", &["e2"]),
            ],
            relations: vec![
                make_relation("e1", "e3", RelationType::RelatedTo),
                make_relation("e1", "e2", RelationType::Prerequisite),
            ],
        };

        let idx1 = build_index(&graph);
        let idx2 = build_index(&graph);

        assert_eq!(idx1.points_for_entity("e1"), idx2.points_for_entity("e1"));
        assert_eq!(idx1.edges_from_entity("e1"), idx2.edges_from_entity("e1"));
    }
}
