use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("kubernetes error: {0}")]
    KubeError(#[from] kube::Error),

    #[error("internal error: {0}")]
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let request_id = Uuid::new_v4().to_string();

        let (status, code, message) = match &self {
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, "not_found", msg.clone()),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, "conflict", msg.clone()),
            AppError::KubeError(e) => {
                tracing::error!(request_id = %request_id, error = %e, "Kubernetes API error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "kube_error",
                    e.to_string(),
                )
            }
            AppError::Internal(msg) => {
                tracing::error!(request_id = %request_id, error = %msg, "Internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    msg.clone(),
                )
            }
        };

        let body = json!({
            "error": {
                "code": code,
                "message": message,
                "request_id": request_id,
            }
        });

        (status, axum::Json(body)).into_response()
    }
}
