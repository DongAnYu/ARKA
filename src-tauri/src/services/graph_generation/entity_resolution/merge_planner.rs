//! Pure merge planning for semantically verified entity pairs.
//!
//! This stage decides *which stable entity IDs belong together* and which
//! existing entity survives. It does not mutate [`PropositionGraph`]. Keeping
//! planning separate from rewriting makes the proposed changes inspectable and
//! lets graph mutation validate the complete plan before applying anything.
//!
//! ```text
//! PropositionGraph + VerifiedEntityCandidate
//!                   ↓
//! validate graph IDs and pair decisions
//!                   ↓
//! connect only SameEntity pairs
//!                   ↓
//! collect transitive connected components
//!                   ↓
//! keep earliest entity in graph order
//!                   ↓
//! EntityMergePlan
//! ```

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;

use super::super::types::PropositionGraph;
use super::semantic_verifier::{EntityMatchDecision, VerifiedEntityCandidate};

/// One transitive entity group to collapse during graph rewriting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityMerge {
    /// Stable ID retained after the merge.
    ///
    /// This is the group member that appears earliest in `graph.entities`.
    pub canonical_entity_id: String,
    /// Stable IDs removed and rewritten to `canonical_entity_id`.
    ///
    /// IDs follow their original graph order and exclude the canonical ID.
    pub merged_entity_ids: Vec<String>,
}

/// Complete deterministic set of entity merges proposed for one graph.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EntityMergePlan {
    /// Merge groups ordered by their canonical entity's graph position.
    pub merges: Vec<EntityMerge>,
}

/// Invalid graph identity or contradictory verifier input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergePlanningError {
    /// `graph.entities` contains the same stable ID more than once.
    DuplicateGraphEntityId { entity_id: String },
    /// A verified pair references an entity absent from the graph.
    UnknownEntityId { entity_id: String },
    /// A malformed verified result compares one entity to itself.
    SelfPair { entity_id: String },
    /// The same unordered pair has incompatible semantic decisions.
    ConflictingPairDecisions {
        left_id: String,
        right_id: String,
        first: EntityMatchDecision,
        second: EntityMatchDecision,
    },
}

impl fmt::Display for MergePlanningError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateGraphEntityId { entity_id } => {
                write!(formatter, "Merge graph contains duplicate entity ID '{entity_id}'")
            }
            Self::UnknownEntityId { entity_id } => write!(
                formatter,
                "Verified merge candidate references unknown entity ID '{entity_id}'"
            ),
            Self::SelfPair { entity_id } => write!(
                formatter,
                "Verified merge candidate compares entity '{entity_id}' with itself"
            ),
            Self::ConflictingPairDecisions {
                left_id,
                right_id,
                first,
                second,
            } => write!(
                formatter,
                "Verified pair '{left_id}' and '{right_id}' has conflicting decisions {first:?} and {second:?}"
            ),
        }
    }
}

impl Error for MergePlanningError {}

/// Builds a deterministic merge plan from semantic verification results.
///
/// Only [`EntityMatchDecision::SameEntity`] creates an edge. Connected edges
/// are transitive: if `A = B` and `B = C`, the plan contains one `A/B/C` group.
/// `DifferentEntity` and `Uncertain` are validated but never merged.
///
/// Canonical selection uses original graph order, not verifier result order.
/// Therefore reordering `verified` cannot change the surviving entity or plan.
///
/// # Errors
///
/// Returns [`MergePlanningError`] when graph IDs are ambiguous, a result refers
/// to an unknown entity, a self-pair is supplied, or one pair has contradictory
/// decisions.
pub fn build_entity_merge_plan(
    graph: &PropositionGraph,
    verified: &[VerifiedEntityCandidate],
) -> Result<EntityMergePlan, MergePlanningError> {
    let entity_index = build_entity_index(graph)?;
    let mut adjacency = vec![Vec::new(); graph.entities.len()];
    let mut pair_decisions = HashMap::with_capacity(verified.len());

    for result in verified {
        let left_index = lookup_entity_index(&entity_index, &result.entity_id)?;
        let right_index = lookup_entity_index(&entity_index, &result.candidate_entity_id)?;

        if left_index == right_index {
            return Err(MergePlanningError::SelfPair {
                entity_id: result.entity_id.clone(),
            });
        }

        // Pair identity is unordered. A→B and B→A must share one validation
        // record so duplicate confirmations are harmless and conflicts surface.
        let pair = if left_index < right_index {
            (left_index, right_index)
        } else {
            (right_index, left_index)
        };
        if let Some(first) = pair_decisions.insert(pair, result.decision) {
            if first != result.decision {
                return Err(MergePlanningError::ConflictingPairDecisions {
                    left_id: graph.entities[pair.0].id.clone(),
                    right_id: graph.entities[pair.1].id.clone(),
                    first,
                    second: result.decision,
                });
            }

            // The same decision for a duplicate/reverse pair adds no new edge.
            continue;
        }

        if result.decision == EntityMatchDecision::SameEntity {
            adjacency[left_index].push(right_index);
            adjacency[right_index].push(left_index);
        }
    }

    Ok(EntityMergePlan {
        merges: connected_merge_groups(graph, &adjacency),
    })
}

/// Maps every stable ID to its authoritative position in `graph.entities`.
fn build_entity_index(
    graph: &PropositionGraph,
) -> Result<HashMap<&str, usize>, MergePlanningError> {
    let mut entity_index = HashMap::with_capacity(graph.entities.len());
    for (index, entity) in graph.entities.iter().enumerate() {
        if entity_index.insert(entity.id.as_str(), index).is_some() {
            return Err(MergePlanningError::DuplicateGraphEntityId {
                entity_id: entity.id.clone(),
            });
        }
    }

    Ok(entity_index)
}

fn lookup_entity_index(
    entity_index: &HashMap<&str, usize>,
    entity_id: &str,
) -> Result<usize, MergePlanningError> {
    entity_index
        .get(entity_id)
        .copied()
        .ok_or_else(|| MergePlanningError::UnknownEntityId {
            entity_id: entity_id.to_string(),
        })
}

/// Finds transitive `SameEntity` components in stable graph order.
fn connected_merge_groups(graph: &PropositionGraph, adjacency: &[Vec<usize>]) -> Vec<EntityMerge> {
    let mut visited = HashSet::with_capacity(graph.entities.len());
    let mut merges = Vec::new();

    // Starting roots in graph order makes group output deterministic and makes
    // the first member of each sorted component the canonical entity.
    for start in 0..graph.entities.len() {
        if adjacency[start].is_empty() || visited.contains(&start) {
            continue;
        }

        let mut stack = vec![start];
        let mut members = Vec::new();
        visited.insert(start);

        while let Some(current) = stack.pop() {
            members.push(current);
            for &neighbour in &adjacency[current] {
                if visited.insert(neighbour) {
                    stack.push(neighbour);
                }
            }
        }

        // Traversal order depends on edge insertion order; graph-index sorting
        // removes that dependency before canonical selection and plan output.
        members.sort_unstable();
        let canonical_index = members[0];
        merges.push(EntityMerge {
            canonical_entity_id: graph.entities[canonical_index].id.clone(),
            merged_entity_ids: members[1..]
                .iter()
                .map(|index| graph.entities[*index].id.clone())
                .collect(),
        });
    }

    merges
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::graph_generation::entity_resolution::semantic_verifier::EntityVerificationSource;
    use crate::services::graph_generation::types::EntityNode;

    fn entity(id: &str, name: &str) -> EntityNode {
        EntityNode {
            id: id.to_string(),
            canonical_name: name.to_string(),
            aliases: vec![name.to_string()],
            chunk_ids: Vec::new(),
        }
    }

    fn graph(ids: &[&str]) -> PropositionGraph {
        PropositionGraph {
            entities: ids.iter().map(|id| entity(id, id)).collect(),
            knowledge_points: Vec::new(),
            relations: Vec::new(),
        }
    }

    fn verified(left: &str, right: &str, decision: EntityMatchDecision) -> VerifiedEntityCandidate {
        VerifiedEntityCandidate {
            entity_id: left.to_string(),
            candidate_entity_id: right.to_string(),
            similarity: 0.8,
            decision,
            reason: String::from("Test decision."),
            source: EntityVerificationSource::Llm,
        }
    }

    #[test]
    fn plans_a_simple_merge_and_keeps_the_earliest_graph_entity() {
        let graph = graph(&["co2-symbol", "carbon-dioxide"]);
        let plan = build_entity_merge_plan(
            &graph,
            &[verified(
                "carbon-dioxide",
                "co2-symbol",
                EntityMatchDecision::SameEntity,
            )],
        )
        .expect("valid same-entity pair should produce a plan");

        assert_eq!(
            plan,
            EntityMergePlan {
                merges: vec![EntityMerge {
                    canonical_entity_id: String::from("co2-symbol"),
                    merged_entity_ids: vec![String::from("carbon-dioxide")],
                }]
            }
        );
    }

    #[test]
    fn different_and_uncertain_decisions_do_not_create_merges() {
        let graph = graph(&["co2", "carbon-fixation", "co2-concentration"]);
        let plan = build_entity_merge_plan(
            &graph,
            &[
                verified(
                    "co2",
                    "carbon-fixation",
                    EntityMatchDecision::DifferentEntity,
                ),
                verified("co2", "co2-concentration", EntityMatchDecision::Uncertain),
            ],
        )
        .expect("non-merge decisions are valid planner input");

        assert!(plan.merges.is_empty());
    }

    #[test]
    fn combines_transitive_matches_into_one_graph_ordered_group() {
        let graph = graph(&["co2-symbol", "co2", "carbon-dioxide", "oxygen"]);
        let plan = build_entity_merge_plan(
            &graph,
            &[
                verified("co2", "carbon-dioxide", EntityMatchDecision::SameEntity),
                verified("co2-symbol", "co2", EntityMatchDecision::SameEntity),
            ],
        )
        .expect("transitive pairs should produce one group");

        assert_eq!(
            plan.merges,
            vec![EntityMerge {
                canonical_entity_id: String::from("co2-symbol"),
                merged_entity_ids: vec![String::from("co2"), String::from("carbon-dioxide")],
            }]
        );
    }

    #[test]
    fn plan_is_independent_of_verified_pair_order_and_direction() {
        let graph = graph(&["a", "b", "c", "d"]);
        let first = vec![
            verified("b", "c", EntityMatchDecision::SameEntity),
            verified("d", "a", EntityMatchDecision::SameEntity),
        ];
        let second = vec![
            verified("a", "d", EntityMatchDecision::SameEntity),
            verified("c", "b", EntityMatchDecision::SameEntity),
        ];

        let first_plan = build_entity_merge_plan(&graph, &first).expect("first plan should work");
        let second_plan =
            build_entity_merge_plan(&graph, &second).expect("second plan should work");

        assert_eq!(first_plan, second_plan);
        assert_eq!(
            first_plan.merges,
            vec![
                EntityMerge {
                    canonical_entity_id: String::from("a"),
                    merged_entity_ids: vec![String::from("d")],
                },
                EntityMerge {
                    canonical_entity_id: String::from("b"),
                    merged_entity_ids: vec![String::from("c")],
                }
            ]
        );
    }

    #[test]
    fn duplicate_and_reverse_confirmations_are_idempotent() {
        let graph = graph(&["a", "b"]);
        let plan = build_entity_merge_plan(
            &graph,
            &[
                verified("a", "b", EntityMatchDecision::SameEntity),
                verified("b", "a", EntityMatchDecision::SameEntity),
                verified("a", "b", EntityMatchDecision::SameEntity),
            ],
        )
        .expect("duplicate confirmations should not duplicate members");

        assert_eq!(plan.merges.len(), 1);
        assert_eq!(plan.merges[0].merged_entity_ids, vec!["b"]);
    }

    #[test]
    fn rejects_unknown_ids_self_pairs_and_duplicate_graph_ids() {
        let valid_graph = graph(&["a", "b"]);
        assert!(matches!(
            build_entity_merge_plan(
                &valid_graph,
                &[verified("a", "missing", EntityMatchDecision::SameEntity)]
            ),
            Err(MergePlanningError::UnknownEntityId { entity_id }) if entity_id == "missing"
        ));
        assert!(matches!(
            build_entity_merge_plan(
                &valid_graph,
                &[verified("a", "a", EntityMatchDecision::SameEntity)]
            ),
            Err(MergePlanningError::SelfPair { entity_id }) if entity_id == "a"
        ));

        let duplicate_graph = graph(&["a", "a"]);
        assert!(matches!(
            build_entity_merge_plan(&duplicate_graph, &[]),
            Err(MergePlanningError::DuplicateGraphEntityId { entity_id }) if entity_id == "a"
        ));
    }

    #[test]
    fn rejects_conflicting_decisions_for_the_same_pair() {
        let graph = graph(&["a", "b"]);
        let error = build_entity_merge_plan(
            &graph,
            &[
                verified("a", "b", EntityMatchDecision::SameEntity),
                verified("b", "a", EntityMatchDecision::DifferentEntity),
            ],
        )
        .expect_err("contradictory evidence must not authorize a merge");

        assert!(matches!(
            error,
            MergePlanningError::ConflictingPairDecisions {
                left_id,
                right_id,
                first: EntityMatchDecision::SameEntity,
                second: EntityMatchDecision::DifferentEntity,
            } if left_id == "a" && right_id == "b"
        ));
    }

    #[test]
    fn empty_verification_results_produce_an_empty_plan() {
        let plan =
            build_entity_merge_plan(&graph(&["a"]), &[]).expect("an empty verified batch is valid");

        assert!(plan.merges.is_empty());
    }
}
