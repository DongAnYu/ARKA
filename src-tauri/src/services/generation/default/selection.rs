//! Fixed Default-generation policy. Scores never change as selection grows.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::models::generation_options::{GenerationOptions, GenerationPurpose};

use super::candidates::{CandidateId, KnowledgeCandidate, SectionIdentity};

pub const SELECTION_POLICY_VERSION: &str = "default-importance-v2";
pub const MAX_POINTS_PER_CHUNK: usize = 4;

/// Initial product weights, to be evaluated rather than treated as calibrated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PurposeWeights {
    pub centrality: u16,
    pub explanatory: u16,
    pub foundational: u16,
    pub distinct: u16,
    pub application: u16,
}

pub fn purpose_weights(purpose: GenerationPurpose) -> PurposeWeights {
    let (centrality, explanatory, foundational, distinct, application) = match purpose {
        GenerationPurpose::Balanced => (25, 20, 25, 15, 15),
        GenerationPurpose::Foundations => (20, 10, 45, 15, 10),
        GenerationPurpose::Explanations => (20, 45, 10, 15, 10),
        GenerationPurpose::PracticalApplication => (20, 15, 10, 15, 40),
    };
    assert_eq!(
        centrality + explanatory + foundational + distinct + application,
        100
    );
    PurposeWeights {
        centrality,
        explanatory,
        foundational,
        distinct,
        application,
    }
}

pub fn purpose_description(purpose: GenerationPurpose) -> &'static str {
    match purpose {
        GenerationPurpose::Balanced => "Broad coverage of important knowledge.",
        GenerationPurpose::Foundations => {
            "Essential definitions, prerequisites, and core principles."
        }
        GenerationPurpose::Explanations => "Mechanisms, causes, relationships, and conditions.",
        GenerationPurpose::PracticalApplication => "Procedures, decisions, and applying knowledge.",
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ImportanceAssessment {
    pub centrality: u8,
    pub explanatory_value: u8,
    pub foundational_value: u8,
    pub distinct_contribution: u8,
    pub application_value: u8,
    pub suitable_learning_target: bool,
    pub requires_source_context: bool,
    pub justification: String,
}

impl ImportanceAssessment {
    pub fn parse(json: &str) -> Result<Self, String> {
        let assessment: Self = serde_json::from_str(json).map_err(|error| error.to_string())?;
        assessment.validate()?;
        Ok(assessment)
    }

    pub fn validate(&self) -> Result<(), String> {
        if [
            self.centrality,
            self.explanatory_value,
            self.foundational_value,
            self.distinct_contribution,
            self.application_value,
        ]
        .iter()
        .any(|score| !(1..=5).contains(score))
        {
            return Err("Importance scores must be integers from 1 through 5".into());
        }
        if self.justification.trim().is_empty() || self.justification.chars().count() > 600 {
            return Err("Assessment justification must contain 1 to 600 characters".into());
        }
        Ok(())
    }

    pub fn weighted_score(&self, purpose: GenerationPurpose) -> u16 {
        let weights = purpose_weights(purpose);
        weights.centrality * u16::from(self.centrality)
            + weights.explanatory * u16::from(self.explanatory_value)
            + weights.foundational * u16::from(self.foundational_value)
            + weights.distinct * u16::from(self.distinct_contribution)
            + weights.application * u16::from(self.application_value)
    }
}

/// An exhausted/ambiguous assessment is retained with a reason, never a score.
#[derive(Debug, Clone)]
pub struct CandidateAssessment {
    pub result: Result<ImportanceAssessment, String>,
    pub used_source_context: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SelectionShortfall {
    pub reason: &'static str,
    pub message: String,
}

/// Default-only job metrics; variant counts are deliberately separate.
#[derive(Debug, Clone, Serialize)]
pub struct DefaultSelectionReport {
    pub options: GenerationOptions,
    pub policy_version: &'static str,
    pub extracted_chunks: usize,
    pub extraction_failed_chunks: usize,
    pub candidate_count: usize,
    /// Completed assessment tasks, including unsuccessful assessments.
    pub assessed_count: usize,
    pub unresolved_assessments: usize,
    pub unsuitable_count: usize,
    pub source_context_reassessments: usize,
    pub eligible_count: usize,
    pub substantive_sections: usize,
    pub selected_sections: usize,
    pub selected_count: usize,
    pub selection_complete: bool,
    pub chunk_capacity_excluded: usize,
    pub generation_total_chunks: usize,
    pub generation_completed_chunks: usize,
    pub generation_failed_chunks: usize,
    pub generation_failed_targets: usize,
    pub generation_omitted_count: usize,
    pub generated_count: usize,
    pub shortfall: Vec<SelectionShortfall>,
}

impl DefaultSelectionReport {
    pub fn new(options: GenerationOptions) -> Self {
        Self {
            options,
            policy_version: SELECTION_POLICY_VERSION,
            extracted_chunks: 0,
            extraction_failed_chunks: 0,
            candidate_count: 0,
            assessed_count: 0,
            unresolved_assessments: 0,
            unsuitable_count: 0,
            source_context_reassessments: 0,
            eligible_count: 0,
            substantive_sections: 0,
            selected_sections: 0,
            selected_count: 0,
            selection_complete: false,
            chunk_capacity_excluded: 0,
            generation_total_chunks: 0,
            generation_completed_chunks: 0,
            generation_failed_chunks: 0,
            generation_failed_targets: 0,
            generation_omitted_count: 0,
            generated_count: 0,
            shortfall: Vec::new(),
        }
    }

    pub fn explain_shortfall(&mut self) {
        self.shortfall.clear();
        if !self.selection_complete
            || self.generated_count >= self.options.max_learning_items() as usize
        {
            return;
        }
        if self.selected_count < self.options.max_learning_items() as usize {
            if self.unresolved_assessments > 0 {
                self.shortfall.push(SelectionShortfall {
                    reason: "unresolved_assessments",
                    message: format!("{} concepts were excluded because their importance assessments failed or still needed context.", self.unresolved_assessments),
                });
            }
            if self.extraction_failed_chunks > 0 {
                self.shortfall.push(SelectionShortfall {
                    reason: "extraction_failures",
                    message: format!(
                        "Knowledge extraction failed for {} source chunks.",
                        self.extraction_failed_chunks
                    ),
                });
            }
            if self.chunk_capacity_excluded > 0 {
                self.shortfall.push(SelectionShortfall {
                    reason: "chunk_capacity",
                    message: format!("{} eligible concepts exceeded the limit of four selected concepts per source chunk.", self.chunk_capacity_excluded),
                });
            }
            if self.eligible_count < self.options.max_learning_items() as usize {
                self.shortfall.push(SelectionShortfall {
                    reason: "insufficient_eligible_concepts",
                    message: format!("{} eligible concepts were available after assessment ({} assessed as unsuitable).", self.eligible_count, self.unsuitable_count),
                });
            }
        }
        if self.generation_failed_targets > 0 {
            self.shortfall.push(SelectionShortfall {
                reason: "generation_failures",
                message: format!(
                    "Generation failed for {} selected concepts across {} chunks.",
                    self.generation_failed_targets, self.generation_failed_chunks
                ),
            });
        }
        if self.generation_omitted_count > 0 {
            self.shortfall.push(SelectionShortfall {
                reason: "generation_omissions",
                message: format!(
                    "The model returned no valid learning item for {} selected concepts.",
                    self.generation_omitted_count
                ),
            });
        }
        if self.generation_completed_chunks < self.generation_total_chunks {
            let remaining = self.selected_count.saturating_sub(
                self.generated_count
                    + self.generation_failed_targets
                    + self.generation_omitted_count,
            );
            self.shortfall.push(SelectionShortfall {
                reason: "generation_stopped",
                message: format!(
                    "Generation stopped before {remaining} selected concepts were processed."
                ),
            });
        }
    }
}

#[derive(Debug)]
pub struct CandidateSelection {
    /// Explicit Stage B allocation, ordered by original point position.
    pub by_chunk: BTreeMap<usize, Vec<KnowledgeCandidate>>,
    pub report: DefaultSelectionReport,
}

pub fn select_candidates(
    candidates: &[KnowledgeCandidate],
    assessments: &BTreeMap<CandidateId, CandidateAssessment>,
    options: GenerationOptions,
) -> CandidateSelection {
    let mut report = DefaultSelectionReport::new(options);
    report.candidate_count = candidates.len();
    report.assessed_count = candidates
        .iter()
        .filter(|candidate| assessments.contains_key(&candidate.id))
        .count();
    let mut seen = BTreeSet::new();
    let mut eligible = Vec::new();
    for candidate in candidates {
        if !seen.insert(candidate.id) {
            continue;
        }
        let Some(outcome) = assessments.get(&candidate.id) else {
            report.unresolved_assessments += 1;
            continue;
        };
        report.source_context_reassessments += usize::from(outcome.used_source_context);
        match &outcome.result {
            Ok(assessment)
                if assessment.validate().is_ok() && !assessment.requires_source_context =>
            {
                if assessment.suitable_learning_target {
                    eligible.push((candidate, assessment.weighted_score(options.purpose())));
                } else {
                    report.unsuitable_count += 1;
                }
            }
            _ => report.unresolved_assessments += 1,
        }
    }
    // One immutable ranking with a complete source-order tie break. No text comparison.
    eligible.sort_by(|(left, left_score), (right, right_score)| {
        right_score
            .cmp(left_score)
            .then(left.source_chunk_index.cmp(&right.source_chunk_index))
            .then(left.section.section_index.cmp(&right.section.section_index))
            .then(left.point_position.cmp(&right.point_position))
            .then(left.id.cmp(&right.id))
    });
    report.eligible_count = eligible.len();
    // Future quality thresholds for section representatives need testing on
    // representative notes: an arbitrary cutoff could exclude valuable
    // definitions or prerequisites. Keep coverage-first selection for now.
    let mut representatives = BTreeSet::<SectionIdentity>::new();
    let representatives = eligible
        .iter()
        .filter_map(|(candidate, _)| {
            representatives
                .insert(candidate.section.clone())
                .then_some(*candidate)
        })
        .collect::<Vec<_>>();
    report.substantive_sections = representatives.len();
    let maximum = options.max_learning_items() as usize;
    let mut selected = BTreeSet::new();
    let mut by_chunk = BTreeMap::<usize, Vec<KnowledgeCandidate>>::new();
    for candidate in representatives
        .into_iter()
        .chain(eligible.iter().map(|(candidate, _)| *candidate))
    {
        if selected.len() == maximum {
            break;
        }
        if selected.contains(&candidate.id) {
            continue;
        }
        let chunk = by_chunk.entry(candidate.source_chunk_index).or_default();
        if chunk.len() == MAX_POINTS_PER_CHUNK {
            continue;
        }
        selected.insert(candidate.id);
        chunk.push(candidate.clone());
    }
    report.selected_count = selected.len();
    report.selected_sections = by_chunk
        .values()
        .flatten()
        .map(|candidate| candidate.section.clone())
        .collect::<BTreeSet<_>>()
        .len();
    report.chunk_capacity_excluded = eligible
        .iter()
        .filter(|(candidate, _)| {
            !selected.contains(&candidate.id)
                && by_chunk
                    .get(&candidate.source_chunk_index)
                    .is_some_and(|chunk| chunk.len() == MAX_POINTS_PER_CHUNK)
        })
        .count();
    report.selection_complete = true;
    report.generation_total_chunks = by_chunk.len();
    for chunk in by_chunk.values_mut() {
        chunk.sort_by_key(|candidate| candidate.point_position);
    }
    CandidateSelection { by_chunk, report }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::chunker::MarkdownChunk;
    use crate::services::generation::default::candidates::DefaultGenerationJob;
    use serde_json::json;

    fn assessment(score: u8) -> ImportanceAssessment {
        ImportanceAssessment::parse(&json!({
            "centrality":score,"explanatory_value":score,"foundational_value":score,"distinct_contribution":score,"application_value":score,
            "suitable_learning_target":true,"requires_source_context":false,"justification":"Grounded learning target."
        }).to_string()).unwrap()
    }

    fn options(maximum: u32) -> GenerationOptions {
        serde_json::from_value(json!({"max_learning_items":maximum})).unwrap()
    }

    fn fixture(
        sections: &[(usize, Vec<u8>)],
    ) -> (
        Vec<KnowledgeCandidate>,
        BTreeMap<CandidateId, CandidateAssessment>,
    ) {
        let chunks = sections
            .iter()
            .enumerate()
            .map(|(index, (section, _))| MarkdownChunk {
                note_path: "note.md".into(),
                note_title: "Subject".into(),
                heading: "Repeated heading".into(),
                content: "Uneven section length. ".repeat((index + 1) * 10),
                section_index: *section,
                chunk_index: index,
                start_line: index * 20 + 1,
                end_line: index * 20 + 20,
            })
            .collect();
        let job = DefaultGenerationJob::new(GenerationOptions::default(), chunks);
        for (index, (_, scores)) in sections.iter().enumerate() {
            // Similar and identical text must not be dropped by the selector.
            job.register_stage_a(
                index,
                &scores
                    .iter()
                    .map(|_| "The same useful point".into())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        }
        let candidates = job.candidates().unwrap();
        let outcomes = candidates
            .iter()
            .map(|candidate| {
                (
                    candidate.id,
                    CandidateAssessment {
                        result: Ok(assessment(
                            sections[candidate.source_chunk_index].1[candidate.point_position],
                        )),
                        used_source_context: false,
                    },
                )
            })
            .collect();
        (candidates, outcomes)
    }

    fn positions(selection: &CandidateSelection) -> Vec<(usize, usize)> {
        selection
            .by_chunk
            .values()
            .flatten()
            .map(|point| (point.source_chunk_index, point.point_position))
            .collect()
    }

    #[test]
    fn purpose_policy_has_exact_weights_and_fixed_scores() {
        let purposes = [
            GenerationPurpose::Balanced,
            GenerationPurpose::Foundations,
            GenerationPurpose::Explanations,
            GenerationPurpose::PracticalApplication,
        ];
        let expected = [
            (25, 20, 25, 15, 15),
            (20, 10, 45, 15, 10),
            (20, 45, 10, 15, 10),
            (20, 15, 10, 15, 40),
        ];
        for (purpose, (c, e, f, d, a)) in purposes.into_iter().zip(expected) {
            assert_eq!(
                purpose_weights(purpose),
                PurposeWeights {
                    centrality: c,
                    explanatory: e,
                    foundational: f,
                    distinct: d,
                    application: a,
                }
            );
            assert_eq!(assessment(1).weighted_score(purpose), 100);
            assert_eq!(assessment(5).weighted_score(purpose), 500);
        }
        let mut foundational = assessment(3);
        foundational.foundational_value = 5;
        foundational.explanatory_value = 1;
        let mut explanatory = assessment(3);
        explanatory.foundational_value = 1;
        explanatory.explanatory_value = 5;
        assert!(
            foundational.weighted_score(GenerationPurpose::Foundations)
                > explanatory.weighted_score(GenerationPurpose::Foundations)
        );
        assert!(
            foundational.weighted_score(GenerationPurpose::Explanations)
                < explanatory.weighted_score(GenerationPurpose::Explanations)
        );
    }

    #[test]
    fn practical_and_explanation_purposes_select_different_supported_targets() {
        let (candidates, mut assessments) = fixture(&[(0, vec![3, 3])]);
        let explanatory = assessments
            .get_mut(&candidates[0].id)
            .unwrap()
            .result
            .as_mut()
            .unwrap();
        explanatory.explanatory_value = 5;
        explanatory.application_value = 1;
        let practical = assessments
            .get_mut(&candidates[1].id)
            .unwrap()
            .result
            .as_mut()
            .unwrap();
        practical.explanatory_value = 1;
        practical.application_value = 5;

        for (purpose, expected) in [("explanations", 0), ("practical_application", 1)] {
            let options =
                serde_json::from_value(json!({"max_learning_items":1,"purpose":purpose})).unwrap();
            let selection = select_candidates(&candidates, &assessments, options);
            assert_eq!(selection.report.selected_count, 1);
            assert_eq!(selection.by_chunk[&0][0].id, candidates[expected].id);
        }
    }

    #[test]
    fn assessment_rejects_invalid_scores_flags_and_extra_ids() {
        let valid = json!({"centrality":3,"explanatory_value":3,"foundational_value":3,"distinct_contribution":3,"application_value":3,
            "suitable_learning_target":true,"requires_source_context":false,"justification":"Valid."});
        assert!(ImportanceAssessment::parse(&valid.to_string()).is_ok());
        for field in [
            "centrality",
            "explanatory_value",
            "foundational_value",
            "distinct_contribution",
            "application_value",
        ] {
            for invalid in [
                json!(0),
                json!(6),
                json!(-1),
                json!(2.5),
                json!("3"),
                json!(null),
                json!(true),
            ] {
                let mut response = valid.clone();
                response[field] = invalid;
                assert!(
                    ImportanceAssessment::parse(&response.to_string()).is_err(),
                    "{response}"
                );
            }
        }
        for field in ["suitable_learning_target", "requires_source_context"] {
            let mut response = valid.clone();
            response[field] = json!("false");
            assert!(ImportanceAssessment::parse(&response.to_string()).is_err());
        }
        for field in ["candidate_id", "chunk_id"] {
            let mut response = valid.clone();
            response[field] = json!("opaque");
            assert!(ImportanceAssessment::parse(&response.to_string()).is_err());
        }
        for justification in ["".to_string(), "  ".to_string(), "x".repeat(601)] {
            let mut response = valid.clone();
            response["justification"] = json!(justification);
            assert!(ImportanceAssessment::parse(&response.to_string()).is_err());
        }
        let mut missing = valid;
        missing
            .as_object_mut()
            .unwrap()
            .remove("requires_source_context");
        assert!(ImportanceAssessment::parse(&missing.to_string()).is_err());
        let mut missing_application = missing;
        missing_application["requires_source_context"] = json!(false);
        missing_application
            .as_object_mut()
            .unwrap()
            .remove("application_value");
        assert!(ImportanceAssessment::parse(&missing_application.to_string()).is_err());
        assert!(ImportanceAssessment::parse("[]").is_err());
    }

    #[test]
    fn section_coverage_handles_budgets_below_equal_and_above_section_count() {
        let (candidates, outcomes) = fixture(&[
            (0, vec![5, 5, 5, 5, 5]),
            (0, vec![4]),
            (1, vec![1]),
            (2, vec![3, 2]),
        ]);
        assert_eq!(
            positions(&select_candidates(&candidates, &outcomes, options(1))),
            vec![(0, 0)]
        );
        assert_eq!(
            positions(&select_candidates(&candidates, &outcomes, options(2))),
            vec![(0, 0), (3, 0)]
        );
        let covered = select_candidates(&candidates, &outcomes, options(3));
        assert_eq!(positions(&covered), vec![(0, 0), (2, 0), (3, 0)]);
        assert_eq!(covered.report.substantive_sections, 3);
        assert_eq!(covered.report.selected_sections, 3);
        assert_eq!(
            positions(&select_candidates(&candidates, &outcomes, options(5))),
            vec![(0, 0), (0, 1), (0, 2), (2, 0), (3, 0)]
        );
    }

    #[test]
    fn stable_ties_capacity_and_duplicate_text_are_independent_of_completion_order() {
        let (mut candidates, outcomes) = fixture(&[(0, vec![3; 7]), (1, vec![3; 3])]);
        let expected = select_candidates(&candidates, &outcomes, options(20));
        assert_eq!(expected.report.selected_count, 7);
        assert_eq!(expected.report.chunk_capacity_excluded, 3);
        assert_eq!(expected.by_chunk[&0].len(), 4);
        assert_eq!(expected.by_chunk[&1].len(), 3);
        assert_eq!(
            positions(&expected),
            vec![(0, 0), (0, 1), (0, 2), (0, 3), (1, 0), (1, 1), (1, 2)]
        );
        candidates.reverse();
        let reversed = outcomes
            .iter()
            .rev()
            .map(|(id, outcome)| (*id, outcome.clone()))
            .collect();
        assert_eq!(
            positions(&select_candidates(&candidates, &reversed, options(20))),
            positions(&expected)
        );
        assert_eq!(
            select_candidates(&candidates, &reversed, options(2))
                .report
                .selected_count,
            2
        );
        for outcome in reversed.values() {
            assert_eq!(
                outcome
                    .result
                    .as_ref()
                    .unwrap()
                    .weighted_score(GenerationPurpose::Balanced),
                300
            );
        }
        candidates.push(candidates[0].clone());
        assert_eq!(
            select_candidates(&candidates, &outcomes, options(20))
                .report
                .selected_count,
            7
        );
    }

    #[test]
    fn empty_unsuitable_failed_and_unresolved_assessments_do_not_fall_back() {
        let empty = select_candidates(&[], &BTreeMap::new(), options(20));
        assert_eq!(empty.report.selected_count, 0);
        let (candidates, mut outcomes) = fixture(&[(0, vec![5; 4])]);
        outcomes
            .get_mut(&candidates[0].id)
            .unwrap()
            .result
            .as_mut()
            .unwrap()
            .suitable_learning_target = false;
        outcomes.get_mut(&candidates[1].id).unwrap().result = Err("Invalid response".into());
        outcomes
            .get_mut(&candidates[2].id)
            .unwrap()
            .result
            .as_mut()
            .unwrap()
            .requires_source_context = true;
        outcomes.remove(&candidates[3].id);
        let mut selection = select_candidates(&candidates, &outcomes, options(20));
        assert!(selection.by_chunk.is_empty());
        assert_eq!(selection.report.unsuitable_count, 1);
        assert_eq!(selection.report.unresolved_assessments, 3);
        selection.report.explain_shortfall();
        assert!(selection
            .report
            .shortfall
            .iter()
            .any(|reason| reason.reason == "unresolved_assessments"));
    }

    #[test]
    fn shortfall_reports_actual_capacity_failures_and_omissions_separately() {
        let (candidates, outcomes) = fixture(&[(0, vec![5; 6]), (1, vec![3; 2])]);
        let mut report = select_candidates(&candidates, &outcomes, options(10)).report;
        report.generated_count = 3;
        report.generation_omitted_count = 1;
        report.generation_failed_chunks = 1;
        report.generation_failed_targets = 2;
        report.generation_completed_chunks = 2;
        report.explain_shortfall();
        let reasons = report
            .shortfall
            .iter()
            .map(|reason| reason.reason)
            .collect::<Vec<_>>();
        assert!(reasons.contains(&"chunk_capacity"));
        assert!(reasons.contains(&"insufficient_eligible_concepts"));
        assert!(reasons.contains(&"generation_omissions"));
        assert!(reasons.contains(&"generation_failures"));
        assert!(!reasons.contains(&"generation_stopped"));
        assert_eq!(report.selected_count, 6);
        assert_eq!(report.generated_count, 3);
    }

    #[test]
    fn default_maximum_is_enforced_on_long_notes_with_more_than_twenty_eligible_points() {
        let sections = (0..10).map(|index| (index, vec![5; 6])).collect::<Vec<_>>();
        let (candidates, outcomes) = fixture(&sections);
        let selection = select_candidates(&candidates, &outcomes, GenerationOptions::default());
        assert_eq!(selection.report.eligible_count, 60);
        assert_eq!(selection.report.selected_count, 20);
        assert_eq!(selection.report.selected_sections, 10);
        assert!(selection.by_chunk.values().all(|points| points.len() <= 4));
    }

    #[test]
    fn omission_with_a_full_allocation_does_not_blame_insufficient_knowledge() {
        let (candidates, outcomes) = fixture(&[(0, vec![5; 4])]);
        let mut report = select_candidates(&candidates, &outcomes, options(2)).report;
        report.generated_count = 1;
        report.generation_omitted_count = 1;
        report.generation_completed_chunks = 1;
        report.explain_shortfall();
        assert_eq!(report.shortfall.len(), 1);
        assert_eq!(report.shortfall[0].reason, "generation_omissions");
    }
}
