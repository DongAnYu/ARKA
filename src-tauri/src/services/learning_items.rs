use super::scheduler::{Rating, SM2Scheduler};
use crate::models::learning_item::*;
use crate::models::question::{Question, QuestionInput};
use sqlx::{Row, SqliteConnection, SqlitePool};

fn invalid(message: &str) -> sqlx::Error {
    sqlx::Error::Protocol(message.into())
}
fn json<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, sqlx::Error> {
    serde_json::from_str(value).map_err(|e| invalid(&format!("Invalid stored content: {e}")))
}

pub async fn load(connection: &mut SqliteConnection, id: i64) -> Result<LearningItem, sqlx::Error> {
    let row = sqlx::query("SELECT * FROM learning_items WHERE id=?")
        .bind(id)
        .fetch_one(&mut *connection)
        .await?;
    let pipeline: Option<String> = row.try_get("pipeline")?;
    let source: Option<String> = row.try_get("source_json")?;
    let mut variants = Vec::new();
    for v in sqlx::query("SELECT * FROM question_variants WHERE learning_item_id=? ORDER BY id")
        .bind(id)
        .fetch_all(&mut *connection)
        .await?
    {
        let format: String = v.try_get("format")?;
        let payload: serde_json::Value = json(&v.try_get::<String, _>("content_json")?)?;
        variants.push(match format.as_str() {
            "mcq" => QuestionVariant::Mcq {
                id: v.try_get("id")?,
                prompt: v.try_get("prompt")?,
                options: serde_json::from_value(payload["options"].clone())
                    .map_err(|_| invalid("Invalid options"))?,
                correct_option_id: payload["correct_option_id"]
                    .as_str()
                    .ok_or_else(|| invalid("Missing correct option"))?
                    .into(),
            },
            "flashcard" => QuestionVariant::Flashcard {
                id: v.try_get("id")?,
                prompt: v.try_get("prompt")?,
            },
            _ => return Err(invalid("Unsupported variant format")),
        });
    }
    Ok(LearningItem {
        id,
        target: row.try_get("target")?,
        answer: row.try_get("answer")?,
        explanation: row.try_get("explanation")?,
        space_id: row.try_get("space_id")?,
        generation: GenerationMetadata {
            model: row.try_get("model")?,
            provider: row.try_get("provider")?,
            pipeline: match pipeline.as_deref() {
                None => None,
                Some("chunk") => Some(GenerationPipeline::Chunk),
                Some("graph") => Some(GenerationPipeline::Graph),
                _ => return Err(invalid("Unsupported pipeline")),
            },
            generated_at: row.try_get("generated_at")?,
        },
        source: source.as_deref().map(json).transpose()?,
        status: match row.try_get::<String, _>("status")?.as_str() {
            "ready" => ItemStatus::Ready,
            "needs_repair" => ItemStatus::NeedsRepair,
            _ => return Err(invalid("Invalid status")),
        },
        schedule: ReviewState {
            repetitions: row.try_get("repetitions")?,
            interval_days: row.try_get("interval_days")?,
            ease_factor: row.try_get("ease_factor")?,
            next_review_at: row.try_get("next_review_at")?,
            last_reviewed_at: row.try_get("last_reviewed_at")?,
        },
        variants,
    })
}

pub async fn list(
    pool: &SqlitePool,
    space_id: Option<i64>,
    due_only: bool,
) -> Result<Vec<LearningItem>, sqlx::Error> {
    let mut transaction = pool.begin().await?;
    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM learning_items WHERE (? IS NULL OR space_id=?) AND (?=0 OR (status='ready' AND (next_review_at IS NULL OR next_review_at <= CURRENT_TIMESTAMP))) ORDER BY COALESCE(next_review_at,'1970-01-01'),id")
        .bind(space_id).bind(space_id).bind(due_only).fetch_all(&mut *transaction).await?;
    let mut items = Vec::with_capacity(ids.len());
    for id in ids {
        items.push(load(&mut transaction, id).await?);
    }
    transaction.commit().await?;
    Ok(items)
}

fn validate(input: &LearningItemInput) -> Result<(), sqlx::Error> {
    if input.answer.trim().is_empty() || input.variants.is_empty() || input.space_id <= 0 {
        return Err(invalid("Answer, variants and recall space are required"));
    }
    let mut formats = std::collections::HashSet::new();
    for variant in &input.variants {
        let (format, prompt) = match variant {
            VariantInput::Flashcard { prompt } => {
                if input.target.as_ref().map_or(true, |t| t.trim().is_empty()) {
                    return Err(invalid("Flashcards need an explicit learning target"));
                }
                ("flashcard", prompt)
            }
            VariantInput::Mcq {
                prompt,
                options,
                correct_option_id,
            } => {
                if options.len() != 4 {
                    return Err(invalid("MCQ requires four options"));
                }
                let mut ids = std::collections::HashSet::new();
                let mut texts = std::collections::HashSet::new();
                for option in options {
                    if option.id.trim().is_empty()
                        || option.text.trim().is_empty()
                        || !ids.insert(&option.id)
                        || !texts.insert(option.text.trim().to_lowercase())
                    {
                        return Err(invalid("MCQ options must be nonempty and distinct"));
                    }
                }
                if !options.iter().any(|o| &o.id == correct_option_id) {
                    return Err(invalid("Invalid correct option ID"));
                }
                ("mcq", prompt)
            }
        };
        if prompt.trim().is_empty() || !formats.insert(format) {
            return Err(invalid("Empty prompt or duplicate format"));
        }
    }
    Ok(())
}

async fn write(
    connection: &mut SqliteConnection,
    id: Option<i64>,
    input: &LearningItemInput,
) -> Result<i64, sqlx::Error> {
    validate(input)?;
    let source = input
        .source
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|_| invalid("Invalid source"))?;
    let id = if let Some(id) = id {
        // Editing content preserves original generation provenance and scheduling.
        let updated = sqlx::query("UPDATE learning_items SET target=?,answer=?,explanation=?,space_id=?,source_json=?,status='ready' WHERE id=?")
            .bind(&input.target).bind(&input.answer).bind(&input.explanation).bind(input.space_id).bind(source).bind(id).execute(&mut *connection).await?;
        if updated.rows_affected() != 1 {
            return Err(sqlx::Error::RowNotFound);
        }
        id
    } else {
        let pipeline = match input.generation.pipeline {
            Some(GenerationPipeline::Chunk) => Some("chunk"),
            Some(GenerationPipeline::Graph) => Some("graph"),
            None => None,
        };
        sqlx::query("INSERT INTO learning_items(target,answer,explanation,space_id,model,provider,pipeline,generated_at,source_json,next_review_at) VALUES(?,?,?,?,?,?,?,?,?,CURRENT_TIMESTAMP)")
            .bind(&input.target).bind(&input.answer).bind(&input.explanation).bind(input.space_id).bind(&input.generation.model).bind(&input.generation.provider).bind(pipeline).bind(&input.generation.generated_at).bind(source)
            .execute(&mut *connection).await?.last_insert_rowid()
    };
    let mut formats = Vec::new();
    for variant in &input.variants {
        let (format, prompt, payload) = match variant {
            VariantInput::Flashcard { prompt } => ("flashcard", prompt, serde_json::json!({})),
            VariantInput::Mcq {
                prompt,
                options,
                correct_option_id,
            } => {
                let mut options = options.clone();
                options
                    .iter_mut()
                    .find(|o| &o.id == correct_option_id)
                    .unwrap()
                    .text = input.answer.clone();
                // Validate again after synchronising the shared answer.
                let distinct: std::collections::HashSet<_> = options
                    .iter()
                    .map(|o| o.text.trim().to_lowercase())
                    .collect();
                if distinct.len() != 4 {
                    return Err(invalid("Shared answer duplicates a distractor"));
                }
                (
                    "mcq",
                    prompt,
                    serde_json::json!({"options": options, "correct_option_id": correct_option_id}),
                )
            }
        };
        formats.push(format);
        sqlx::query("INSERT INTO question_variants(learning_item_id,format,prompt,content_json) VALUES(?,?,?,?) ON CONFLICT(learning_item_id,format) DO UPDATE SET prompt=excluded.prompt,content_json=excluded.content_json")
            .bind(id).bind(format).bind(prompt).bind(payload.to_string()).execute(&mut *connection).await?;
    }
    for format in ["mcq", "flashcard"] {
        if !formats.contains(&format) {
            sqlx::query("DELETE FROM question_variants WHERE learning_item_id=? AND format=?")
                .bind(id)
                .bind(format)
                .execute(&mut *connection)
                .await?;
        }
    }
    Ok(id)
}

pub async fn save(
    pool: &SqlitePool,
    inputs: Vec<LearningItemInput>,
) -> Result<Vec<LearningItem>, sqlx::Error> {
    let mut transaction = pool.begin().await?;
    let mut result = Vec::new();
    for input in inputs {
        let id = write(&mut transaction, None, &input).await?;
        result.push(load(&mut transaction, id).await?);
    }
    transaction.commit().await?;
    Ok(result)
}

pub async fn modify(
    pool: &SqlitePool,
    id: i64,
    edit: LearningItemEditInput,
) -> Result<LearningItem, sqlx::Error> {
    let mut transaction = pool.begin().await?;
    let existing = load(&mut transaction, id).await?;
    let input = LearningItemInput {
        target: edit.target,
        answer: edit.answer,
        explanation: edit.explanation,
        space_id: edit.space_id,
        generation: existing.generation,
        source: existing.source,
        variants: edit.variants,
    };
    write(&mut transaction, Some(id), &input).await?;
    let result = load(&mut transaction, id).await?;
    transaction.commit().await?;
    Ok(result)
}

pub fn from_generated(
    content: GeneratedItem,
    space_id: i64,
    generation: GenerationMetadata,
    source: SourceReference,
    correct_index: usize,
) -> Result<LearningItemInput, sqlx::Error> {
    if space_id <= 0 {
        return Err(invalid("Recall space is required"));
    }
    if generation.pipeline != Some(GenerationPipeline::Chunk) {
        return Err(invalid(
            "Generated learning-item saves require the chunk pipeline",
        ));
    }

    let mut variants = vec![VariantInput::Flashcard {
        prompt: content.flashcard.prompt.clone(),
    }];
    if let Some(mcq) = content.mcq {
        let correct_index = correct_index % 4;
        let mut option_texts = mcq.distractors;
        option_texts.insert(correct_index, content.answer.clone());
        let options = option_texts
            .into_iter()
            .enumerate()
            .map(|(index, text)| McqOption {
                id: ((b'A' + index as u8) as char).to_string(),
                text,
            })
            .collect::<Vec<_>>();
        variants.push(VariantInput::Mcq {
            prompt: mcq
                .prompt
                .unwrap_or_else(|| content.flashcard.prompt.clone()),
            options,
            correct_option_id: ((b'A' + correct_index as u8) as char).to_string(),
        });
    }

    Ok(LearningItemInput {
        target: Some(content.target),
        answer: content.answer,
        explanation: content.explanation,
        space_id,
        generation,
        source: Some(source),
        variants,
    })
}

pub async fn review(
    pool: &SqlitePool,
    submission: ReviewSubmission,
) -> Result<LearningItem, sqlx::Error> {
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    let mut item = load(&mut transaction, submission.learning_item_id).await?;
    if item.status != ItemStatus::Ready {
        return Err(invalid("Item needs repair"));
    }
    let variant = item
        .variants
        .iter()
        .find(|v| match v {
            QuestionVariant::Mcq { id, .. } | QuestionVariant::Flashcard { id, .. } => {
                *id == submission.variant_id
            }
        })
        .ok_or_else(|| invalid("Variant does not belong to this item"))?;
    let rating = match (variant, submission.response) {
        (
            QuestionVariant::Mcq {
                options,
                correct_option_id,
                ..
            },
            ReviewResponse::Mcq { selected_option_id },
        ) => {
            if !options.iter().any(|o| o.id == selected_option_id) {
                return Err(invalid("Unknown answer option"));
            }
            if *correct_option_id == selected_option_id {
                Rating::Easy
            } else {
                Rating::Again
            }
        }
        (QuestionVariant::Flashcard { .. }, ReviewResponse::Flashcard { rating }) => match rating {
            ReviewRating::Again => Rating::Again,
            ReviewRating::Hard => Rating::Hard,
            ReviewRating::Good => Rating::Good,
            ReviewRating::Easy => Rating::Easy,
        },
        _ => return Err(invalid("Response does not match variant format")),
    };
    SM2Scheduler::review_state(&mut item.schedule, rating);
    let state = &item.schedule;
    sqlx::query("UPDATE learning_items SET repetitions=?,interval_days=?,ease_factor=?,next_review_at=?,last_reviewed_at=? WHERE id=?")
        .bind(state.repetitions).bind(state.interval_days).bind(state.ease_factor).bind(&state.next_review_at).bind(&state.last_reviewed_at).bind(item.id).execute(&mut *transaction).await?;
    transaction.commit().await?;
    Ok(item)
}

/// Both existing chunk and graph MCQ output use this adapter; no flashcard is inferred.
pub fn from_mcq(question: QuestionInput, model: String) -> Result<LearningItemInput, sqlx::Error> {
    let options: Vec<_> = [
        question.option_a,
        question.option_b,
        question.option_c,
        question.option_d,
    ]
    .into_iter()
    .enumerate()
    .map(|(i, text)| McqOption {
        id: ((b'A' + i as u8) as char).to_string(),
        text,
    })
    .collect();
    let answer = options
        .iter()
        .find(|o| o.id == question.correct_answer)
        .ok_or_else(|| invalid("Invalid correct answer label"))?
        .text
        .clone();
    Ok(LearningItemInput {
        target: None,
        answer,
        explanation: question.explanation,
        space_id: if question.space_id > 0 {
            question.space_id
        } else {
            1
        },
        generation: GenerationMetadata {
            model: Some(model),
            provider: None,
            pipeline: None,
            generated_at: None,
        },
        source: None,
        variants: vec![VariantInput::Mcq {
            prompt: question.question,
            options,
            correct_option_id: question.correct_answer,
        }],
    })
}

pub fn to_mcq(item: LearningItem) -> Result<Option<Question>, sqlx::Error> {
    let Some(QuestionVariant::Mcq {
        prompt,
        options,
        correct_option_id,
        ..
    }) = item
        .variants
        .iter()
        .find(|v| matches!(v, QuestionVariant::Mcq { .. }))
    else {
        return Ok(None);
    };
    if options.len() != 4 {
        return Err(invalid("MCQ compatibility requires four options"));
    }
    let correct_index = options
        .iter()
        .position(|o| &o.id == correct_option_id)
        .ok_or_else(|| invalid("Invalid stored answer"))?;
    Ok(Some(Question {
        id: item.id,
        question: prompt.clone(),
        option_a: options[0].text.clone(),
        option_b: options[1].text.clone(),
        option_c: options[2].text.clone(),
        option_d: options[3].text.clone(),
        correct_answer: ((b'A' + correct_index as u8) as char).to_string(),
        explanation: item.explanation,
        model: item.generation.model,
        space_id: item.space_id,
        repetitions: item.schedule.repetitions,
        interval_days: item.schedule.interval_days,
        ease_factor: item.schedule.ease_factor,
        next_review_at: item
            .schedule
            .next_review_at
            .map(|v| {
                chrono::NaiveDateTime::parse_from_str(&v, "%Y-%m-%d %H:%M:%S%.f")
                    .or_else(|_| v.parse())
                    .map_err(|_| invalid("Invalid review timestamp"))
            })
            .transpose()?,
        last_reviewed_at: item
            .schedule
            .last_reviewed_at
            .map(|v| {
                chrono::NaiveDateTime::parse_from_str(&v, "%Y-%m-%d %H:%M:%S%.f")
                    .or_else(|_| v.parse())
                    .map_err(|_| invalid("Invalid review timestamp"))
            })
            .transpose()?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    async fn fixture() -> (SqlitePool, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "arka-parent-{}-{}.db",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(true)
                    .foreign_keys(true),
            )
            .await
            .unwrap();
        super::super::database::run_migrations(&mut *pool.acquire().await.unwrap())
            .await
            .unwrap();
        (pool, path)
    }

    async fn close(pool: SqlitePool, path: std::path::PathBuf) {
        pool.close().await;
        std::fs::remove_file(path).unwrap();
    }

    fn paired() -> LearningItemInput {
        let mut input = from_mcq(
            QuestionInput {
                question: "Worst-case complexity?".into(),
                option_a: "O(1)".into(),
                option_b: "O(log n)".into(),
                option_c: "O(n)".into(),
                option_d: "O(n²)".into(),
                correct_answer: "B".into(),
                explanation: Some("Halve the interval".into()),
                space_id: 1,
            },
            "original-model".into(),
        )
        .unwrap();
        input.target = Some("Binary search worst-case complexity".into());
        input.generation.provider = Some("ollama".into());
        input.variants.push(VariantInput::Flashcard {
            prompt: "What is binary search's worst-case complexity?".into(),
        });
        input
    }

    fn editable(input: LearningItemInput) -> LearningItemEditInput {
        LearningItemEditInput {
            target: input.target,
            answer: input.answer,
            explanation: input.explanation,
            space_id: input.space_id,
            variants: input.variants,
        }
    }

    #[test]
    fn generated_save_materializes_content_with_backend_provenance() {
        let output: GeneratedItemsOutput = serde_json::from_str(include_str!(
            "../models/learning_item/fixtures/generated-pair.json"
        ))
        .unwrap();
        let generation = GenerationMetadata {
            model: Some(String::from("generation-model")),
            provider: Some(String::from("openai")),
            pipeline: Some(GenerationPipeline::Chunk),
            generated_at: Some(String::from("2026-09-09T10:00:00Z")),
        };
        let source = SourceReference {
            note_path: String::from("notes/algorithms.md"),
            start_line: 4,
            end_line: 12,
            knowledge_point: String::from("Binary search has logarithmic worst-case time."),
        };

        let input = from_generated(
            output.items.into_iter().next().unwrap(),
            7,
            generation.clone(),
            source.clone(),
            1,
        )
        .unwrap();

        assert_eq!(input.generation, generation);
        assert_eq!(input.source, Some(source));
        assert_eq!(
            input.target.as_deref(),
            Some("Binary search has O(log n) worst-case search time on a sorted array.")
        );
        assert!(matches!(input.variants[0], VariantInput::Flashcard { .. }));
        match &input.variants[1] {
            VariantInput::Mcq {
                options,
                correct_option_id,
                ..
            } => {
                assert_eq!(correct_option_id, "B");
                assert_eq!(options[1].text, "O(log n)");
            }
            _ => panic!("expected MCQ variant"),
        }
    }

    #[test]
    fn canonical_generated_save_rejects_non_chunk_provenance() {
        let output: GeneratedItemsOutput = serde_json::from_str(include_str!(
            "../models/learning_item/fixtures/generated-flashcard-only.json"
        ))
        .unwrap();
        let result = from_generated(
            output.items.into_iter().next().unwrap(),
            1,
            GenerationMetadata {
                model: Some(String::from("graph-model")),
                provider: Some(String::from("ollama")),
                pipeline: Some(GenerationPipeline::Graph),
                generated_at: None,
            },
            SourceReference {
                note_path: String::from("note.md"),
                start_line: 1,
                end_line: 2,
                knowledge_point: String::from("Point"),
            },
            0,
        );
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn variants_share_one_due_parent_and_reviews_write_no_history() {
        let (pool, path) = fixture().await;
        let item = save(&pool, vec![paired()]).await.unwrap().remove(0);
        assert_eq!(list(&pool, None, true).await.unwrap().len(), 1);
        assert_eq!(item.variants.len(), 2);
        let mcq_id = match &item.variants[0] {
            QuestionVariant::Mcq { id, .. } => *id,
            _ => panic!(),
        };
        let result = review(
            &pool,
            ReviewSubmission {
                learning_item_id: item.id,
                variant_id: mcq_id,
                response: ReviewResponse::Mcq {
                    selected_option_id: "B".into(),
                },
            },
        )
        .await
        .unwrap();
        assert_eq!(result.schedule.repetitions, 1);
        assert_eq!(result.schedule.interval_days, 1);
        assert!((result.schedule.ease_factor - 2.6).abs() < 1e-8);
        assert!(list(&pool, None, true).await.unwrap().is_empty());
        let flash_id = match &item.variants[1] {
            QuestionVariant::Flashcard { id, .. } => *id,
            _ => panic!(),
        };
        let result = review(
            &pool,
            ReviewSubmission {
                learning_item_id: item.id,
                variant_id: flash_id,
                response: ReviewResponse::Flashcard {
                    rating: ReviewRating::Again,
                },
            },
        )
        .await
        .unwrap();
        assert_eq!(result.schedule.repetitions, 0);
        assert_eq!(result.schedule.interval_days, 1);
        let events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM legacy_review_history")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(events, 0);
        close(pool, path).await;
    }

    #[tokio::test]
    async fn edit_preserves_schedule_provenance_and_variant_ids() {
        let (pool, path) = fixture().await;
        let mut original = paired();
        original.source = Some(SourceReference {
            note_path: String::from("notes/algorithms.md"),
            start_line: 10,
            end_line: 14,
            knowledge_point: String::from("Binary search halves the search interval."),
        });
        let item = save(&pool, vec![original]).await.unwrap().remove(0);
        let mut input = paired();
        input.answer = "O(log₂ n)".into();
        input.generation.model = Some("changed-settings".into());
        let changed = modify(&pool, item.id, editable(input)).await.unwrap();
        assert_eq!(changed.generation, item.generation);
        assert_eq!(changed.source, item.source);
        assert_eq!(changed.schedule, item.schedule);
        match (&item.variants[0], &changed.variants[0]) {
            (
                QuestionVariant::Mcq { id: old, .. },
                QuestionVariant::Mcq {
                    id,
                    options,
                    correct_option_id,
                    ..
                },
            ) => {
                assert_eq!(old, id);
                assert_eq!(
                    options
                        .iter()
                        .find(|o| &o.id == correct_option_id)
                        .unwrap()
                        .text,
                    "O(log₂ n)"
                );
            }
            _ => panic!(),
        }
        close(pool, path).await;
    }

    #[tokio::test]
    async fn invalid_batch_and_response_leave_database_unchanged() {
        let (pool, path) = fixture().await;
        let mut invalid = paired();
        invalid.answer = "O(n)".into();
        assert!(save(&pool, vec![paired(), invalid]).await.is_err());
        assert!(list(&pool, None, false).await.unwrap().is_empty());
        let item = save(&pool, vec![paired()]).await.unwrap().remove(0);
        let mcq_id = match item.variants[0] {
            QuestionVariant::Mcq { id, .. } => id,
            _ => panic!(),
        };
        for response in [
            ReviewResponse::Mcq {
                selected_option_id: "missing".into(),
            },
            ReviewResponse::Flashcard {
                rating: ReviewRating::Easy,
            },
        ] {
            assert!(review(
                &pool,
                ReviewSubmission {
                    learning_item_id: item.id,
                    variant_id: mcq_id,
                    response
                }
            )
            .await
            .is_err());
        }
        assert!(review(
            &pool,
            ReviewSubmission {
                learning_item_id: item.id,
                variant_id: 999,
                response: ReviewResponse::Mcq {
                    selected_option_id: "B".into()
                }
            }
        )
        .await
        .is_err());
        assert_eq!(
            load(&mut *pool.acquire().await.unwrap(), item.id)
                .await
                .unwrap()
                .schedule,
            item.schedule
        );
        close(pool, path).await;
    }

    #[tokio::test]
    async fn legacy_adapter_does_not_invent_flashcards_and_labels_follow_option_identity() {
        let (pool, path) = fixture().await;
        let mut input = paired();
        input.variants.truncate(1);
        input.target = None;
        if let VariantInput::Mcq {
            options,
            correct_option_id,
            ..
        } = &mut input.variants[0]
        {
            options[1].id = "stable-correct-id".into();
            *correct_option_id = "stable-correct-id".into();
        }
        let item = save(&pool, vec![input]).await.unwrap().remove(0);
        assert!(!item
            .variants
            .iter()
            .any(|variant| matches!(variant, QuestionVariant::Flashcard { .. })));
        let question = to_mcq(item).unwrap().unwrap();
        assert_eq!(question.correct_answer, "B");
        assert_eq!(question.model.as_deref(), Some("original-model"));
        close(pool, path).await;
    }

    #[tokio::test]
    async fn flashcard_only_items_support_all_ratings_and_space_filtering() {
        let (pool, path) = fixture().await;
        sqlx::query("INSERT INTO recall_spaces(id,name) VALUES(2,'Other')")
            .execute(&pool)
            .await
            .unwrap();
        for (rating, expected_repetitions) in [
            (ReviewRating::Again, 0),
            (ReviewRating::Hard, 1),
            (ReviewRating::Good, 1),
            (ReviewRating::Easy, 1),
        ] {
            let mut input = paired();
            input.variants.remove(0);
            input.space_id = 2;
            let item = save(&pool, vec![input]).await.unwrap().remove(0);
            assert!(to_mcq(item.clone()).unwrap().is_none());
            let id = match item.variants[0] {
                QuestionVariant::Flashcard { id, .. } => id,
                _ => panic!(),
            };
            let result = review(
                &pool,
                ReviewSubmission {
                    learning_item_id: item.id,
                    variant_id: id,
                    response: ReviewResponse::Flashcard { rating },
                },
            )
            .await
            .unwrap();
            assert_eq!(result.schedule.repetitions, expected_repetitions);
        }
        assert!(list(&pool, Some(1), false).await.unwrap().is_empty());
        assert_eq!(list(&pool, Some(2), false).await.unwrap().len(), 4);
        close(pool, path).await;
    }
}
