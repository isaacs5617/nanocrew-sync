use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("Authentication required")]
    Unauthenticated,

    #[error("Invalid credentials")]
    InvalidCredentials,

    #[error("Account already exists")]
    AlreadyExists,

    #[error("Drive not found")]
    DriveNotFound,

    #[error("Drive is already mounted")]
    AlreadyMounted,

    #[error("Unmount the drive before removing it")]
    DriveStillMounted,

    #[error("Credential store error: {0}")]
    Keyring(String),

    #[error("Mount error: {0}")]
    Mount(String),

    #[error("Connection test failed: {0}")]
    ConnectionTest(String),

    #[error("Password hashing error: {0}")]
    PasswordHash(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<AppError> for String {
    fn from(e: AppError) -> String {
        e.to_string()
    }
}
