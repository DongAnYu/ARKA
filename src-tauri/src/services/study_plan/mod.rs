//! Daily study-plan preferences, queue selection, and review transactions.
//! The public entry points remain shared by Tauri commands and learning-item reviews.

mod preferences;
mod queue;
mod review;
pub mod self_directed;
mod state;

// Evaluation targets compile these services without the desktop command callers.
#[allow(unused_imports)]
pub use preferences::{preferences, save_preferences};
#[allow(unused_imports)]
pub use queue::get_plan;
pub(super) use review::record_event;
#[allow(unused_imports)]
pub use review::review;
pub use state::local_day;

fn invalid(message: &str) -> sqlx::Error {
    sqlx::Error::Protocol(message.into())
}

#[cfg(test)]
mod tests;
