//! The one error type that crosses the IPC boundary (docs/IPC.md).

use serde::{Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("network: {0}")]
    Network(String),
    #[error("auth: {0}")]
    Auth(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("server: {0}")]
    Server(String),
    #[error("io: {0}")]
    Io(String),
    #[error("decode: {0}")]
    Decode(String),
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("internal: {0}")]
    Internal(String),
}

impl AppError {
    pub fn kind(&self) -> &'static str {
        match self {
            AppError::Network(_) => "network",
            AppError::Auth(_) => "auth",
            AppError::NotFound(_) => "notFound",
            AppError::Server(_) => "server",
            AppError::Io(_) => "io",
            AppError::Decode(_) => "decode",
            AppError::Invalid(_) => "invalid",
            AppError::Internal(_) => "internal",
        }
    }

    fn message(&self) -> String {
        match self {
            AppError::Network(m)
            | AppError::Auth(m)
            | AppError::NotFound(m)
            | AppError::Server(m)
            | AppError::Io(m)
            | AppError::Decode(m)
            | AppError::Invalid(m)
            | AppError::Internal(m) => m.clone(),
        }
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("AppError", 2)?;
        st.serialize_field("kind", self.kind())?;
        st.serialize_field("message", &self.message())?;
        st.end()
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Io(e.to_string())
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        AppError::Internal(format!("db: {e}"))
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_kind_and_message() {
        let v = serde_json::to_value(AppError::NotFound("album 1".into())).unwrap();
        assert_eq!(v["kind"], "notFound");
        assert_eq!(v["message"], "album 1");
    }
}
