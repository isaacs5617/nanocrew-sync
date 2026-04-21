use std::sync::Mutex;
use rusqlite::Connection;
use crate::error::AppError;

pub fn store(db: &Mutex<Connection>, drive_id: i64, secret: &str) -> Result<(), AppError> {
    let conn = db.lock().unwrap_or_else(|p| p.into_inner());
    conn.execute(
        "UPDATE drives SET secret_key = ?1 WHERE id = ?2",
        rusqlite::params![secret, drive_id],
    ).map_err(AppError::Db)?;
    Ok(())
}

pub fn retrieve(db: &Mutex<Connection>, drive_id: i64) -> Result<String, AppError> {
    let conn = db.lock().unwrap_or_else(|p| p.into_inner());
    let secret: String = conn
        .query_row(
            "SELECT secret_key FROM drives WHERE id = ?1",
            rusqlite::params![drive_id],
            |r| r.get(0),
        )
        .map_err(|_| AppError::Keyring(
            "No stored credential found for this drive. Remove and re-add the drive to restore access.".into(),
        ))?;
    if secret.is_empty() {
        return Err(AppError::Keyring(
            "No stored credential found for this drive. Remove and re-add the drive to restore access.".into(),
        ));
    }
    Ok(secret)
}

// Secret is stored in the drive row and is deleted automatically when the row is deleted.
pub fn delete(_db: &Mutex<Connection>, _drive_id: i64) -> Result<(), AppError> {
    Ok(())
}
