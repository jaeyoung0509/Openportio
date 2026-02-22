use axum::{http::StatusCode, Json};
use openportio_server::api::{ApiError, ApiErrorResponse};

pub(crate) fn not_found(message: String) -> ApiError {
    (
        StatusCode::NOT_FOUND,
        Json(ApiErrorResponse {
            code: "not_found".to_string(),
            message,
            detail: None,
            details: None,
        }),
    )
}

pub(crate) fn readiness_error(err: sqlx::Error) -> ApiError {
    tracing::warn!(error = %err, "readiness check failed");
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(ApiErrorResponse {
            code: "not_ready".to_string(),
            message: "database is unavailable".to_string(),
            detail: None,
            details: None,
        }),
    )
}

pub(crate) fn database_error(err: sqlx::Error) -> ApiError {
    tracing::error!(error = %err, "database operation failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiErrorResponse::internal_server_error()),
    )
}
