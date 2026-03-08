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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;
    use http_body_util::BodyExt;

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    /// Converts an AppError into its HTTP response, extracts the status code
    /// and parses the body as JSON. This helper avoids duplicating the
    /// response-extraction boilerplate across every test.
    async fn error_to_json(err: AppError) -> (StatusCode, serde_json::Value) {
        let response = err.into_response();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes)
            .expect("error response body must be valid JSON");
        (status, json)
    }

    // -----------------------------------------------------------------------
    // Individual variant tests
    // -----------------------------------------------------------------------

    /// **Test Cases #16, #19 — NotFound produces 404 with correct body**
    ///
    /// WHY THIS MATTERS:
    /// 404 responses are the primary signal to CLI/UI that an agent doesn't
    /// exist. The response must include structured JSON with the error code,
    /// message, and request_id for debugging and programmatic handling.
    ///
    /// WHAT THIS TEST DOES:
    /// 1. Creates a NotFound error with a test message
    /// 2. Converts to HTTP response
    /// 3. Asserts status=404, code="not_found", message matches input
    /// 4. Asserts request_id is present (UUID)
    ///
    /// IF THIS FAILS:
    /// 404 errors may return wrong status codes or malformed JSON, breaking
    /// error handling in the CLI and web UI.
    ///
    /// WHAT IS BEING TESTED:
    /// `AppError::NotFound` IntoResponse impl — pure conversion.
    #[tokio::test]
    async fn p1_not_found_produces_404_with_structured_json() {
        let message = "instance 'ghost-agent' not found";

        let (status, json) = error_to_json(AppError::NotFound(message.to_string())).await;

        assert_eq!(status, StatusCode::NOT_FOUND, "NotFound must return 404");
        assert_eq!(
            json["error"]["code"].as_str(),
            Some("not_found"),
            "error code must be 'not_found'"
        );
        assert_eq!(
            json["error"]["message"].as_str(),
            Some(message),
            "error message must match the input"
        );
        assert!(
            json["error"]["request_id"].as_str().is_some(),
            "response must include a request_id for debugging"
        );
    }

    /// **Test Case #6 — Conflict produces 409 with correct body**
    ///
    /// WHY THIS MATTERS:
    /// 409 Conflict is returned when creating a duplicate agent. The CLI must
    /// be able to distinguish "already exists" from other errors to provide
    /// helpful user guidance.
    ///
    /// WHAT THIS TEST DOES:
    /// 1. Creates a Conflict error
    /// 2. Converts to HTTP response
    /// 3. Asserts status=409, code="conflict"
    ///
    /// IF THIS FAILS:
    /// Duplicate creation attempts may return 500 instead of 409, confusing
    /// users and breaking idempotency checks in automation scripts.
    ///
    /// WHAT IS BEING TESTED:
    /// `AppError::Conflict` IntoResponse impl — pure conversion.
    #[tokio::test]
    async fn p1_conflict_produces_409_with_structured_json() {
        let message = "instance 'duplicate-agent' already exists";

        let (status, json) = error_to_json(AppError::Conflict(message.to_string())).await;

        assert_eq!(status, StatusCode::CONFLICT, "Conflict must return 409");
        assert_eq!(
            json["error"]["code"].as_str(),
            Some("conflict"),
            "error code must be 'conflict'"
        );
        assert_eq!(
            json["error"]["message"].as_str(),
            Some(message),
            "error message must match the input"
        );
    }

    /// **Test Case #22 — Internal error produces 500**
    ///
    /// WHY THIS MATTERS:
    /// Internal errors represent unexpected server failures. They must return
    /// 500 with enough detail for operators to debug via the request_id,
    /// without leaking sensitive implementation details.
    ///
    /// WHAT THIS TEST DOES:
    /// 1. Creates an Internal error
    /// 2. Converts to HTTP response
    /// 3. Asserts status=500, code="internal_error"
    ///
    /// IF THIS FAILS:
    /// Internal errors may return wrong status codes, causing monitoring
    /// systems to miscategorize server failures.
    ///
    /// WHAT IS BEING TESTED:
    /// `AppError::Internal` IntoResponse impl — pure conversion.
    #[tokio::test]
    async fn p2_internal_error_produces_500_with_structured_json() {
        let (status, json) =
            error_to_json(AppError::Internal("something broke".to_string())).await;

        assert_eq!(
            status,
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal must return 500"
        );
        assert_eq!(
            json["error"]["code"].as_str(),
            Some("internal_error"),
            "error code must be 'internal_error'"
        );
    }

    /// **Test Case #22 — KubeError produces 500**
    ///
    /// WHY THIS MATTERS:
    /// Kubernetes API errors bubble up as KubeError variants. They must be
    /// wrapped in the same structured JSON format as other errors so that
    /// clients have a consistent error-handling path.
    ///
    /// WHAT THIS TEST DOES:
    /// 1. Creates a KubeError from a synthetic Kubernetes API error
    /// 2. Converts to HTTP response
    /// 3. Asserts status=500, code="kube_error"
    ///
    /// IF THIS FAILS:
    /// Kubernetes errors will produce unstructured responses, breaking
    /// programmatic error handling in CLI and automation tools.
    ///
    /// WHAT IS BEING TESTED:
    /// `AppError::KubeError` IntoResponse impl — pure conversion.
    #[tokio::test]
    async fn p2_kube_error_produces_500_with_structured_json() {
        let kube_err = kube::Error::Api(kube::error::ErrorResponse {
            code: 500,
            message: "etcd timeout".to_string(),
            reason: "InternalError".to_string(),
            status: "Failure".to_string(),
        });

        let (status, json) = error_to_json(AppError::KubeError(kube_err)).await;

        assert_eq!(
            status,
            StatusCode::INTERNAL_SERVER_ERROR,
            "KubeError must return 500"
        );
        assert_eq!(
            json["error"]["code"].as_str(),
            Some("kube_error"),
            "error code must be 'kube_error'"
        );
    }

    /// **Test Case #22 — All error variants share consistent JSON structure**
    ///
    /// WHY THIS MATTERS:
    /// Clients rely on a consistent error envelope `{error: {code, message,
    /// request_id}}` for programmatic error handling. If any variant deviates,
    /// client-side parsing fails for that specific error type.
    ///
    /// WHAT THIS TEST DOES:
    /// 1. Creates one instance of every AppError variant
    /// 2. Converts each to HTTP response JSON
    /// 3. Asserts all responses have the exact same top-level structure
    ///
    /// IF THIS FAILS:
    /// One or more error variants produce a different JSON shape, breaking
    /// the error handling contract for API consumers.
    ///
    /// WHAT IS BEING TESTED:
    /// IntoResponse impl consistency across all variants — pure conversion.
    #[tokio::test]
    async fn p2_all_error_variants_have_consistent_json_structure() {
        let variants: Vec<AppError> = vec![
            AppError::NotFound("test".to_string()),
            AppError::Conflict("test".to_string()),
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

            let error_obj = json.get("error").unwrap_or_else(|| {
                panic!("variant {} must have top-level 'error' key", variant_name)
            });
            assert!(
                error_obj.get("code").and_then(|v| v.as_str()).is_some(),
                "variant {} must have error.code as string",
                variant_name
            );
            assert!(
                error_obj.get("message").and_then(|v| v.as_str()).is_some(),
                "variant {} must have error.message as string",
                variant_name
            );
            assert!(
                error_obj
                    .get("request_id")
                    .and_then(|v| v.as_str())
                    .is_some(),
                "variant {} must have error.request_id as string",
                variant_name
            );
        }
    }
}
