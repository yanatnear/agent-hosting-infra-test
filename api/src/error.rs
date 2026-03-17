use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("kubernetes error: {0}")]
    KubeError(#[from] kube::Error),

    #[error("internal error: {0}")]
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, msg.clone()),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            AppError::KubeError(e) => {
                tracing::error!(error = %e, "Kubernetes API error");
                (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
            }
            AppError::Internal(msg) => {
                tracing::error!(error = %msg, "Internal error");
                (StatusCode::INTERNAL_SERVER_ERROR, msg.clone())
            }
        };

        let body = json!({ "error": message });
        (status, axum::Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;
    use http_body_util::BodyExt;

    async fn error_to_json(err: AppError) -> (StatusCode, serde_json::Value) {
        let response = err.into_response();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes)
            .expect("error response body must be valid JSON");
        (status, json)
    }

    #[tokio::test]
    async fn p1_not_found_produces_404() {
        let message = "instance 'ghost-agent' not found";
        let (status, json) = error_to_json(AppError::NotFound(message.to_string())).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json["error"].as_str(), Some(message));
    }

    #[tokio::test]
    async fn p1_conflict_produces_409() {
        let message = "instance 'duplicate-agent' already exists";
        let (status, json) = error_to_json(AppError::Conflict(message.to_string())).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(json["error"].as_str(), Some(message));
    }

    #[tokio::test]
    async fn p1_bad_request_produces_400() {
        let message = "missing required field: nearai_api_key";
        let (status, json) = error_to_json(AppError::BadRequest(message.to_string())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"].as_str(), Some(message));
    }

    #[tokio::test]
    async fn p2_internal_error_produces_500() {
        let (status, json) =
            error_to_json(AppError::Internal("something broke".to_string())).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(json["error"].as_str().is_some());
    }

    #[tokio::test]
    async fn p2_kube_error_produces_500() {
        let kube_err = kube::Error::Api(kube::error::ErrorResponse {
            code: 500,
            message: "etcd timeout".to_string(),
            reason: "InternalError".to_string(),
            status: "Failure".to_string(),
        });
        let (status, json) = error_to_json(AppError::KubeError(kube_err)).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(json["error"].as_str().is_some());
    }

    #[tokio::test]
    async fn p2_all_error_variants_produce_flat_error_string() {
        let variants: Vec<AppError> = vec![
            AppError::NotFound("test".to_string()),
            AppError::Conflict("test".to_string()),
            AppError::BadRequest("test".to_string()),
            AppError::Internal("test".to_string()),
            AppError::KubeError(kube::Error::Api(kube::error::ErrorResponse {
                code: 500,
                message: "test".to_string(),
                reason: "test".to_string(),
                status: "Failure".to_string(),
            })),
        ];

        for err in variants {
            let variant_name = format!("{:?}", err);
            let (_status, json) = error_to_json(err).await;
            assert!(
                json["error"].as_str().is_some(),
                "variant {} must have 'error' as a string",
                variant_name
            );
        }
    }
}
