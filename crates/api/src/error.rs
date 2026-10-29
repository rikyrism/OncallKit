use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

pub type AppResult<T> = Result<T, AppError>;

/// Errors returned by the HTTP API. Every variant renders as
/// `{"error": {"code": "...", "message": "..."}}` and internal details never leave the server.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(String),
    #[error("{message}")]
    Validation { field: Option<String>, message: String },
    #[error("authentication required")]
    Unauthenticated,
    #[error("{0}")]
    Forbidden(String),
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    Conflict(String),
    #[error("too many requests")]
    RateLimited,
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl AppError {
    pub fn validation(field: &str, message: impl Into<String>) -> Self {
        AppError::Validation { field: Some(field.into()), message: message.into() }
    }

    pub fn forbidden() -> Self {
        AppError::Forbidden("you do not have permission to do this".into())
    }
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        if let sqlx::Error::RowNotFound = err {
            return AppError::NotFound;
        }
        if let Some(db) = err.as_database_error() {
            if db.is_unique_violation() {
                return AppError::Conflict("a record with these details already exists".into());
            }
            if db.is_foreign_key_violation() {
                return AppError::BadRequest("a referenced record does not exist or is still in use".into());
            }
            if db.is_check_violation() {
                return AppError::BadRequest("the request contains an invalid value".into());
            }
        }
        AppError::Internal(err.into())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            AppError::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            AppError::Validation { .. } => (StatusCode::UNPROCESSABLE_ENTITY, "validation_failed"),
            AppError::Unauthenticated => (StatusCode::UNAUTHORIZED, "unauthenticated"),
            AppError::Forbidden(_) => (StatusCode::FORBIDDEN, "forbidden"),
            AppError::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            AppError::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
            AppError::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
            AppError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
        };
        let message = match &self {
            AppError::Internal(err) => {
                tracing::error!(error = %format!("{err:#}"), "internal error");
                "something went wrong on our side".to_string()
            }
            other => other.to_string(),
        };
        let field = match &self {
            AppError::Validation { field, .. } => field.clone(),
            _ => None,
        };
        (status, Json(json!({ "error": { "code": code, "message": message, "field": field } }))).into_response()
    }
}
