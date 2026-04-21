use crate::{error::AppError, state::AppState};

/// Validates token against the in-memory session map.
/// Returns account_id on success.
pub fn require_auth(state: &AppState, token: &str) -> Result<i64, AppError> {
    let sessions = state.sessions.lock().unwrap_or_else(|p| p.into_inner());
    sessions
        .get(token)
        .map(|s| s.account_id)
        .ok_or(AppError::Unauthenticated)
}
