//! Structured error type shared by every Tauri command.
//!
//! Errors are serialized to the frontend as `{ code, message, remediation, details }`
//! so the UI can render actionable dialogs instead of opaque strings.

use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StructuredError {
    pub code: String,
    pub message: String,
    pub remediation: Option<String>,
    pub details: Option<Value>,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct AppError {
    pub code: String,
    pub message: String,
    pub remediation: Option<String>,
    pub details: Option<Value>,
}

impl AppError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            remediation: None,
            details: None,
        }
    }

    pub fn with_remediation(mut self, remediation: impl Into<String>) -> Self {
        self.remediation = Some(remediation.into());
        self
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::new("VALIDATION_FAILED", message)
            .with_remediation("Review the highlighted fields and try again.")
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new("NOT_FOUND", message)
    }

    pub fn io(message: impl Into<String>) -> Self {
        Self::new("IO_ERROR", message).with_remediation("Check that the file exists and is writable.")
    }

    pub fn db(message: impl Into<String>) -> Self {
        Self::new("DATABASE_ERROR", message)
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new("UNSUPPORTED", message)
            .with_remediation("Use a supported option for this harness or configure a generic JSON profile.")
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new("CONFLICT", message)
            .with_remediation("Resolve the conflict in the preview dialog before writing.")
    }

    pub fn permission_denied(message: impl Into<String>) -> Self {
        Self::new("PERMISSION_DENIED", message)
            .with_remediation("Universal MCP Manager only writes to the configuration locations it detected.")
    }

    pub fn file_changed(message: impl Into<String>) -> Self {
        Self::new("FILE_CHANGED_ON_DISK", message)
            .with_remediation("Reopen the preview to see the external change, then try again.")
    }

    pub fn locked(message: impl Into<String>) -> Self {
        Self::new("FILE_LOCKED", message)
            .with_remediation("Close the application that owns the file (or the editor holding it) and retry.")
    }

    pub fn secret_store(message: impl Into<String>) -> Self {
        Self::new("SECRET_STORE_ERROR", message)
            .with_remediation("Windows Credential Manager must be reachable for secret storage. Literal values still work.")
    }

    pub fn to_payload(&self) -> StructuredError {
        StructuredError {
            code: self.code.clone(),
            message: self.message.clone(),
            remediation: self.remediation.clone(),
            details: self.details.clone(),
        }
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_payload().serialize(serializer)
    }
}

impl From<std::io::Error> for AppError {
    fn from(value: std::io::Error) -> Self {
        let code = match value.kind() {
            std::io::ErrorKind::PermissionDenied => "PERMISSION_DENIED",
            std::io::ErrorKind::NotFound => "NOT_FOUND",
            _ => "IO_ERROR",
        };
        let mut error = AppError::new(code, format!("I/O error: {value}"));
        if value.kind() == std::io::ErrorKind::PermissionDenied {
            error = error.with_remediation(
                "The file may be open in another application or the directory may be read-only.",
            );
        }
        error
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(value: rusqlite::Error) -> Self {
        AppError::db(format!("SQLite error: {value}"))
    }
}

impl From<serde_json::Error> for AppError {
    fn from(value: serde_json::Error) -> Self {
        AppError::validation(format!("JSON error: {value}"))
    }
}

impl From<url::ParseError> for AppError {
    fn from(value: url::ParseError) -> Self {
        AppError::validation(format!("Invalid URL: {value}"))
    }
}

pub type AppResult<T> = Result<T, AppError>;
