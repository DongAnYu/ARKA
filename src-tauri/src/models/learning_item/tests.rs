use super::*;

const PAIRED: &str = include_str!("fixtures/generated-pair.json");
const FLASHCARD_ONLY: &str = include_str!("fixtures/generated-flashcard-only.json");
const LEGACY: &str = include_str!("fixtures/legacy-mcq.json");

fn output(raw: &str) -> GeneratedItemsOutput {
    serde_json::from_str(raw).expect("fixture should deserialize")
}

#[test]
fn generated_pairs_and_flashcard_only_items_validate() {
    for raw in [PAIRED, FLASHCARD_ONLY] {
        let batch = output(raw);
        assert_eq!(validate_generated_output(&batch, &["kp_2"]), Ok(()));
        let encoded = serde_json::to_value(&batch).unwrap();
        assert_eq!(
            encoded,
            serde_json::from_str::<serde_json::Value>(raw).unwrap()
        );
    }
}

#[test]
fn legacy_mcq_does_not_infer_a_flashcard_or_target() {
    let item: LearningItem = serde_json::from_str(LEGACY).unwrap();
    assert!(!item
        .variants
        .iter()
        .any(|variant| matches!(variant, QuestionVariant::Flashcard { .. })));
    assert!(item.target.is_none());
    assert_eq!(item.variants.len(), 1);
    assert_eq!(item.generation.model.as_deref(), Some("legacy-model"));
    assert!(item.generation.provider.is_none());
    assert_eq!(item.schedule.interval_days, 38);
    assert_eq!(
        serde_json::to_value(item).unwrap(),
        serde_json::from_str::<serde_json::Value>(LEGACY).unwrap()
    );
}

#[test]
fn paired_item_keeps_one_schedule_and_two_explicit_variants() {
    let mut item: LearningItem = serde_json::from_str(LEGACY).unwrap();
    item.target = Some("Binary search worst-case complexity".into());
    item.variants.push(QuestionVariant::Flashcard {
        id: 86,
        prompt: "What is binary search's worst-case complexity?".into(),
    });
    assert!(item
        .variants
        .iter()
        .any(|variant| matches!(variant, QuestionVariant::Flashcard { .. })));
    assert_eq!(item.variants.len(), 2);
    assert_eq!(item.schedule.repetitions, 4);
}

#[test]
fn generated_items_require_a_real_flashcard() {
    for replacement in [serde_json::Value::Null, serde_json::json!({})] {
        let mut value: serde_json::Value = serde_json::from_str(PAIRED).unwrap();
        value["items"][0]["flashcard"] = replacement;
        assert!(serde_json::from_value::<GeneratedItemsOutput>(value).is_err());
    }
    let mut batch = output(PAIRED);
    batch.items[0].flashcard.prompt = "  ".into();
    assert_eq!(
        validate_generated_output(&batch, &["kp_2"]),
        Err("empty_core_field")
    );
}

#[test]
fn invalid_optional_mcq_is_detected_without_mutating_the_flashcard() {
    let mut batch = output(PAIRED);
    batch.items[0].mcq.as_mut().unwrap().distractors[0] = " O(log n) ".into();
    let original = batch.clone();
    assert_eq!(
        validate_generated_output(&batch, &["kp_2"]),
        Err("duplicate_option")
    );
    assert_eq!(batch, original);
    // The generation layer will decide whether to keep the valid core; contracts
    // must not silently repair or drop content on deserialization.
}

#[test]
fn batch_limits_and_point_identity_are_enforced() {
    let mut batch = output(PAIRED);
    assert_eq!(
        validate_generated_output(&batch, &["other"]),
        Err("unknown_knowledge_point")
    );
    batch.items.push(batch.items[0].clone());
    assert_eq!(
        validate_generated_output(&batch, &["kp_2"]),
        Err("duplicate_knowledge_point")
    );
    batch.items = vec![batch.items[0].clone(); 5];
    assert_eq!(
        validate_generated_output(&batch, &["kp_2"]),
        Err("too_many_items")
    );
    batch.items.clear();
    assert_eq!(validate_generated_output(&batch, &[]), Ok(()));
}

#[test]
fn omission_reason_and_mcq_content_must_agree() {
    let mut batch = output(FLASHCARD_ONLY);
    batch.items[0].mcq_omission_reason = None;
    assert_eq!(
        validate_generated_output(&batch, &["kp_2"]),
        Err("missing_omission_reason")
    );
    batch.items[0].mcq_omission_reason = Some("   ".to_string());
    assert_eq!(
        validate_generated_output(&batch, &["kp_2"]),
        Err("missing_omission_reason")
    );
    let mut batch = output(PAIRED);
    batch.items[0].mcq_omission_reason = Some("MCQ is ambiguous.".to_string());
    assert_eq!(
        validate_generated_output(&batch, &["kp_2"]),
        Err("unexpected_omission_reason")
    );
}

#[test]
fn mcq_payload_checks_count_empty_and_duplicate_options() {
    for (distractors, expected) in [
        (vec!["a", "b"], "distractor_count"),
        (vec!["a", " ", "c"], "empty_distractor"),
        (vec!["A", " a ", "c"], "duplicate_option"),
    ] {
        let mut batch = output(PAIRED);
        batch.items[0].mcq.as_mut().unwrap().distractors =
            distractors.into_iter().map(String::from).collect();
        assert_eq!(validate_generated_output(&batch, &["kp_2"]), Err(expected));
    }
}

#[test]
fn transient_reviews_need_no_history_metadata() {
    for response in [
        serde_json::json!({"format": "mcq", "selected_option_id": "B"}),
        serde_json::json!({"format": "flashcard", "rating": "good"}),
    ] {
        let value = serde_json::json!({
            "learning_item_id": 42, "variant_id": 84, "response": response
        });
        let submission: ReviewSubmission = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(submission).unwrap(), value);
    }
}

#[test]
fn submission_cannot_supply_its_own_mcq_correctness() {
    let value = serde_json::json!({
        "learning_item_id": 42, "variant_id": 84,
        "response": {"format": "mcq", "selected_option_id": "B", "is_correct": true}
    });
    assert!(serde_json::from_value::<ReviewSubmission>(value).is_err());
}

#[test]
fn generated_save_input_cannot_supply_provenance() {
    let content = serde_json::from_str::<serde_json::Value>(PAIRED).unwrap()["items"][0].clone();
    let valid = serde_json::json!({
        "draft_id": "draft-1",
        "content": content.clone()
    });
    assert!(serde_json::from_value::<GeneratedLearningItemSaveInput>(valid).is_ok());

    let value = serde_json::json!({
        "draft_id": "draft-1",
        "content": content,
        "generation": {
            "model": "spoofed-model",
            "provider": "openai",
            "pipeline": "chunk",
            "generated_at": "2026-09-09T12:00:00Z"
        }
    });

    assert!(serde_json::from_value::<GeneratedLearningItemSaveInput>(value).is_err());
}

#[test]
fn edit_input_cannot_replace_generation_or_source_provenance() {
    let value = serde_json::json!({
        "target": "Edited target",
        "answer": "Edited answer",
        "explanation": null,
        "space_id": 1,
        "variants": [{"format": "flashcard", "prompt": "Edited prompt?"}],
        "generation": {"model": "spoofed-model"},
        "source": null
    });

    assert!(serde_json::from_value::<LearningItemEditInput>(value).is_err());
}
