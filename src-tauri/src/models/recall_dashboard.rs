use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct RecallDashboard {
    pub due_today_count: i64,
    pub overdue_count: i64,
    pub reviewed_today_count: i64,
    pub correct_today_count: i64,
    pub spaces: Vec<RecallSpaceSummary>,
}

#[derive(Debug, Serialize)]
pub struct RecallSpaceSummary {
    pub id: i64,
    pub name: String,
    pub total_questions: i64,
    pub due_count: i64,
    pub overdue_count: i64,
    pub reviewed_today_count: i64,
    pub correct_today_count: i64,
}
