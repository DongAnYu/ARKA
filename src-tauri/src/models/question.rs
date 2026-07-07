use serde::{Deserialize, Serialize};
use sqlx::FromRow;

fn default_space_id() -> i64 {
	1
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct Question {
	pub id: i64,
	pub question: String,
	pub option_a: String,
	pub option_b: String,
	pub option_c: String,
	pub option_d: String,
	pub correct_answer: String,
	pub explanation: Option<String>,
	pub model: Option<String>,
	pub space_id: i64,
}

#[derive(Debug, Deserialize)]
pub struct QuestionInput {
	pub question: String,
	pub option_a: String,
	pub option_b: String,
	pub option_c: String,
	pub option_d: String,
	pub correct_answer: String,
	pub explanation: Option<String>,
	#[serde(default = "default_space_id")]
	pub space_id: i64,
}
