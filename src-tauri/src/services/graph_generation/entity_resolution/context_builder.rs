use std::collections::{HashMap, HashSet};

use super::super::types::PropositionGraph;

/// The stable entity identity and source evidence used to create an embedding.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityContext {
    pub entity_id: String,
    pub canonical_name: String,
    pub aliases: Vec<String>,
    pub knowledge_points: Vec<String>,
}

impl EntityContext {
    /// Formats the entity and its evidence as deterministic embedding input.
    ///
    /// Entities without knowledge points still produce useful input containing
    /// their canonical name and aliases.
    pub fn embedding_text(&self) -> String {
        let mut text = format!(
            "Entity: {}\nAliases: {}",
            self.canonical_name,
            self.aliases.join(", ")
        );

        if !self.knowledge_points.is_empty() {
            text.push_str("\nContext:");
            for point in &self.knowledge_points {
                text.push_str("\n- ");
                text.push_str(point);
            }
        }

        text
    }
}

/// Builds one embedding context for every entity in a consolidated graph.
///
/// Knowledge points are selected by stable entity ID, retain graph order, and
/// are deduplicated by knowledge-point ID. The returned contexts retain entity
/// order so repeated runs over the same graph produce identical embedding
/// batches.
///
/// ```text
/// Entity: CO₂
/// Aliases: CO₂, CO2
/// Context:
/// - CO₂ is attached to RuBP by RuBisCO.
/// - Carbon fixation incorporates CO₂.
/// ```
pub fn build_entity_contexts(
    graph: &PropositionGraph,
    max_points_per_entity: usize,
) -> Vec<EntityContext> {
    let mut contexts = graph
        .entities
        .iter()
        .map(|entity| EntityContext {
            entity_id: entity.id.clone(),
            canonical_name: entity.canonical_name.clone(),
            aliases: entity.aliases.clone(),
            knowledge_points: Vec::new(),
        })
        .collect::<Vec<_>>();

    if max_points_per_entity == 0 || contexts.is_empty() {
        return contexts;
    }

    let context_index = contexts
        .iter()
        .enumerate()
        .map(|(index, context)| (context.entity_id.clone(), index))
        .collect::<HashMap<_, _>>();
    let mut seen_point_ids = vec![HashSet::new(); contexts.len()];

    for point in &graph.knowledge_points {
        for entity_id in &point.entity_ids {
            let Some(&index) = context_index.get(entity_id.as_str()) else {
                continue;
            };

            if contexts[index].knowledge_points.len() >= max_points_per_entity {
                continue;
            }

            if seen_point_ids[index].insert(point.id.as_str()) {
                contexts[index].knowledge_points.push(point.point.clone());
            }
        }
    }

    contexts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::graph_generation::types::{
        EntityNode, KnowledgePoint, KnowledgeType, PropositionGraph,
    };

    fn entity(id: &str, name: &str, aliases: &[&str]) -> EntityNode {
        EntityNode {
            id: id.to_string(),
            canonical_name: name.to_string(),
            aliases: aliases.iter().map(|alias| alias.to_string()).collect(),
            chunk_ids: Vec::new(),
        }
    }

    fn point(id: &str, text: &str, entity_ids: &[&str]) -> KnowledgePoint {
        KnowledgePoint {
            id: id.to_string(),
            point: text.to_string(),
            knowledge_type: KnowledgeType::Fact,
            chunk_id: String::from("chunk-1"),
            raw_entity_names: Vec::new(),
            entity_ids: entity_ids.iter().map(|id| id.to_string()).collect(),
            raw_relations: Vec::new(),
        }
    }

    fn graph(entities: Vec<EntityNode>, knowledge_points: Vec<KnowledgePoint>) -> PropositionGraph {
        PropositionGraph {
            entities,
            knowledge_points,
            relations: Vec::new(),
        }
    }

    #[test]
    fn selects_only_points_that_reference_the_entity() {
        let graph = graph(
            vec![
                entity("entity-co2", "CO₂", &["CO₂", "CO2"]),
                entity("entity-oxygen", "oxygen", &["oxygen"]),
            ],
            vec![
                point(
                    "kp-1",
                    "CO₂ is attached to RuBP by RuBisCO.",
                    &["entity-co2"],
                ),
                point(
                    "kp-2",
                    "Oxygen is released when water is split.",
                    &["entity-oxygen"],
                ),
            ],
        );

        let contexts = build_entity_contexts(&graph, 3);

        assert_eq!(contexts.len(), 2);
        assert_eq!(
            contexts[0].knowledge_points,
            vec!["CO₂ is attached to RuBP by RuBisCO."]
        );
        assert_eq!(
            contexts[1].knowledge_points,
            vec!["Oxygen is released when water is split."]
        );
    }

    #[test]
    fn preserves_entity_aliases_and_entity_order() {
        let graph = graph(
            vec![
                entity("entity-co2", "CO₂", &["CO₂", "CO2"]),
                entity("entity-g3p", "G3P", &["G3P"]),
            ],
            Vec::new(),
        );

        let contexts = build_entity_contexts(&graph, 3);

        assert_eq!(contexts[0].entity_id, "entity-co2");
        assert_eq!(contexts[0].aliases, vec!["CO₂", "CO2"]);
        assert_eq!(contexts[1].entity_id, "entity-g3p");
    }

    #[test]
    fn limits_points_in_graph_order() {
        let graph = graph(
            vec![entity("entity-co2", "CO₂", &["CO₂"])],
            vec![
                point("kp-1", "First fact.", &["entity-co2"]),
                point("kp-2", "Second fact.", &["entity-co2"]),
                point("kp-3", "Third fact.", &["entity-co2"]),
            ],
        );

        let contexts = build_entity_contexts(&graph, 2);

        assert_eq!(
            contexts[0].knowledge_points,
            vec!["First fact.", "Second fact."]
        );
    }

    #[test]
    fn deduplicates_repeated_point_ids_and_entity_references() {
        let graph = graph(
            vec![entity("entity-co2", "CO₂", &["CO₂"])],
            vec![
                point(
                    "kp-1",
                    "CO₂ participates in carbon fixation.",
                    &["entity-co2", "entity-co2"],
                ),
                point(
                    "kp-1",
                    "Duplicate extraction of the same point.",
                    &["entity-co2"],
                ),
            ],
        );

        let contexts = build_entity_contexts(&graph, 3);

        assert_eq!(
            contexts[0].knowledge_points,
            vec!["CO₂ participates in carbon fixation."]
        );
    }

    #[test]
    fn keeps_entities_without_context_and_supports_a_zero_limit() {
        let graph = graph(
            vec![entity("entity-co2", "CO₂", &["CO₂", "CO2"])],
            vec![point("kp-1", "A fact about CO₂.", &["entity-co2"])],
        );

        let contexts = build_entity_contexts(&graph, 0);

        assert_eq!(contexts.len(), 1);
        assert!(contexts[0].knowledge_points.is_empty());
        assert_eq!(
            contexts[0].embedding_text(),
            "Entity: CO₂\nAliases: CO₂, CO2"
        );
    }

    #[test]
    fn formats_embedding_text_deterministically() {
        let context = EntityContext {
            entity_id: String::from("entity-co2"),
            canonical_name: String::from("CO₂"),
            aliases: vec![String::from("CO₂"), String::from("CO2")],
            knowledge_points: vec![
                String::from("CO₂ is attached to RuBP by RuBisCO."),
                String::from("Carbon fixation incorporates CO₂."),
            ],
        };

        assert_eq!(
            context.embedding_text(),
            "Entity: CO₂\nAliases: CO₂, CO2\nContext:\n- CO₂ is attached to RuBP by RuBisCO.\n- Carbon fixation incorporates CO₂."
        );
    }
}
