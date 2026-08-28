//! Applies a validated [`EntityMergePlan`] to a [`PropositionGraph`].
//!
//! Rewriting is a pure transformation: the input graph is borrowed and a new
//! graph is returned. The complete graph and plan are validated before output
//! construction, so an error cannot leave the caller with a partially mutated
//! graph.
//!
//! ```text
//! PropositionGraph + EntityMergePlan
//!                 ↓
//! validate graph references and disjoint merge groups
//!                 ↓
//! merge entity aliases and chunk provenance
//!                 ↓
//! rewrite KnowledgePoint.entity_ids
//!                 ↓
//! rewrite and deduplicate resolved relations
//!                 ↓
//! rewritten PropositionGraph
//! ```

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;

use super::super::types::{EntityNode, PropositionGraph, Relation};
use super::merge_planner::EntityMergePlan;

/// Structural graph or merge-plan problem found before rewriting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphRewriteError {
    DuplicateGraphEntityId {
        entity_id: String,
    },
    UnknownKnowledgePointEntityId {
        point_id: String,
        entity_id: String,
    },
    UnknownRelationEntityId {
        relation_index: usize,
        endpoint: &'static str,
        entity_id: String,
    },
    EmptyMergeGroup {
        canonical_entity_id: String,
    },
    UnknownPlanEntityId {
        entity_id: String,
    },
    EntityAssignedToMultipleMerges {
        entity_id: String,
    },
    CanonicalEntityListedAsMerged {
        entity_id: String,
    },
    CanonicalEntityIsNotEarliest {
        canonical_entity_id: String,
        earliest_entity_id: String,
    },
}

impl fmt::Display for GraphRewriteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateGraphEntityId { entity_id } => {
                write!(formatter, "Rewrite graph contains duplicate entity ID '{entity_id}'")
            }
            Self::UnknownKnowledgePointEntityId {
                point_id,
                entity_id,
            } => write!(
                formatter,
                "Knowledge point '{point_id}' references unknown entity ID '{entity_id}'"
            ),
            Self::UnknownRelationEntityId {
                relation_index,
                endpoint,
                entity_id,
            } => write!(
                formatter,
                "Relation {relation_index} {endpoint} references unknown entity ID '{entity_id}'"
            ),
            Self::EmptyMergeGroup {
                canonical_entity_id,
            } => write!(
                formatter,
                "Merge for canonical entity '{canonical_entity_id}' has no merged entities"
            ),
            Self::UnknownPlanEntityId { entity_id } => {
                write!(formatter, "Merge plan references unknown entity ID '{entity_id}'")
            }
            Self::EntityAssignedToMultipleMerges { entity_id } => write!(
                formatter,
                "Entity '{entity_id}' is assigned more than once in the merge plan"
            ),
            Self::CanonicalEntityListedAsMerged { entity_id } => write!(
                formatter,
                "Canonical entity '{entity_id}' is also listed as its own merged entity"
            ),
            Self::CanonicalEntityIsNotEarliest {
                canonical_entity_id,
                earliest_entity_id,
            } => write!(
                formatter,
                "Canonical entity '{canonical_entity_id}' is not the earliest graph member; expected '{earliest_entity_id}'"
            ),
        }
    }
}

impl Error for GraphRewriteError {}

struct ValidatedRewritePlan {
    /// Every removed entity ID maps to the stable ID that survives.
    replacement_by_id: HashMap<String, String>,
    /// Canonical ID maps to all group member indices in original graph order.
    member_indices_by_canonical: HashMap<String, Vec<usize>>,
}

/// Applies an entity merge plan and returns a fully rewritten graph.
///
/// For each merge group this function:
///
/// - retains the canonical entity's ID, name, and graph position;
/// - appends every member's canonical name and aliases without duplicates;
/// - combines chunk IDs without duplicates;
/// - replaces removed IDs in knowledge points and relations;
/// - removes self-relations created by collapsing two different entities; and
/// - deduplicates identical `(source, target, relation_type)` relations.
///
/// `raw_entity_names` and `raw_relations` remain unchanged because they record
/// the original extracted wording rather than resolved graph identity.
///
/// # Errors
///
/// Returns [`GraphRewriteError`] when the source graph has ambiguous or dangling
/// IDs, or when the supplied plan is unknown, overlapping, empty, or violates
/// the planner's earliest-canonical contract.
pub fn rewrite_graph_with_entity_merges(
    graph: &PropositionGraph,
    plan: &EntityMergePlan,
) -> Result<PropositionGraph, GraphRewriteError> {
    let entity_index = validate_graph_references(graph)?;
    let validated_plan = validate_plan(graph, plan, &entity_index)?;

    let entities = rewrite_entities(graph, &validated_plan);
    let knowledge_points = graph
        .knowledge_points
        .iter()
        .cloned()
        .map(|mut point| {
            point.entity_ids =
                rewrite_id_list(&point.entity_ids, &validated_plan.replacement_by_id);
            point
        })
        .collect();
    let relations = rewrite_relations(graph, &validated_plan.replacement_by_id);

    Ok(PropositionGraph {
        entities,
        knowledge_points,
        relations,
    })
}

/// Validates stable entity identity and every resolved graph reference.
fn validate_graph_references(
    graph: &PropositionGraph,
) -> Result<HashMap<&str, usize>, GraphRewriteError> {
    let mut entity_index = HashMap::with_capacity(graph.entities.len());
    for (index, entity) in graph.entities.iter().enumerate() {
        if entity_index.insert(entity.id.as_str(), index).is_some() {
            return Err(GraphRewriteError::DuplicateGraphEntityId {
                entity_id: entity.id.clone(),
            });
        }
    }

    for point in &graph.knowledge_points {
        for entity_id in &point.entity_ids {
            if !entity_index.contains_key(entity_id.as_str()) {
                return Err(GraphRewriteError::UnknownKnowledgePointEntityId {
                    point_id: point.id.clone(),
                    entity_id: entity_id.clone(),
                });
            }
        }
    }

    for (relation_index, relation) in graph.relations.iter().enumerate() {
        for (endpoint, entity_id) in [
            ("source_id", &relation.source_id),
            ("target_id", &relation.target_id),
        ] {
            if !entity_index.contains_key(entity_id.as_str()) {
                return Err(GraphRewriteError::UnknownRelationEntityId {
                    relation_index,
                    endpoint,
                    entity_id: entity_id.clone(),
                });
            }
        }
    }

    Ok(entity_index)
}

/// Validates all merge groups and precomputes rewrite lookups.
fn validate_plan(
    graph: &PropositionGraph,
    plan: &EntityMergePlan,
    entity_index: &HashMap<&str, usize>,
) -> Result<ValidatedRewritePlan, GraphRewriteError> {
    let mut assigned_ids = HashSet::new();
    let mut replacement_by_id = HashMap::new();
    let mut member_indices_by_canonical = HashMap::new();

    for entity_merge in &plan.merges {
        if entity_merge.merged_entity_ids.is_empty() {
            return Err(GraphRewriteError::EmptyMergeGroup {
                canonical_entity_id: entity_merge.canonical_entity_id.clone(),
            });
        }

        let canonical_index =
            plan_entity_index(entity_index, entity_merge.canonical_entity_id.as_str())?;
        if !assigned_ids.insert(entity_merge.canonical_entity_id.as_str()) {
            return Err(GraphRewriteError::EntityAssignedToMultipleMerges {
                entity_id: entity_merge.canonical_entity_id.clone(),
            });
        }

        let mut member_indices = vec![canonical_index];
        for merged_id in &entity_merge.merged_entity_ids {
            if merged_id == &entity_merge.canonical_entity_id {
                return Err(GraphRewriteError::CanonicalEntityListedAsMerged {
                    entity_id: merged_id.clone(),
                });
            }
            let merged_index = plan_entity_index(entity_index, merged_id)?;
            if !assigned_ids.insert(merged_id.as_str()) {
                return Err(GraphRewriteError::EntityAssignedToMultipleMerges {
                    entity_id: merged_id.clone(),
                });
            }

            member_indices.push(merged_index);
            replacement_by_id.insert(merged_id.clone(), entity_merge.canonical_entity_id.clone());
        }

        // The planner promises graph-order canonical selection. Enforcing it
        // here prevents a manually constructed plan from changing that policy.
        member_indices.sort_unstable();
        if member_indices[0] != canonical_index {
            return Err(GraphRewriteError::CanonicalEntityIsNotEarliest {
                canonical_entity_id: entity_merge.canonical_entity_id.clone(),
                earliest_entity_id: graph.entities[member_indices[0]].id.clone(),
            });
        }

        member_indices_by_canonical
            .insert(entity_merge.canonical_entity_id.clone(), member_indices);
    }

    Ok(ValidatedRewritePlan {
        replacement_by_id,
        member_indices_by_canonical,
    })
}

fn plan_entity_index(
    entity_index: &HashMap<&str, usize>,
    entity_id: &str,
) -> Result<usize, GraphRewriteError> {
    entity_index
        .get(entity_id)
        .copied()
        .ok_or_else(|| GraphRewriteError::UnknownPlanEntityId {
            entity_id: entity_id.to_string(),
        })
}

fn rewrite_entities(graph: &PropositionGraph, plan: &ValidatedRewritePlan) -> Vec<EntityNode> {
    let mut rewritten = Vec::with_capacity(
        graph
            .entities
            .len()
            .saturating_sub(plan.replacement_by_id.len()),
    );

    for entity in &graph.entities {
        // Removed members are represented by their earlier canonical entity.
        if plan.replacement_by_id.contains_key(entity.id.as_str()) {
            continue;
        }

        let Some(member_indices) = plan.member_indices_by_canonical.get(&entity.id) else {
            rewritten.push(entity.clone());
            continue;
        };

        let mut merged_entity = entity.clone();
        merged_entity.aliases.clear();
        merged_entity.chunk_ids.clear();

        let mut seen_aliases = HashSet::new();
        let mut seen_chunk_ids = HashSet::new();
        for member_index in member_indices {
            let member = &graph.entities[*member_index];
            push_unique(
                &mut merged_entity.aliases,
                &mut seen_aliases,
                &member.canonical_name,
            );
            for alias in &member.aliases {
                push_unique(&mut merged_entity.aliases, &mut seen_aliases, alias);
            }
            for chunk_id in &member.chunk_ids {
                push_unique(&mut merged_entity.chunk_ids, &mut seen_chunk_ids, chunk_id);
            }
        }

        rewritten.push(merged_entity);
    }

    rewritten
}

fn rewrite_id_list(ids: &[String], replacement_by_id: &HashMap<String, String>) -> Vec<String> {
    let mut rewritten = Vec::with_capacity(ids.len());
    let mut seen = HashSet::with_capacity(ids.len());

    for entity_id in ids {
        let resolved = replacement_by_id.get(entity_id).unwrap_or(entity_id);
        if seen.insert(resolved.as_str()) {
            rewritten.push(resolved.clone());
        }
    }

    rewritten
}

fn rewrite_relations(
    graph: &PropositionGraph,
    replacement_by_id: &HashMap<String, String>,
) -> Vec<Relation> {
    let mut rewritten = Vec::with_capacity(graph.relations.len());
    let mut seen = HashSet::with_capacity(graph.relations.len());

    for relation in &graph.relations {
        let source_id = replacement_by_id
            .get(&relation.source_id)
            .unwrap_or(&relation.source_id)
            .clone();
        let target_id = replacement_by_id
            .get(&relation.target_id)
            .unwrap_or(&relation.target_id)
            .clone();

        // Remove only self-relations introduced by collapsing two previously
        // distinct entities. An existing intentional self-relation is retained.
        if relation.source_id != relation.target_id && source_id == target_id {
            continue;
        }

        let key = (source_id.clone(), target_id.clone(), relation.relation_type);
        if seen.insert(key) {
            rewritten.push(Relation {
                source_id,
                target_id,
                relation_type: relation.relation_type,
            });
        }
    }

    rewritten
}

fn push_unique(target: &mut Vec<String>, seen: &mut HashSet<String>, value: &str) {
    if seen.insert(value.to_string()) {
        target.push(value.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::graph_generation::entity_resolution::merge_planner::EntityMerge;
    use crate::services::graph_generation::types::{
        KnowledgePoint, KnowledgeType, RelationRef, RelationType,
    };

    fn entity(id: &str, name: &str, aliases: &[&str], chunks: &[&str]) -> EntityNode {
        EntityNode {
            id: id.to_string(),
            canonical_name: name.to_string(),
            aliases: aliases.iter().map(|value| value.to_string()).collect(),
            chunk_ids: chunks.iter().map(|value| value.to_string()).collect(),
        }
    }

    fn point(id: &str, entity_ids: &[&str]) -> KnowledgePoint {
        KnowledgePoint {
            id: id.to_string(),
            point: String::from("Test knowledge point."),
            knowledge_type: KnowledgeType::Fact,
            chunk_id: String::from("chunk-1"),
            raw_entity_names: vec![String::from("original wording")],
            entity_ids: entity_ids.iter().map(|value| value.to_string()).collect(),
            raw_relations: vec![RelationRef {
                target_entity_name: String::from("original target wording"),
                relation_type: RelationType::RelatedTo,
                source_quote: None,
            }],
        }
    }

    fn relation(source: &str, target: &str, relation_type: RelationType) -> Relation {
        Relation {
            source_id: source.to_string(),
            target_id: target.to_string(),
            relation_type,
        }
    }

    fn merge(canonical: &str, merged: &[&str]) -> EntityMerge {
        EntityMerge {
            canonical_entity_id: canonical.to_string(),
            merged_entity_ids: merged.iter().map(|value| value.to_string()).collect(),
        }
    }

    fn graph() -> PropositionGraph {
        PropositionGraph {
            entities: vec![
                entity("co2-symbol", "CO₂", &["CO₂", "CO2"], &["chunk-1"]),
                entity(
                    "carbon-dioxide",
                    "carbon dioxide",
                    &["carbon dioxide", "CO2"],
                    &["chunk-2"],
                ),
                entity("oxygen", "oxygen", &["oxygen"], &["chunk-3"]),
            ],
            knowledge_points: vec![point("kp-1", &["co2-symbol", "carbon-dioxide", "oxygen"])],
            relations: Vec::new(),
        }
    }

    #[test]
    fn merges_entity_aliases_and_chunks_in_graph_order() {
        let rewritten = rewrite_graph_with_entity_merges(
            &graph(),
            &EntityMergePlan {
                merges: vec![merge("co2-symbol", &["carbon-dioxide"])],
            },
        )
        .expect("valid plan should rewrite entities");

        assert_eq!(rewritten.entities.len(), 2);
        assert_eq!(rewritten.entities[0].id, "co2-symbol");
        assert_eq!(rewritten.entities[0].canonical_name, "CO₂");
        assert_eq!(
            rewritten.entities[0].aliases,
            vec!["CO₂", "CO2", "carbon dioxide"]
        );
        assert_eq!(rewritten.entities[0].chunk_ids, vec!["chunk-1", "chunk-2"]);
        assert_eq!(rewritten.entities[1].id, "oxygen");
    }

    #[test]
    fn rewrites_and_deduplicates_knowledge_point_ids_but_preserves_raw_evidence() {
        let rewritten = rewrite_graph_with_entity_merges(
            &graph(),
            &EntityMergePlan {
                merges: vec![merge("co2-symbol", &["carbon-dioxide"])],
            },
        )
        .expect("valid plan should rewrite knowledge points");
        let point = &rewritten.knowledge_points[0];

        assert_eq!(point.entity_ids, vec!["co2-symbol", "oxygen"]);
        assert_eq!(point.raw_entity_names, vec!["original wording"]);
        assert_eq!(
            point.raw_relations[0].target_entity_name,
            "original target wording"
        );
    }

    #[test]
    fn rewrites_relations_deduplicates_triples_and_removes_created_self_relations() {
        let mut source = graph();
        source.relations = vec![
            relation("co2-symbol", "oxygen", RelationType::RelatedTo),
            relation("carbon-dioxide", "oxygen", RelationType::RelatedTo),
            relation("carbon-dioxide", "oxygen", RelationType::Consequence),
            relation("co2-symbol", "carbon-dioxide", RelationType::RelatedTo),
            relation("oxygen", "oxygen", RelationType::RelatedTo),
        ];
        let rewritten = rewrite_graph_with_entity_merges(
            &source,
            &EntityMergePlan {
                merges: vec![merge("co2-symbol", &["carbon-dioxide"])],
            },
        )
        .expect("valid plan should rewrite relations");

        assert_eq!(
            rewritten
                .relations
                .iter()
                .map(|relation| (
                    relation.source_id.as_str(),
                    relation.target_id.as_str(),
                    relation.relation_type,
                ))
                .collect::<Vec<_>>(),
            vec![
                ("co2-symbol", "oxygen", RelationType::RelatedTo),
                ("co2-symbol", "oxygen", RelationType::Consequence),
                ("oxygen", "oxygen", RelationType::RelatedTo),
            ]
        );
    }

    #[test]
    fn an_empty_plan_returns_an_unchanged_graph() {
        let source = graph();
        let rewritten = rewrite_graph_with_entity_merges(&source, &EntityMergePlan::default())
            .expect("empty plan is valid");

        assert_eq!(
            serde_json::to_value(rewritten).expect("rewritten graph should serialize"),
            serde_json::to_value(source).expect("source graph should serialize")
        );
    }

    #[test]
    fn rejects_unknown_overlapping_empty_and_non_earliest_plan_groups() {
        let source = graph();
        assert!(matches!(
            rewrite_graph_with_entity_merges(
                &source,
                &EntityMergePlan {
                    merges: vec![merge("co2-symbol", &["missing"])]
                }
            ),
            Err(GraphRewriteError::UnknownPlanEntityId { entity_id }) if entity_id == "missing"
        ));
        assert!(matches!(
            rewrite_graph_with_entity_merges(
                &source,
                &EntityMergePlan {
                    merges: vec![
                        merge("co2-symbol", &["carbon-dioxide"]),
                        merge("oxygen", &["carbon-dioxide"])
                    ]
                }
            ),
            Err(GraphRewriteError::EntityAssignedToMultipleMerges { entity_id })
                if entity_id == "carbon-dioxide"
        ));
        assert!(matches!(
            rewrite_graph_with_entity_merges(
                &source,
                &EntityMergePlan {
                    merges: vec![merge("co2-symbol", &[])]
                }
            ),
            Err(GraphRewriteError::EmptyMergeGroup { .. })
        ));
        assert!(matches!(
            rewrite_graph_with_entity_merges(
                &source,
                &EntityMergePlan {
                    merges: vec![merge("carbon-dioxide", &["co2-symbol"])]
                }
            ),
            Err(GraphRewriteError::CanonicalEntityIsNotEarliest { .. })
        ));
    }

    #[test]
    fn rejects_a_canonical_entity_repeated_inside_its_group() {
        let source = graph();
        let error = rewrite_graph_with_entity_merges(
            &source,
            &EntityMergePlan {
                merges: vec![merge("co2-symbol", &["co2-symbol"])],
            },
        )
        .expect_err("canonical ID cannot also be removed");

        assert_eq!(
            error,
            GraphRewriteError::CanonicalEntityListedAsMerged {
                entity_id: String::from("co2-symbol")
            }
        );
    }

    #[test]
    fn rejects_duplicate_or_dangling_source_graph_ids() {
        let mut duplicate = graph();
        duplicate.entities.push(entity("oxygen", "O₂", &[], &[]));
        assert!(matches!(
            rewrite_graph_with_entity_merges(&duplicate, &EntityMergePlan::default()),
            Err(GraphRewriteError::DuplicateGraphEntityId { entity_id }) if entity_id == "oxygen"
        ));

        let mut dangling_point = graph();
        dangling_point.knowledge_points[0]
            .entity_ids
            .push(String::from("missing"));
        assert!(matches!(
            rewrite_graph_with_entity_merges(&dangling_point, &EntityMergePlan::default()),
            Err(GraphRewriteError::UnknownKnowledgePointEntityId { entity_id, .. })
                if entity_id == "missing"
        ));

        let mut dangling_relation = graph();
        dangling_relation
            .relations
            .push(relation("missing", "oxygen", RelationType::RelatedTo));
        assert!(matches!(
            rewrite_graph_with_entity_merges(&dangling_relation, &EntityMergePlan::default()),
            Err(GraphRewriteError::UnknownRelationEntityId {
                endpoint: "source_id",
                entity_id,
                ..
            }) if entity_id == "missing"
        ));
    }
}
