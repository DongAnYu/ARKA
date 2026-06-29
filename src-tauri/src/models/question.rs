use serde::Serialize;
use sqlx::FromRow;

#[derive(Debug, Serialize, FromRow)]
pub struct Question {
	pub id: i64,
	pub question: String,
	pub option_a: String,
	pub option_b: String,
	pub option_c: String,
	pub option_d: String,
	pub correct_answer: String,
	pub explanation: Option<String>,
}
