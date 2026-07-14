use crate::models::question::Question;
use chrono::{Duration, Utc};
use std::str::FromStr;

#[allow(dead_code)]
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rating {
    Again = 2,
    Hard = 3,
    Good = 4,
    Easy = 5,
}

impl FromStr for Rating {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "again" => Ok(Rating::Again),
            "hard" => Ok(Rating::Hard),
            "good" => Ok(Rating::Good),
            "easy" => Ok(Rating::Easy),
            _ => Err(format!("Unsupported rating: {value}")),
        }
    }
}

impl Rating {
    fn quality(self) -> i32 {
        self as i32
    }
}

#[allow(dead_code)]
pub struct SM2Scheduler;

/// Implements the SuperMemo-2 spaced repetition algorithm.
/// NOTE:
/// This function currently mutates the `Question` model directly for simplicity.
///
/// Future refactor:
/// - Decouple the scheduler from the database model.
/// - Accept a lightweight `ReviewState` instead of `Question`.
/// - Return a `ReviewResult` containing only scheduling fields.
/// - Let a higher-level `ReviewService` apply the result and persist changes.
///
/// This separation will:
/// - Keep the SM-2 scheduler independent of SQLx/Tauri.
/// - Make unit testing easier.
/// - Allow the scheduler to be reused for other review item types
///   (e.g. concepts, flashcards, cloze cards).
impl SM2Scheduler {
    #[allow(dead_code)]
    pub fn review_question(question: &mut Question, rating: Rating) {
        let now = Utc::now().naive_utc();
        let quality = rating.quality();
        let mut repetitions = question.repetitions;
        let mut interval_days = question.interval_days;
        let mut ease_factor = f64::from(question.ease_factor);

        if quality < 3 {
            repetitions = 0;
            interval_days = 1;
        } else {
            repetitions += 1;

            if repetitions == 1 {
                interval_days = 1;
            } else if repetitions == 2 {
                interval_days = 6;
            } else {
                interval_days = (f64::from(interval_days) * ease_factor).round() as i32;
            }
        }

        let diff = 5 - quality;
        ease_factor += 0.1 - f64::from(diff) * (0.08 + f64::from(diff) * 0.02);

        if ease_factor < 1.3 {
            ease_factor = 1.3;
        }

        question.repetitions = repetitions;
        question.interval_days = interval_days;
        question.ease_factor = ease_factor;
        question.last_reviewed_at = Some(now);
        question.next_review_at = Some(now + Duration::days(interval_days as i64));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn question_with_schedule(repetitions: i32, interval_days: i32, ease_factor: f64) -> Question {
        Question {
            id: 1,
            question: "Question?".to_string(),
            option_a: "A".to_string(),
            option_b: "B".to_string(),
            option_c: "C".to_string(),
            option_d: "D".to_string(),
            correct_answer: "A".to_string(),
            explanation: None,
            model: Some("test".to_string()),
            space_id: 1,
            repetitions,
            interval_days,
            ease_factor,
            next_review_at: None,
            last_reviewed_at: None,
        }
    }

    #[test]
    fn good_reviews_follow_sm2_interval_progression() {
        let mut question = question_with_schedule(0, 0, 2.5);

        SM2Scheduler::review_question(&mut question, Rating::Good);
        assert_eq!(question.repetitions, 1);
        assert_eq!(question.interval_days, 1);
        assert_eq!(question.ease_factor, 2.5);

        SM2Scheduler::review_question(&mut question, Rating::Good);
        assert_eq!(question.repetitions, 2);
        assert_eq!(question.interval_days, 6);

        SM2Scheduler::review_question(&mut question, Rating::Good);
        assert_eq!(question.repetitions, 3);
        assert_eq!(question.interval_days, 15);

        SM2Scheduler::review_question(&mut question, Rating::Good);
        assert_eq!(question.repetitions, 4);
        assert_eq!(question.interval_days, 38);
    }

    #[test]
    fn rating_updates_ease_with_sm2_formula() {
        const FLOAT_TOLERANCE: f64 = 1e-10;

        let mut easy = question_with_schedule(0, 0, 2.5);
        SM2Scheduler::review_question(&mut easy, Rating::Easy);
        assert!((easy.ease_factor - 2.6).abs() < f64::EPSILON);

        let mut hard = question_with_schedule(0, 0, 2.5);
        SM2Scheduler::review_question(&mut hard, Rating::Hard);
        assert!((hard.ease_factor - 2.36).abs() < f64::EPSILON);

        let mut again = question_with_schedule(3, 12, 2.5);
        SM2Scheduler::review_question(&mut again, Rating::Again);
        assert!((again.ease_factor - 2.18).abs() < FLOAT_TOLERANCE);
    }

    #[test]
    fn again_resets_repetitions_and_ease_never_drops_below_minimum() {
        let mut question = question_with_schedule(3, 12, 1.35);

        SM2Scheduler::review_question(&mut question, Rating::Again);

        assert_eq!(question.repetitions, 0);
        assert_eq!(question.interval_days, 1);
        assert_eq!(question.ease_factor, 1.3);
        assert!(question.last_reviewed_at.is_some());
        assert!(question.next_review_at.is_some());
    }
}
