//! JSON schema and validation for Stage A graph-based extraction output.
//!
//! Separates wire format (what LLM returns) from business format (what we store).
//! Wire types are deserialized from JSON; then hydrated into business types with
//! injected metadata (chunk_id, generated IDs, etc.).

use std::collections::HashSet;
use std::error::Error;
use std::fmt;

use serde::Deserialize;
use serde_json::{json, Value};

use super::types::{
    ExtractedKnowledge, KnowledgePoint, KnowledgeType, RawEntityMention, RelationRef,
    RelationType,
};

// =====================================================================
// 1. Error Types
// =====================================================================

/// Top-level error for Stage A parsing and validation.
/// Distinguishes parse failures (malformed JSON) from validation failures
/// (well-formed JSON, bad content) so callers can handle recovery differently.
#[derive(Debug)]
pub enum GraphStageAError {
    /// JSON deserialization failed — malformed input.
    Parse(serde_json::Error),
    /// JSON is valid but content is semantically invalid.
    Validation(GraphStageAValidationError),
}

/// Semantic validation errors — the JSON parses, but content is wrong.
#[derive(Debug)]
pub enum GraphStageAValidationError {
    /// Root element is not a JSON object.
    MalformedShape,
    /// Required field is missing entirely (not just empty).
    MissingField { field: &'static str },
    /// Point text is empty or whitespace-only.
    EmptyPoint { index: usize },
    /// Entity name is empty or whitespace-only.
    EmptyEntityName { entity_index: usize },
    /// raw_entity_name is empty or whitespace-only.
    EmptyRawEntityName { point_index: usize, entity_index: usize },
    /// raw_entity_names contains duplicate names (case-insensitive).
    DuplicateRawEntityNames { point_index: usize },
    /// raw_entity_names array is empty (every point must be about something).
    NoRawEntityNames { point_index: usize },
    /// knowledge_type value is not one of the four valid types.
    InvalidKnowledgeType { point_index: usize, value: String },
    /// relation_type value is not one of the six valid types.
    InvalidRelationType {
        point_index: usize,
        relation_index: usize,
        value: String,
    },
    /// relation target_entity_name is empty or whitespace-only.
    EmptyRelationTarget {
        point_index: usize,
        relation_index: usize,
    },
}

impl fmt::Display for GraphStageAError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(err) => write!(f, "Invalid LLM JSON payload: {err}"),
            Self::Validation(err) => write!(f, "LLM JSON validation failed: {err}"),
        }
    }
}

impl Error for GraphStageAError {}

impl fmt::Display for GraphStageAValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedShape => write!(f, "Root element must be a JSON object"),
            Self::MissingField { field } => write!(f, "Required field '{field}' is missing"),
            Self::EmptyPoint { index } => {
                write!(f, "knowledge_point at index {index} must be non-empty")
            }
            Self::EmptyEntityName { entity_index } => {
                write!(f, "entity at index {entity_index} must be non-empty")
            }
            Self::EmptyRawEntityName { point_index, entity_index } => {
                write!(
                    f,
                    "raw_entity_name at point {point_index}, entity {entity_index} must be non-empty"
                )
            }
            Self::DuplicateRawEntityNames { point_index } => {
                write!(
                    f,
                    "raw_entity_names at point {point_index} contains duplicate names (case-insensitive)"
                )
            }
            Self::NoRawEntityNames { point_index } => {
                write!(
                    f,
                    "knowledge_point at index {point_index} must be about at least one entity"
                )
            }
            Self::InvalidKnowledgeType { point_index, value } => {
                write!(
                    f,
                    "knowledge_type at point {point_index} has invalid value '{value}' \
                     (expected: definition, fact, procedural, conceptual)"
                )
            }
            Self::InvalidRelationType {
                point_index,
                relation_index,
                value,
            } => {
                write!(
                    f,
                    "relation_type at point {point_index}, relation {relation_index} has invalid value '{value}' \
                     (expected: related_to, contrasts, prerequisite, consequence, example, counter_example)"
                )
            }
            Self::EmptyRelationTarget {
                point_index,
                relation_index,
            } => {
                write!(
                    f,
                    "target_entity_name at point {point_index}, relation {relation_index} must be non-empty"
                )
            }
        }
    }
}

impl Error for GraphStageAValidationError {}

// =====================================================================
// 2. Wire/DTO Types — what the LLM returns
// =====================================================================

/// Wire format for a single relation as returned by LLM.
/// Mirrors exactly what's in the JSON; no business logic.
#[derive(Debug, Clone, Deserialize)]
struct WireRelationRef {
    pub target_entity_name: String,
    pub relation_type: String, // Validated as enum during semantic checks
    #[serde(default)]
    pub source_quote: Option<String>,
}

/// Wire format for a single knowledge point as returned by LLM.
/// Deliberately excludes id, chunk_id, entity_ids — those are injected during hydration.
#[derive(Debug, Clone, Deserialize)]
struct WireKnowledgePoint {
    pub point: String,
    pub knowledge_type: String, // Validated as enum during semantic checks
    pub raw_entity_names: Vec<String>,
    #[serde(default)]
    pub raw_relations: Vec<WireRelationRef>,
}

/// Wire format for full Stage A output as returned by LLM.
/// Mirrors the JSON structure; hydrated into ExtractedKnowledge with metadata injection.
#[derive(Debug, Clone, Deserialize)]
struct WireExtractedKnowledge {
    pub entities: Vec<String>,
    pub knowledge_points: Vec<WireKnowledgePoint>,
}

// =====================================================================
// 3. JSON Schema Generator
// =====================================================================

/// Returns the JSON schema that Stage A output must conform to.
/// Used for LLM schema enforcement (Ollama `format` field, OpenRouter `response_format`).
///
/// # Schema Sync Requirements
///
/// KEEP IN SYNC with:
/// - `types.rs` — `KnowledgeType` enum: definition, fact, procedural, conceptual
/// - `types.rs` — `RelationType` enum: related_to, contrasts, prerequisite, consequence, example, counter_example
/// - `stage_a_prompt.rs` — descriptions of the four knowledge types and six relation types
///
/// If any of these change, update all four locations simultaneously to prevent schema/prompt drift.
pub fn stage_a_format_schema() -> Value {
    // Note: Moved before validate_stage_a_json() to maintain logical flow (schema definition → validation → parsing)
    json!({
        "type": "object",
        "properties": {
            "entities": {
                "type": "array",
                "items": {
                    "type": "string"
                },
                "description": "All entities mentioned in this chunk"
            },
            "knowledge_points": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "point": {
                            "type": "string",
                            "description": "A testable proposition or claim"
                        },
                        "knowledge_type": {
                            "type": "string",
                            "enum": ["definition", "fact", "procedural", "conceptual"],
                            "description": "Category of this knowledge point (sync with types.rs KnowledgeType)"
                        },
                        "raw_entity_names": {
                            "type": "array",
                            "items": {
                                "type": "string"
                            },
                            "description": "All entities this point is about (must be non-empty)"
                        },
                        "raw_relations": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "target_entity_name": {
                                        "type": "string",
                                        "description": "Entity this point relates to"
                                    },
                                    "relation_type": {
                                        "type": "string",
                                        "enum": [
                                            "related_to",
                                            "contrasts",
                                            "prerequisite",
                                            "consequence",
                                            "example",
                                            "counter_example"
                                        ],
                                        "description": "Type of relationship (sync with types.rs RelationType)"
                                    },
                                    "source_quote": {
                                        "type": ["string", "null"],
                                        "description": "Optional text span supporting this relation"
                                    }
                                },
                                "required": ["target_entity_name", "relation_type"]
                            },
                            "description": "Relations to other entities"
                        }
                    },
                    "required": ["point", "knowledge_type", "raw_entity_names", "raw_relations"]
                }
            }
        },
        "required": ["entities", "knowledge_points"]
    })
}

// =====================================================================
// 4. Semantic Validation
// =====================================================================

/// Validates Stage A JSON output semantically.
/// Runs against raw `serde_json::Value` to give indexed error messages before typed deserialization.
/// Checks: non-empty texts, valid enum values, no duplicates, required collections non-empty.
fn validate_stage_a_json(value: &Value) -> Result<(), GraphStageAError> {

    // Check root is an object
    let obj = value.as_object().ok_or(GraphStageAError::Validation(
        GraphStageAValidationError::MalformedShape,
    ))?;

    // Check entities array exists and is non-empty
    let entities = obj.get("entities").ok_or(GraphStageAError::Validation(
        GraphStageAValidationError::MissingField { field: "entities" },
    ))?;
    let entities = entities.as_array().ok_or(GraphStageAError::Validation(
        GraphStageAValidationError::MalformedShape,
    ))?;

    for (idx, entity) in entities.iter().enumerate() {
        let name = entity
            .as_str()
            .ok_or(GraphStageAError::Validation(
                GraphStageAValidationError::MalformedShape,
            ))?
            .trim();
        if name.is_empty() {
            return Err(GraphStageAError::Validation(
                GraphStageAValidationError::EmptyEntityName { entity_index: idx },
            ));
        }
    }

    // Check knowledge_points array exists and is non-empty
    let points = obj.get("knowledge_points").ok_or(GraphStageAError::Validation(
        GraphStageAValidationError::MissingField {
            field: "knowledge_points",
        },
    ))?;
    let points = points.as_array().ok_or(GraphStageAError::Validation(
        GraphStageAValidationError::MalformedShape,
    ))?;

    for (point_idx, point_val) in points.iter().enumerate() {
        let point_obj = point_val.as_object().ok_or(GraphStageAError::Validation(
            GraphStageAValidationError::MalformedShape,
        ))?;

        // Validate point text
        let point_text = point_obj
            .get("point")
            .ok_or(GraphStageAError::Validation(
                GraphStageAValidationError::MissingField { field: "point" },
            ))?
            .as_str()
            .ok_or(GraphStageAError::Validation(
                GraphStageAValidationError::MalformedShape,
            ))?;
        if point_text.trim().is_empty() {
            return Err(GraphStageAError::Validation(
                GraphStageAValidationError::EmptyPoint { index: point_idx },
            ));
        }

        // Validate knowledge_type
        let kt_str = point_obj
            .get("knowledge_type")
            .ok_or(GraphStageAError::Validation(
                GraphStageAValidationError::MissingField {
                    field: "knowledge_type",
                },
            ))?
            .as_str()
            .ok_or(GraphStageAError::Validation(
                GraphStageAValidationError::MalformedShape,
            ))?;
        match kt_str {
            "definition" | "fact" | "procedural" | "conceptual" => {}
            _ => {
                return Err(GraphStageAError::Validation(
                    GraphStageAValidationError::InvalidKnowledgeType {
                        point_index: point_idx,
                        value: kt_str.to_string(),
                    },
                ))
            }
        }

        // Validate raw_entity_names
        let raw_entity_names = point_obj
            .get("raw_entity_names")
            .ok_or(GraphStageAError::Validation(
                GraphStageAValidationError::MissingField {
                    field: "raw_entity_names",
                },
            ))?
            .as_array()
            .ok_or(GraphStageAError::Validation(
                GraphStageAValidationError::MalformedShape,
            ))?;

        if raw_entity_names.is_empty() {
            return Err(GraphStageAError::Validation(
                GraphStageAValidationError::NoRawEntityNames { point_index: point_idx },
            ));
        }

        let mut seen_entity_names = HashSet::new();
        for (entity_idx, entity_val) in raw_entity_names.iter().enumerate() {
            let entity_name = entity_val
                .as_str()
                .ok_or(GraphStageAError::Validation(
                    GraphStageAValidationError::MalformedShape,
                ))?
                .trim();
            if entity_name.is_empty() {
                return Err(GraphStageAError::Validation(
                    GraphStageAValidationError::EmptyRawEntityName {
                        point_index: point_idx,
                        entity_index: entity_idx,
                    },
                ));
            }
            // Case-insensitive duplicate check
            if !seen_entity_names.insert(entity_name.to_lowercase()) {
                return Err(GraphStageAError::Validation(
                    GraphStageAValidationError::DuplicateRawEntityNames { point_index: point_idx },
                ));
            }
        }

        // Validate raw_relations (defaults to empty if missing)
        let raw_relations_opt = point_obj.get("raw_relations");
        let raw_relations_array = raw_relations_opt
            .map(|v| v.as_array())
            .flatten();
        let empty_vec = Vec::new();
        let raw_relations = raw_relations_array.unwrap_or(&empty_vec);

        for (rel_idx, rel_val) in raw_relations.iter().enumerate() {
            let rel_obj = rel_val.as_object().ok_or(GraphStageAError::Validation(
                GraphStageAValidationError::MalformedShape,
            ))?;

            // Validate target_entity_name
            let target = rel_obj
                .get("target_entity_name")
                .ok_or(GraphStageAError::Validation(
                    GraphStageAValidationError::MissingField {
                        field: "target_entity_name",
                    },
                ))?
                .as_str()
                .ok_or(GraphStageAError::Validation(
                    GraphStageAValidationError::MalformedShape,
                ))?
                .trim();
            if target.is_empty() {
                return Err(GraphStageAError::Validation(
                    GraphStageAValidationError::EmptyRelationTarget {
                        point_index: point_idx,
                        relation_index: rel_idx,
                    },
                ));
            }

            // Validate relation_type
            let rt_str = rel_obj
                .get("relation_type")
                .ok_or(GraphStageAError::Validation(
                    GraphStageAValidationError::MissingField {
                        field: "relation_type",
                    },
                ))?
                .as_str()
                .ok_or(GraphStageAError::Validation(
                    GraphStageAValidationError::MalformedShape,
                ))?;
            match rt_str {
                "related_to" | "contrasts" | "prerequisite" | "consequence" | "example"
                | "counter_example" => {}
                _ => {
                    return Err(GraphStageAError::Validation(
                        GraphStageAValidationError::InvalidRelationType {
                            point_index: point_idx,
                            relation_index: rel_idx,
                            value: rt_str.to_string(),
                        },
                    ))
                }
            }
        }
    }

    Ok(())
}

// =====================================================================
// 5. Parsing Pipeline (3 stages: parse → validate → hydrate)
// =====================================================================

/// Parses and validates Stage A LLM output.
///
/// Three-stage pipeline:
/// 1. Parse JSON into raw Value (serde_json::Error on malformed input)
/// 2. Validate semantically (indexed validation errors on bad content)
/// 3. Deserialize to wire types, then hydrate to business types
///    - Injects chunk_id into every KnowledgePoint
///    - Generates stable IDs (`{chunk_id}-kp-{index}`)
///    - Converts strings to enums
///
/// # Arguments
/// - `json_payload` — JSON string from LLM
/// - `chunk_id` — The chunk this extraction came from (injected into all points)
///
/// # Returns
/// Fully hydrated `ExtractedKnowledge` ready for Pass 3 consolidation.
pub fn parse_stage_a_output(
    json_payload: &str,
    chunk_id: String,
) -> Result<ExtractedKnowledge, GraphStageAError> {
    // Stage 1: Parse JSON
    let value: Value = serde_json::from_str(json_payload).map_err(GraphStageAError::Parse)?;

    // Stage 2: Validate semantically
    validate_stage_a_json(&value)?;

    // Stage 3: Deserialize to wire types, then hydrate
    let wire: WireExtractedKnowledge =
        serde_json::from_value(value).map_err(GraphStageAError::Parse)?;

    // Build the entity set from the declared list, then auto-union any names
    // referenced in knowledge_points that the LLM forgot to declare upfront.
    // This makes hydration self-healing: a single missing entity name does not
    // discard an entire chunk's worth of knowledge points.
    let mut known_names: HashSet<String> = wire
        .entities
        .iter()
        .map(|n| n.trim().to_lowercase())
        .collect();
    let mut all_entity_names: Vec<String> = wire.entities.clone();
    for kp in &wire.knowledge_points {
        for name in &kp.raw_entity_names {
            let key = name.trim().to_lowercase();
            if !known_names.contains(&key) {
                known_names.insert(key);
                all_entity_names.push(name.trim().to_string());
            }
        }
        for rel in &kp.raw_relations {
            let key = rel.target_entity_name.trim().to_lowercase();
            if !known_names.contains(&key) {
                known_names.insert(key);
                all_entity_names.push(rel.target_entity_name.trim().to_string());
            }
        }
    }

    let raw_entities: Vec<RawEntityMention> = all_entity_names
        .into_iter()
        .map(|name| RawEntityMention {
            name,
            chunk_id: chunk_id.clone(),
        })
        .collect();

    // Hydrate knowledge points: generate IDs, convert strings to enums
    let knowledge_points = wire
        .knowledge_points
        .into_iter()
        .enumerate()
        .map(|(index, wire_point)| {
            let id = format!("{}-kp-{}", chunk_id, index);
            let knowledge_type = match wire_point.knowledge_type.as_str() {
                "definition" => KnowledgeType::Definition,
                "fact" => KnowledgeType::Fact,
                "procedural" => KnowledgeType::Procedural,
                "conceptual" => KnowledgeType::Conceptual,
                _ => unreachable!("validate_stage_a_json guarantees a valid knowledge_type"),
            };

            let raw_relations = wire_point
                .raw_relations
                .into_iter()
                .map(|wire_rel| {
                    let relation_type = match wire_rel.relation_type.as_str() {
                        "related_to" => RelationType::RelatedTo,
                        "contrasts" => RelationType::Contrasts,
                        "prerequisite" => RelationType::Prerequisite,
                        "consequence" => RelationType::Consequence,
                        "example" => RelationType::Example,
                        "counter_example" => RelationType::CounterExample,
                        _ => unreachable!("validate_stage_a_json guarantees a valid relation_type"),
                    };
                    RelationRef {
                        target_entity_name: wire_rel.target_entity_name,
                        relation_type,
                        source_quote: wire_rel.source_quote,
                    }
                })
                .collect();

            KnowledgePoint {
                id,
                point: wire_point.point,
                knowledge_type,
                chunk_id: chunk_id.clone(),
                raw_entity_names: wire_point.raw_entity_names,
                entity_ids: Vec::new(), // Populated by Pass 3
                raw_relations,
            }
        })
        .collect();

    Ok(ExtractedKnowledge {
        chunk_id,
        raw_entities,
        knowledge_points,
    })
}

// =====================================================================
// 6. Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_stage_a_output() {
        let json = r#"{
            "entities": ["chloroplast", "chlorophyll"],
            "knowledge_points": [
                {
                    "point": "Chloroplasts contain chlorophyll pigment",
                    "knowledge_type": "fact",
                    "raw_entity_names": ["chloroplast", "chlorophyll"],
                    "raw_relations": [
                        {
                            "target_entity_name": "chlorophyll",
                            "relation_type": "related_to",
                            "source_quote": null
                        }
                    ]
                }
            ]
        }"#;

        let result = parse_stage_a_output(json, "chunk_1".to_string());
        assert!(result.is_ok());

        let extracted = result.unwrap();
        assert_eq!(extracted.chunk_id, "chunk_1");
        assert_eq!(extracted.raw_entities.len(), 2);
        assert_eq!(extracted.knowledge_points.len(), 1);
        let point = &extracted.knowledge_points[0];
        assert_eq!(point.id, "chunk_1-kp-0");
        assert_eq!(point.chunk_id, "chunk_1");
        assert!(point.entity_ids.is_empty());
    }

    #[test]
    fn test_empty_point_rejected() {
        let json = r#"{
            "entities": ["entity1"],
            "knowledge_points": [
                {
                    "point": "",
                    "knowledge_type": "fact",
                    "raw_entity_names": ["entity1"],
                    "raw_relations": []
                }
            ]
        }"#;

        let result = parse_stage_a_output(json, "chunk_1".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_knowledge_type_rejected() {
        let json = r#"{
            "entities": ["entity1"],
            "knowledge_points": [
                {
                    "point": "A fact",
                    "knowledge_type": "invalid_type",
                    "raw_entity_names": ["entity1"],
                    "raw_relations": []
                }
            ]
        }"#;

        let result = parse_stage_a_output(json, "chunk_1".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_raw_entity_names_rejected() {
        let json = r#"{
            "entities": ["entity1"],
            "knowledge_points": [
                {
                    "point": "A fact",
                    "knowledge_type": "fact",
                    "raw_entity_names": [],
                    "raw_relations": []
                }
            ]
        }"#;

        let result = parse_stage_a_output(json, "chunk_1".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_duplicate_raw_entity_names_rejected() {
        let json = r#"{
            "entities": ["entity1"],
            "knowledge_points": [
                {
                    "point": "A fact",
                    "knowledge_type": "fact",
                    "raw_entity_names": ["entity1", "ENTITY1"],
                    "raw_relations": []
                }
            ]
        }"#;

        let result = parse_stage_a_output(json, "chunk_1".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_relation_type_rejected() {
        let json = r#"{
            "entities": ["entity1", "entity2"],
            "knowledge_points": [
                {
                    "point": "A fact",
                    "knowledge_type": "fact",
                    "raw_entity_names": ["entity1"],
                    "raw_relations": [
                        {
                            "target_entity_name": "entity2",
                            "relation_type": "invalid_relation",
                            "source_quote": null
                        }
                    ]
                }
            ]
        }"#;

        let result = parse_stage_a_output(json, "chunk_1".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_hydration_round_trip() {
        let json = r#"{
            "entities": ["chloroplast", "thylakoid"],
            "knowledge_points": [
                {
                    "point": "Thylakoids are membranes in chloroplasts",
                    "knowledge_type": "definition",
                    "raw_entity_names": ["thylakoid", "chloroplast"],
                    "raw_relations": []
                },
                {
                    "point": "Light reactions occur in thylakoids",
                    "knowledge_type": "fact",
                    "raw_entity_names": ["thylakoid"],
                    "raw_relations": [
                        {
                            "target_entity_name": "chloroplast",
                            "relation_type": "prerequisite",
                            "source_quote": "Thylakoids are inside chloroplasts"
                        }
                    ]
                }
            ]
        }"#;

        let result = parse_stage_a_output(json, "photosynthesis_chunk".to_string());
        assert!(result.is_ok());

        let extracted = result.unwrap();
        assert_eq!(extracted.chunk_id, "photosynthesis_chunk");
        assert_eq!(extracted.knowledge_points.len(), 2);

        // Check first point
        let p0 = &extracted.knowledge_points[0];
        assert_eq!(p0.id, "photosynthesis_chunk-kp-0");
        assert_eq!(p0.chunk_id, "photosynthesis_chunk");
        assert!(p0.entity_ids.is_empty());
        assert_eq!(p0.raw_entity_names.len(), 2);

        // Check second point
        let p1 = &extracted.knowledge_points[1];
        assert_eq!(p1.id, "photosynthesis_chunk-kp-1");
        assert_eq!(p1.chunk_id, "photosynthesis_chunk");
        assert_eq!(p1.raw_relations.len(), 1);
        assert_eq!(p1.raw_relations[0].target_entity_name, "chloroplast");
    }

    #[test]
    fn test_empty_relations_array_works() {
        let json = r#"{
            "entities": ["entity1"],
            "knowledge_points": [
                {
                    "point": "A fact",
                    "knowledge_type": "fact",
                    "raw_entity_names": ["entity1"],
                    "raw_relations": []
                }
            ]
        }"#;

        let result = parse_stage_a_output(json, "chunk_1".to_string());
        assert!(result.is_ok());
        let extracted = result.unwrap();
        assert_eq!(extracted.knowledge_points[0].raw_relations.len(), 0);
    }

    #[test]
    fn test_missing_relations_field_defaults_to_empty() {
        let json = r#"{
            "entities": ["entity1"],
            "knowledge_points": [
                {
                    "point": "A fact",
                    "knowledge_type": "fact",
                    "raw_entity_names": ["entity1"]
                }
            ]
        }"#;

        let result = parse_stage_a_output(json, "chunk_1".to_string());
        assert!(result.is_ok());
        let extracted = result.unwrap();
        assert_eq!(extracted.knowledge_points[0].raw_relations.len(), 0);
    }

    #[test]
    fn test_missing_source_quote_in_relation_defaults_to_none() {
        let json = r#"{
            "entities": ["entity1", "entity2"],
            "knowledge_points": [
                {
                    "point": "A fact",
                    "knowledge_type": "fact",
                    "raw_entity_names": ["entity1"],
                    "raw_relations": [
                        {
                            "target_entity_name": "entity2",
                            "relation_type": "related_to"
                        }
                    ]
                }
            ]
        }"#;

        let result = parse_stage_a_output(json, "chunk_1".to_string());
        assert!(result.is_ok());
        let extracted = result.unwrap();
        assert_eq!(extracted.knowledge_points[0].raw_relations.len(), 1);
        assert_eq!(
            extracted.knowledge_points[0].raw_relations[0].target_entity_name,
            "entity2"
        );
        assert!(
            extracted.knowledge_points[0].raw_relations[0].source_quote.is_none(),
            "source_quote should default to None when key is omitted"
        );
    }

    #[test]
    fn test_malformed_root_rejected() {
        let json = r#"[]"#;
        let result = parse_stage_a_output(json, "chunk_1".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_undeclared_raw_entity_auto_unioned() {
        // raw_entity_names references "thylakoid" but entities list only has "chloroplast".
        // Hydration should auto-union "thylakoid" rather than reject the chunk.
        let json = r#"{
            "entities": ["chloroplast"],
            "knowledge_points": [
                {
                    "point": "Thylakoids are membranes inside chloroplasts",
                    "knowledge_type": "fact",
                    "raw_entity_names": ["thylakoid", "chloroplast"],
                    "raw_relations": []
                }
            ]
        }"#;

        let result = parse_stage_a_output(json, "chunk_1".to_string());
        assert!(result.is_ok(), "should succeed via auto-union, got: {:?}", result);
        let extracted = result.unwrap();
        let entity_names: Vec<&str> = extracted.raw_entities.iter().map(|e| e.name.as_str()).collect();
        assert!(entity_names.contains(&"thylakoid"), "auto-unioned entity should appear in raw_entities");
        assert!(entity_names.contains(&"chloroplast"));
        assert_eq!(extracted.knowledge_points[0].raw_entity_names, vec!["thylakoid", "chloroplast"]);
    }

    #[test]
    fn test_undeclared_relation_target_auto_unioned() {
        // relation target "membrane" is not in the entities list.
        // Hydration should auto-union "membrane" rather than reject the chunk.
        let json = r#"{
            "entities": ["chloroplast"],
            "knowledge_points": [
                {
                    "point": "Chloroplasts contain membranes",
                    "knowledge_type": "fact",
                    "raw_entity_names": ["chloroplast"],
                    "raw_relations": [
                        {
                            "target_entity_name": "membrane",
                            "relation_type": "related_to",
                            "source_quote": null
                        }
                    ]
                }
            ]
        }"#;

        let result = parse_stage_a_output(json, "chunk_1".to_string());
        assert!(result.is_ok(), "should succeed via auto-union, got: {:?}", result);
        let extracted = result.unwrap();
        let entity_names: Vec<&str> = extracted.raw_entities.iter().map(|e| e.name.as_str()).collect();
        assert!(entity_names.contains(&"membrane"), "auto-unioned relation target should appear in raw_entities");
        assert!(entity_names.contains(&"chloroplast"));
    }
}
