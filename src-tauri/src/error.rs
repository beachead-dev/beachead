use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum OrchestratorError {
    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Duplicate name: {0}")]
    DuplicateName(String),

    #[error("Cannot delete: has dependent resources ({0})")]
    HasDependents(String),

    #[error("Cannot delete: has active sessions")]
    ActiveSessions,

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("sbx CLI is not available: {0}")]
    SbxUnavailable(String),

    #[error("sbx CLI error: {0}")]
    SbxError(String),

    #[error("Operation timed out: {0}")]
    SbxTimeout(String),

    #[error("Docker error: {0}")]
    DockerError(String),

    #[error("Missing credentials: {0}")]
    MissingCredentials(String),

    #[error("Port exhaustion: no available ports in configured range")]
    PortExhaustion,

    #[error("Workspace not found: {0}")]
    WorkspaceNotFound(String),

    #[error("Decryption failed: {0}")]
    DecryptionFailed(String),

    #[error("Database error: {0}")]
    Database(String),

    #[error("PTY error: {0}")]
    PtyError(String),

    #[error("Sync operation already in progress: {0}")]
    SyncInProgress(String),

    #[error("Git binary not found: {0}")]
    GitNotFound(String),

    #[error("Git operation timed out: {0}")]
    GitTimeout(String),

    #[error("Git authentication failed: {0}")]
    GitAuthFailure(String),

    #[error("Merge conflict: {0}")]
    MergeConflict(String),

    #[error("Keyring unavailable: {0}")]
    KeyringUnavailable(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

#[derive(Serialize)]
struct ErrorResponse {
    error: ErrorBody,
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
}

impl OrchestratorError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::Validation(_) => StatusCode::BAD_REQUEST,
            Self::DuplicateName(_) => StatusCode::CONFLICT,
            Self::HasDependents(_) => StatusCode::CONFLICT,
            Self::ActiveSessions => StatusCode::CONFLICT,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::SbxError(_) => StatusCode::BAD_GATEWAY,
            Self::SbxUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::SbxTimeout(_) => StatusCode::GATEWAY_TIMEOUT,
            Self::DockerError(_) => StatusCode::BAD_GATEWAY,
            Self::MissingCredentials(_) => StatusCode::PRECONDITION_FAILED,
            Self::PortExhaustion => StatusCode::SERVICE_UNAVAILABLE,
            Self::WorkspaceNotFound(_) => StatusCode::BAD_REQUEST,
            Self::DecryptionFailed(_) => StatusCode::BAD_REQUEST,
            Self::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::PtyError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::SyncInProgress(_) => StatusCode::CONFLICT,
            Self::GitNotFound(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::GitTimeout(_) => StatusCode::REQUEST_TIMEOUT,
            Self::GitAuthFailure(_) => StatusCode::UNAUTHORIZED,
            Self::MergeConflict(_) => StatusCode::CONFLICT,
            Self::KeyringUnavailable(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_code(&self) -> &'static str {
        match self {
            Self::Validation(_) => "VALIDATION_ERROR",
            Self::DuplicateName(_) => "DUPLICATE_NAME",
            Self::HasDependents(_) => "HAS_DEPENDENTS",
            Self::ActiveSessions => "ACTIVE_SESSIONS",
            Self::NotFound(_) => "NOT_FOUND",
            Self::SbxError(_) => "SBX_ERROR",
            Self::SbxUnavailable(_) => "SBX_UNAVAILABLE",
            Self::SbxTimeout(_) => "SBX_TIMEOUT",
            Self::DockerError(_) => "DOCKER_ERROR",
            Self::MissingCredentials(_) => "MISSING_CREDENTIALS",
            Self::PortExhaustion => "PORT_EXHAUSTION",
            Self::WorkspaceNotFound(_) => "WORKSPACE_NOT_FOUND",
            Self::DecryptionFailed(_) => "DECRYPTION_FAILED",
            Self::Database(_) => "DATABASE_ERROR",
            Self::PtyError(_) => "PTY_ERROR",
            Self::SyncInProgress(_) => "SYNC_IN_PROGRESS",
            Self::GitNotFound(_) => "GIT_NOT_FOUND",
            Self::GitTimeout(_) => "GIT_TIMEOUT",
            Self::GitAuthFailure(_) => "GIT_AUTH_FAILURE",
            Self::MergeConflict(_) => "MERGE_CONFLICT",
            Self::KeyringUnavailable(_) => "KEYRING_UNAVAILABLE",
            Self::Internal(_) => "INTERNAL_ERROR",
        }
    }
}

impl IntoResponse for OrchestratorError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let body = ErrorResponse {
            error: ErrorBody {
                code: self.error_code(),
                message: self.to_string(),
            },
        };

        (status, axum::Json(body)).into_response()
    }
}

impl From<rusqlite::Error> for OrchestratorError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Database(err.to_string())
    }
}

impl From<std::io::Error> for OrchestratorError {
    fn from(err: std::io::Error) -> Self {
        Self::Internal(err.to_string())
    }
}
