//! Bundle assembly (Pass 5a): Select related points and assemble GraphContextBundle.
//!
//! For each KnowledgePoint in the consolidated graph, builds a bundle containing:
//! - root_point: the point itself
//! - related_points: 0-3 neighboring points (1-hop only, scored by shared entities)
//! - question_type: Relational if supporting_relations exist, Recall otherwise
//! - supporting_entities/relations: entities and edges from root's perspective
//!
//! TODO: Performance optimization for scale (6000+ entities / 4000+ KPs):
//! Currently uses graph.entities.iter().find() and graph.knowledge_points.iter().find()
//! inside loops. Use index.entity(id) and index.point(id) O(1) lookups instead.
//! This is straightforward: replace all `.find()` calls with index method lookups.
//! Estimated impact: 100x+ faster for large graphs. Defer until benchmark shows bottleneck.

use std::collections::{HashMap, HashSet};

use super::graph_index::GraphIndex;
use super::types::{GraphContextBundle, KnowledgePoint, QuestionType, Relation};
use crate::services::graph_generation::types::PropositionGraph;

const MAX_RELATED_POINTS: usize = 3;

// =====================================================================
// Public API
// =====================================================================

/// Assembles GraphContextBundles from a consolidated graph.
///
/// For each KnowledgePoint, selects 0–3 related points using:
/// 1. Same-entity neighbors (points mentioning any of root's entities)
/// 2. Relation neighbors (points reachable via 1-hop outgoing edges)
/// 3. Scoring by # shared entities (no weighting)
/// 4. Deterministic ordering: score desc, then point ID
pub fn assemble_bundles(graph: &PropositionGraph, index: &GraphIndex) -> Vec<GraphContextBundle> {
    let mut bundles = Vec::new();

    for root_point in &graph.knowledge_points {
        // ── Find 1-hop neighbors ────────────────────────────────────────
        let related = find_related_points(root_point, graph, index);

        // ── Collect supporting entities and relations ────────────────────
        // Supporting relations: all relations where source OR target is in root's entities
        let mut supporting_entity_ids: HashSet<String> = root_point.entity_ids.iter().cloned().collect();
        let mut supporting_relations: Vec<Relation> = graph
            .relations
            .iter()
            .filter(|rel| {
                supporting_entity_ids.contains(&rel.source_id) || supporting_entity_ids.contains(&rel.target_id)
            })
            .cloned()
            .collect();

        // Supporting entities: all entities that appear in supporting_relations
        let mut supporting_entities = Vec::new();
        let mut seen_entity_ids = HashSet::new();
        for rel in &supporting_relations {
            if seen_entity_ids.insert(rel.source_id.clone()) {
                if let Some(entity) = graph.entities.iter().find(|e| e.id == rel.source_id) {
                    supporting_entities.push(entity.clone());
                }
            }
            if seen_entity_ids.insert(rel.target_id.clone()) {
                if let Some(entity) = graph.entities.iter().find(|e| e.id == rel.target_id) {
                    supporting_entities.push(entity.clone());
                }
            }
        }

        // ── Compute question_type based on bundle contents ───────────────
        let question_type = if supporting_relations.is_empty() {
            QuestionType::Recall
        } else {
            QuestionType::Relational
        };

        bundles.push(GraphContextBundle {
            root_point: root_point.clone(),
            related_points: related,
            question_type,
            supporting_entities,
            supporting_relations,
        });
    }

    bundles
}

// =====================================================================
// Internal helpers
// =====================================================================

/// Selects 0–3 related points for a root KnowledgePoint.
///
/// Strategy:
/// 1. Find all points that share at least one entity with root (same-entity neighbors)
/// 2. Find all points reachable via relation edges (both incoming and outgoing)
/// 3. Score each candidate by # of shared entities (counted during collection)
/// 4. Sort by score (desc) then point ID (for determinism)
/// 5. Take top 3, excluding root itself
fn find_related_points(
    root_point: &KnowledgePoint,
    graph: &PropositionGraph,
    index: &GraphIndex,
) -> Vec<KnowledgePoint> {
    let mut candidates: HashMap<String, usize> = HashMap::new(); // point_id → score

    // ── Collect same-entity neighbors ────────────────────────────────
    for entity_id in &root_point.entity_ids {
        for neighbor_id in index.points_for_entity(entity_id) {
            if neighbor_id != root_point.id {
                *candidates.entry(neighbor_id).or_insert(0) += 1;
            }
        }
    }

    // ── Collect relation neighbors (both incoming and outgoing) ────────
    for entity_id in &root_point.entity_ids {
        // Outgoing edges: entity_id is the source
        for (_, target_entity_id) in index.edges_from_entity(entity_id) {
            for neighbor_id in index.points_for_entity(&target_entity_id) {
                if neighbor_id != root_point.id {
                    *candidates.entry(neighbor_id).or_insert(0) += 1;
                }
            }
        }

        // Incoming edges: entity_id is the target
        for rel in &graph.relations {
            if rel.target_id == *entity_id {
                for neighbor_id in index.points_for_entity(&rel.source_id) {
                    if neighbor_id != root_point.id {
                        *candidates.entry(neighbor_id).or_insert(0) += 1;
                    }
                }
            }
        }
    }

    // ── Score and sort ──────────────────────────────────────────────────
    // Use score from HashMap (# shared entities already counted during neighbor collection)
    let mut scored: Vec<(String, usize)> = candidates.into_iter().collect();

    // Sort: score desc, then point_id for determinism
    scored.sort_by(|a, b| {
        b.1.cmp(&a.1) // score descending
            .then_with(|| a.0.cmp(&b.0)) // point_id ascending
    });

    // ── Retrieve and return top MAX_RELATED_POINTS ────────────────────
    let mut result = Vec::new();
    for (point_id, _) in scored.into_iter().take(MAX_RELATED_POINTS) {
        if let Some(point) = graph.knowledge_points.iter().find(|p| p.id == point_id) {
            result.push(point.clone());
        }
    }

    result
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::graph_generation::graph_index::build_index;
    use crate::services::graph_generation::types::{EntityNode, KnowledgeType, Relation, RelationType};

    fn make_entity(id: &str, name: &str) -> EntityNode {
        EntityNode {
            id: id.to_string(),
            canonical_name: name.to_string(),
            aliases: vec![name.to_string()],
            chunk_ids: vec!["c1".to_string()],
        }
    }

    fn make_point(id: &str, entity_ids: &[&str]) -> KnowledgePoint {
        KnowledgePoint {
            id: id.to_string(),
            point: format!("Test point {id}"),
            knowledge_type: KnowledgeType::Fact,
            chunk_id: "c1".to_string(),
            raw_entity_names: vec![],
            entity_ids: entity_ids.iter().map(|s| s.to_string()).collect(),
            raw_relations: vec![],
        }
    }

    fn make_relation(source_id: &str, target_id: &str) -> Relation {
        Relation {
            source_id: source_id.to_string(),
            target_id: target_id.to_string(),
            relation_type: RelationType::RelatedTo,
        }
    }

    #[test]
    fn test_isolated_point() {
        // Single point with no neighbors
        let graph = PropositionGraph {
            entities: vec![make_entity("e1", "JWT")],
            knowledge_points: vec![make_point("kp1", &["e1"])],
            relations: vec![],
        };
        let index = build_index(&graph);
        let bundles = assemble_bundles(&graph, &index);

        assert_eq!(bundles.len(), 1);
        assert_eq!(bundles[0].root_point.id, "kp1");
        assert!(bundles[0].related_points.is_empty());
        assert_eq!(bundles[0].question_type, QuestionType::Recall);
    }

    #[test]
    fn test_one_neighbour() {
        // Two points sharing one entity
        let graph = PropositionGraph {
            entities: vec![
                make_entity("e1", "JWT"),
                make_entity("e2", "Token"),
            ],
            knowledge_points: vec![
                make_point("kp1", &["e1"]),
                make_point("kp2", &["e1", "e2"]),
            ],
            relations: vec![],
        };
        let index = build_index(&graph);
        let bundles = assemble_bundles(&graph, &index);

        let kp1_bundle = bundles.iter().find(|b| b.root_point.id == "kp1").unwrap();
        assert_eq!(kp1_bundle.related_points.len(), 1);
        assert_eq!(kp1_bundle.related_points[0].id, "kp2");
    }

    #[test]
    fn test_many_neighbours_capped_at_three() {
        // Root point with many neighbors sharing different entities
        let graph = PropositionGraph {
            entities: vec![
                make_entity("e1", "Root"),
                make_entity("e2", "Neighbor1"),
                make_entity("e3", "Neighbor2"),
                make_entity("e4", "Neighbor3"),
                make_entity("e5", "Neighbor4"),
            ],
            knowledge_points: vec![
                make_point("root", &["e1"]),
                make_point("n1", &["e1", "e2"]),
                make_point("n2", &["e1", "e3"]),
                make_point("n3", &["e1", "e4"]),
                make_point("n4", &["e1", "e5"]),
            ],
            relations: vec![],
        };
        let index = build_index(&graph);
        let bundles = assemble_bundles(&graph, &index);

        let root_bundle = bundles.iter().find(|b| b.root_point.id == "root").unwrap();
        assert_eq!(root_bundle.related_points.len(), 3); // capped at MAX_RELATED_POINTS
    }

    #[test]
    fn test_duplicate_neighbour_removed() {
        // Candidate point appears multiple times in candidates set (via different entities)
        // but should only appear once in related_points
        let graph = PropositionGraph {
            entities: vec![
                make_entity("e1", "A"),
                make_entity("e2", "B"),
                make_entity("e3", "C"),
            ],
            knowledge_points: vec![
                make_point("root", &["e1", "e2"]),
                make_point("n1", &["e1", "e2", "e3"]), // shares 2 entities with root
            ],
            relations: vec![],
        };
        let index = build_index(&graph);
        let bundles = assemble_bundles(&graph, &index);

        let root_bundle = bundles.iter().find(|b| b.root_point.id == "root").unwrap();
        assert_eq!(root_bundle.related_points.len(), 1);
        assert_eq!(root_bundle.related_points[0].id, "n1");
    }

    #[test]
    fn test_deterministic_ordering() {
        // Multiple neighbors with same score should be ordered by ID
        let graph = PropositionGraph {
            entities: vec![
                make_entity("e1", "Root"),
                make_entity("e2", "A"),
                make_entity("e3", "B"),
                make_entity("e4", "C"),
            ],
            knowledge_points: vec![
                make_point("root", &["e1"]),
                make_point("z_neighbor", &["e1", "e2"]), // score 1
                make_point("a_neighbor", &["e1", "e3"]), // score 1
                make_point("m_neighbor", &["e1", "e4"]), // score 1
            ],
            relations: vec![],
        };
        let index = build_index(&graph);
        let bundles = assemble_bundles(&graph, &index);

        let root_bundle = bundles.iter().find(|b| b.root_point.id == "root").unwrap();
        assert_eq!(root_bundle.related_points.len(), 3);
        // Should be ordered by ID: a_neighbor, m_neighbor, z_neighbor
        assert_eq!(root_bundle.related_points[0].id, "a_neighbor");
        assert_eq!(root_bundle.related_points[1].id, "m_neighbor");
        assert_eq!(root_bundle.related_points[2].id, "z_neighbor");
    }

    #[test]
    fn test_question_type_relational_on_supporting_relations() {
        // Bundle with supporting relations → Relational
        let graph = PropositionGraph {
            entities: vec![
                make_entity("e1", "JWT"),
                make_entity("e2", "Cookie"),
            ],
            knowledge_points: vec![
                make_point("kp1", &["e1"]),
                make_point("kp2", &["e2"]),
            ],
            relations: vec![make_relation("e1", "e2")],
        };
        let index = build_index(&graph);
        let bundles = assemble_bundles(&graph, &index);

        let kp1_bundle = bundles.iter().find(|b| b.root_point.id == "kp1").unwrap();
        assert_eq!(kp1_bundle.question_type, QuestionType::Relational);
        assert!(!kp1_bundle.supporting_relations.is_empty());
    }

    #[test]
    fn test_incoming_relations_captured() {
        // Bundle should capture relations where root's entity is the TARGET
        let graph = PropositionGraph {
            entities: vec![
                make_entity("e1", "JWT"),
                make_entity("e2", "Cookie"),
            ],
            knowledge_points: vec![
                make_point("kp1", &["e1"]),
                make_point("kp2", &["e2"]),
            ],
            relations: vec![make_relation("e2", "e1")], // e2 → e1 (incoming to root)
        };
        let index = build_index(&graph);
        let bundles = assemble_bundles(&graph, &index);

        let kp1_bundle = bundles.iter().find(|b| b.root_point.id == "kp1").unwrap();
        assert_eq!(kp1_bundle.question_type, QuestionType::Relational);
        assert_eq!(kp1_bundle.supporting_relations.len(), 1);
        assert_eq!(kp1_bundle.supporting_relations[0].source_id, "e2");
        assert_eq!(kp1_bundle.supporting_relations[0].target_id, "e1");
    }

    #[test]
    fn test_question_type_recall_on_empty_supporting_relations() {
        // Bundle with no supporting relations → Recall
        let graph = PropositionGraph {
            entities: vec![make_entity("e1", "Bubble Sort")],
            knowledge_points: vec![make_point("kp1", &["e1"])],
            relations: vec![],
        };
        let index = build_index(&graph);
        let bundles = assemble_bundles(&graph, &index);

        let kp1_bundle = bundles.iter().find(|b| b.root_point.id == "kp1").unwrap();
        assert_eq!(kp1_bundle.question_type, QuestionType::Recall);
        assert!(kp1_bundle.supporting_relations.is_empty());
    }

    #[test]
    fn test_root_never_appears_in_related() {
        // Root should never include itself in related_points
        let graph = PropositionGraph {
            entities: vec![make_entity("e1", "A")],
            knowledge_points: vec![make_point("root", &["e1"])],
            relations: vec![],
        };
        let index = build_index(&graph);
        let bundles = assemble_bundles(&graph, &index);

        let root_bundle = bundles.iter().find(|b| b.root_point.id == "root").unwrap();
        assert!(!root_bundle.related_points.iter().any(|p| p.id == "root"));
    }

    #[test]
    fn test_incoming_relation_neighbours_captured() {
        // Root = Cookies should find JWT via incoming edge (JWT → Cookies)
        let graph = PropositionGraph {
            entities: vec![
                make_entity("e1", "JWT"),
                make_entity("e2", "Cookies"),
            ],
            knowledge_points: vec![
                make_point("kp_jwt", &["e1"]),
                make_point("kp_cookies", &["e2"]),
            ],
            relations: vec![make_relation("e1", "e2")], // JWT → Cookies
        };
        let index = build_index(&graph);
        let bundles = assemble_bundles(&graph, &index);

        // Cookies bundle should include JWT as a relation neighbor
        let cookies_bundle = bundles.iter().find(|b| b.root_point.id == "kp_cookies").unwrap();
        assert_eq!(cookies_bundle.related_points.len(), 1);
        assert_eq!(cookies_bundle.related_points[0].id, "kp_jwt");
    }

    #[test]
    fn test_scoring_by_shared_entities() {
        // Verify candidates are scored by # shared entities
        let graph = PropositionGraph {
            entities: vec![
                make_entity("e1", "JWT"),
                make_entity("e2", "Cookie"),
                make_entity("e3", "Session"),
            ],
            knowledge_points: vec![
                make_point("root", &["e1", "e2"]),
                make_point("c1", &["e1"]), // score 1
                make_point("c2", &["e1", "e2"]), // score 2 → should rank first
                make_point("c3", &["e2", "e3"]), // score 1
            ],
            relations: vec![],
        };
        let index = build_index(&graph);
        let bundles = assemble_bundles(&graph, &index);

        let root_bundle = bundles.iter().find(|b| b.root_point.id == "root").unwrap();
        assert_eq!(root_bundle.related_points.len(), 3);
        // c2 should be first (score 2)
        assert_eq!(root_bundle.related_points[0].id, "c2");
        // Then c1 and c3 (score 1) ordered by ID
        assert_eq!(root_bundle.related_points[1].id, "c1");
        assert_eq!(root_bundle.related_points[2].id, "c3");
    }
}
