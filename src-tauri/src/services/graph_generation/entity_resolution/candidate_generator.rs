//! Embedding-similarity candidate generation for semantic entity resolution.
//!
//! Similarity only decides which entity pairs deserve semantic verification.
//! It never authorizes a merge by itself.

use std::collections::HashSet;
use std::error::Error;
use std::fmt;

use super::embedding_generator::EntityEmbedding;

/// One directed candidate selected for an entity's verification shortlist.
///
/// The pair is semantically unordered. Direction records which entity's
/// top-K search selected it; reverse duplicates are omitted.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityCandidate {
    pub entity_id: String,
    pub candidate_entity_id: String,
    pub similarity: f32,
}

/// Loose retrieval settings used before semantic verification.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CandidateConfig {
    pub minimum_similarity: f32,
    pub max_candidates_per_entity: usize,
}

impl CandidateConfig {
    /// Validates retrieval settings without generating candidates.
    ///
    /// The orchestration layer calls this before contacting an embedding
    /// provider so invalid local configuration cannot incur a remote request.
    pub fn validate(&self) -> Result<(), CandidateGenerationError> {
        validate_config(self)
    }
}

/// Invalid configuration or vector data encountered during candidate search.
#[derive(Debug, Clone, PartialEq)]
pub enum CandidateGenerationError {
    InvalidMinimumSimilarity {
        value: f32,
    },
    DuplicateEntityId {
        entity_id: String,
    },
    DimensionMismatch {
        entity_id: String,
        expected: usize,
        actual: usize,
    },
    ZeroMagnitudeVector {
        entity_id: String,
    },
}

impl fmt::Display for CandidateGenerationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMinimumSimilarity { value } => write!(
                formatter,
                "Candidate minimum similarity must be finite and between -1 and 1; received {value}"
            ),
            Self::DuplicateEntityId { entity_id } => {
                write!(
                    formatter,
                    "Candidate input contains duplicate entity ID '{entity_id}'"
                )
            }
            Self::DimensionMismatch {
                entity_id,
                expected,
                actual,
            } => write!(
                formatter,
                "Embedding for entity '{entity_id}' has {actual} dimensions; expected {expected}"
            ),
            Self::ZeroMagnitudeVector { entity_id } => write!(
                formatter,
                "Embedding for entity '{entity_id}' has zero magnitude and cannot be compared"
            ),
        }
    }
}

impl Error for CandidateGenerationError {}

/// Produces a deterministic, loose shortlist for later semantic verification.
///
/// Each entity independently selects at most `max_candidates_per_entity`
/// neighbours at or above `minimum_similarity`. Source entities and tied
/// candidates are ordered by stable entity ID. Once either direction of a pair
/// has been retained, the reverse direction is omitted so the verifier sees an
/// unordered entity pair only once. The final list is ordered by descending
/// similarity, then by its two stable IDs.
pub fn generate_entity_candidates(
    embeddings: &[EntityEmbedding],
    config: &CandidateConfig,
) -> Result<Vec<EntityCandidate>, CandidateGenerationError> {
    validate_config(config)?;

    if embeddings.is_empty() {
        return Ok(Vec::new());
    }

    let mut entity_indices = (0..embeddings.len()).collect::<Vec<_>>();
    entity_indices.sort_by(|left, right| {
        embeddings[*left]
            .entity_id
            .cmp(&embeddings[*right].entity_id)
    });

    let expected_dimensions = embeddings[entity_indices[0]].vector.dimensions();
    let mut seen_entity_ids = HashSet::with_capacity(embeddings.len());
    let mut magnitudes = vec![0.0; embeddings.len()];

    for &entity_index in &entity_indices {
        let embedding = &embeddings[entity_index];
        if !seen_entity_ids.insert(embedding.entity_id.as_str()) {
            return Err(CandidateGenerationError::DuplicateEntityId {
                entity_id: embedding.entity_id.clone(),
            });
        }

        let actual_dimensions = embedding.vector.dimensions();
        if actual_dimensions != expected_dimensions {
            return Err(CandidateGenerationError::DimensionMismatch {
                entity_id: embedding.entity_id.clone(),
                expected: expected_dimensions,
                actual: actual_dimensions,
            });
        }

        let squared_magnitude = embedding
            .vector
            .values()
            .iter()
            .map(|value| f64::from(*value).powi(2))
            .sum::<f64>();
        if squared_magnitude == 0.0 {
            return Err(CandidateGenerationError::ZeroMagnitudeVector {
                entity_id: embedding.entity_id.clone(),
            });
        }
        magnitudes[entity_index] = squared_magnitude.sqrt();
    }

    if config.max_candidates_per_entity == 0 || embeddings.len() < 2 {
        return Ok(Vec::new());
    }

    let mut seen_pairs = HashSet::new();
    let mut candidates = Vec::new();

    for &entity_index in &entity_indices {
        let mut entity_candidates = entity_indices
            .iter()
            .copied()
            .filter(|candidate_index| *candidate_index != entity_index)
            .filter_map(|candidate_index| {
                let similarity = cosine_similarity(
                    &embeddings[entity_index],
                    magnitudes[entity_index],
                    &embeddings[candidate_index],
                    magnitudes[candidate_index],
                );

                (similarity >= config.minimum_similarity).then_some((candidate_index, similarity))
            })
            .collect::<Vec<_>>();

        entity_candidates.sort_by(
            |(left_index, left_similarity), (right_index, right_similarity)| {
                right_similarity.total_cmp(left_similarity).then_with(|| {
                    embeddings[*left_index]
                        .entity_id
                        .cmp(&embeddings[*right_index].entity_id)
                })
            },
        );
        entity_candidates.truncate(config.max_candidates_per_entity);

        for (candidate_index, similarity) in entity_candidates {
            let entity_id = &embeddings[entity_index].entity_id;
            let candidate_entity_id = &embeddings[candidate_index].entity_id;
            let pair_key = if entity_id < candidate_entity_id {
                (entity_id.clone(), candidate_entity_id.clone())
            } else {
                (candidate_entity_id.clone(), entity_id.clone())
            };

            if seen_pairs.insert(pair_key) {
                candidates.push(EntityCandidate {
                    entity_id: entity_id.clone(),
                    candidate_entity_id: candidate_entity_id.clone(),
                    similarity,
                });
            }
        }
    }

    candidates.sort_by(|left, right| {
        right
            .similarity
            .total_cmp(&left.similarity)
            .then_with(|| left.entity_id.cmp(&right.entity_id))
            .then_with(|| left.candidate_entity_id.cmp(&right.candidate_entity_id))
    });

    Ok(candidates)
}

fn validate_config(config: &CandidateConfig) -> Result<(), CandidateGenerationError> {
    if !config.minimum_similarity.is_finite() || !(-1.0..=1.0).contains(&config.minimum_similarity)
    {
        return Err(CandidateGenerationError::InvalidMinimumSimilarity {
            value: config.minimum_similarity,
        });
    }

    Ok(())
}

fn cosine_similarity(
    left: &EntityEmbedding,
    left_magnitude: f64,
    right: &EntityEmbedding,
    right_magnitude: f64,
) -> f32 {
    let dot_product = left
        .vector
        .values()
        .iter()
        .zip(right.vector.values())
        .map(|(left, right)| f64::from(*left) * f64::from(*right))
        .sum::<f64>();

    (dot_product / (left_magnitude * right_magnitude)).clamp(-1.0, 1.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::embedding::EmbeddingBatch;

    fn embedding(entity_id: &str, values: &[f32]) -> EntityEmbedding {
        let vector = EmbeddingBatch::try_from_raw(vec![values.to_vec()], 1)
            .expect("test vector should be valid")
            .into_vectors()
            .pop()
            .expect("test batch should contain one vector");

        EntityEmbedding {
            entity_id: entity_id.to_string(),
            vector,
        }
    }

    fn config(minimum_similarity: f32, max_candidates_per_entity: usize) -> CandidateConfig {
        CandidateConfig {
            minimum_similarity,
            max_candidates_per_entity,
        }
    }

    #[test]
    fn selects_similar_vectors() {
        let embeddings = vec![
            embedding("carbon-dioxide", &[1.0, 0.0]),
            embedding("co2", &[0.99, 0.01]),
        ];

        let candidates = generate_entity_candidates(&embeddings, &config(0.9, 3))
            .expect("similar vectors should be compared");

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].entity_id, "carbon-dioxide");
        assert_eq!(candidates[0].candidate_entity_id, "co2");
        assert!(candidates[0].similarity > 0.99);
    }

    #[test]
    fn excludes_vectors_below_the_loose_threshold() {
        let embeddings = vec![
            embedding("carbon-dioxide", &[1.0, 0.0]),
            embedding("oxygen", &[0.0, 1.0]),
        ];

        let candidates = generate_entity_candidates(&embeddings, &config(0.5, 3))
            .expect("valid vectors should be compared");

        assert!(candidates.is_empty());
    }

    #[test]
    fn limits_each_source_entity_to_its_top_three_candidates() {
        let embeddings = vec![
            embedding("a", &[1.0, 0.0]),
            embedding("b", &[1.0, 0.01]),
            embedding("c", &[1.0, 0.02]),
            embedding("d", &[1.0, 0.03]),
            embedding("e", &[1.0, 0.04]),
        ];

        let candidates = generate_entity_candidates(&embeddings, &config(0.9, 3))
            .expect("valid vectors should be compared");
        let candidates_from_a = candidates
            .iter()
            .filter(|candidate| candidate.entity_id == "a")
            .collect::<Vec<_>>();

        assert_eq!(candidates_from_a.len(), 3);
        assert!(candidates_from_a
            .iter()
            .all(|candidate| candidate.candidate_entity_id != "e"));
        assert!(candidates
            .iter()
            .fold(
                std::collections::HashMap::<&str, usize>::new(),
                |mut counts, candidate| {
                    *counts.entry(&candidate.entity_id).or_default() += 1;
                    counts
                }
            )
            .values()
            .all(|count| *count <= 3));
    }

    #[test]
    fn excludes_self_comparisons() {
        let embeddings = vec![
            embedding("a", &[1.0, 0.0]),
            embedding("b", &[1.0, 0.0]),
            embedding("c", &[1.0, 0.0]),
        ];

        let candidates = generate_entity_candidates(&embeddings, &config(0.5, 3))
            .expect("valid vectors should be compared");

        assert!(candidates
            .iter()
            .all(|candidate| candidate.entity_id != candidate.candidate_entity_id));
    }

    #[test]
    fn removes_reverse_duplicate_pairs() {
        let embeddings = vec![embedding("a", &[1.0, 0.0]), embedding("b", &[1.0, 0.0])];

        let candidates = generate_entity_candidates(&embeddings, &config(0.5, 3))
            .expect("valid vectors should be compared");

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].entity_id, "a");
        assert_eq!(candidates[0].candidate_entity_id, "b");
    }

    #[test]
    fn ties_and_input_order_use_stable_entity_ids() {
        let first = vec![
            embedding("c", &[1.0, 0.0]),
            embedding("a", &[1.0, 0.0]),
            embedding("b", &[1.0, 0.0]),
        ];
        let second = vec![
            embedding("b", &[1.0, 0.0]),
            embedding("c", &[1.0, 0.0]),
            embedding("a", &[1.0, 0.0]),
        ];

        let first_candidates = generate_entity_candidates(&first, &config(0.5, 3))
            .expect("valid vectors should be compared");
        let second_candidates = generate_entity_candidates(&second, &config(0.5, 3))
            .expect("valid vectors should be compared");

        assert_eq!(first_candidates, second_candidates);
        assert_eq!(
            first_candidates
                .iter()
                .map(|candidate| (
                    candidate.entity_id.as_str(),
                    candidate.candidate_entity_id.as_str()
                ))
                .collect::<Vec<_>>(),
            vec![("a", "b"), ("a", "c"), ("b", "c")]
        );
    }

    #[test]
    fn rejects_dimension_mismatches() {
        let embeddings = vec![
            embedding("a", &[1.0, 0.0]),
            embedding("b", &[1.0, 0.0, 0.0]),
        ];

        let error = generate_entity_candidates(&embeddings, &config(0.5, 3))
            .expect_err("different vector dimensions must be rejected");

        assert_eq!(
            error,
            CandidateGenerationError::DimensionMismatch {
                entity_id: String::from("b"),
                expected: 2,
                actual: 3,
            }
        );
    }

    #[test]
    fn rejects_duplicate_entity_ids() {
        let embeddings = vec![
            embedding("duplicate", &[1.0, 0.0]),
            embedding("duplicate", &[1.0, 0.0]),
        ];

        let error = generate_entity_candidates(&embeddings, &config(0.5, 3))
            .expect_err("duplicate stable IDs would make pair identity ambiguous");

        assert_eq!(
            error,
            CandidateGenerationError::DuplicateEntityId {
                entity_id: String::from("duplicate"),
            }
        );
    }

    #[test]
    fn rejects_invalid_thresholds_and_zero_magnitude_vectors() {
        let invalid_threshold =
            generate_entity_candidates(&[], &config(f32::NAN, 3)).expect_err("NaN must fail");
        assert!(matches!(
            invalid_threshold,
            CandidateGenerationError::InvalidMinimumSimilarity { value } if value.is_nan()
        ));

        let error = generate_entity_candidates(&[embedding("zero", &[0.0, 0.0])], &config(0.5, 3))
            .expect_err("zero vectors cannot produce cosine similarity");
        assert_eq!(
            error,
            CandidateGenerationError::ZeroMagnitudeVector {
                entity_id: String::from("zero"),
            }
        );
    }
}
