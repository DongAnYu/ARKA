use super::learning_item::LearningItem;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StudyPreferences {
    pub daily_target: i64,
    pub max_new_items: i64,
}

#[derive(Debug, Serialize)]
pub struct PlannedItem {
    pub was_new: bool,
    pub is_extra: bool,
    pub item: LearningItem,
}

#[derive(Debug, Serialize)]
pub struct DailyStudyPlan {
    pub local_date: String,
    pub daily_target: i64,
    pub max_new_items: i64,
    pub space_id: Option<i64>,
    pub completed_count: i64,
    pub total_count: i64,
    pub extra_completed_count: i64,
    pub can_study_more: bool,
    pub items: Vec<PlannedItem>,
}
