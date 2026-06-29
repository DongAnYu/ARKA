use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Note {
  pub id: Option<i64>,
  pub path: String,
  pub title: String,
  pub content: String,
  pub last_modified: String,
}
