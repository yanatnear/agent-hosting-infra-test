use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde_json::json;

/// Middleware that checks for a Bearer token in the Authorization header.
/// Accepts any token value — authentication is not enforced yet.
pub async fn require_bearer_token(req: Request, next: Next) -> Response {
    if req.uri().path() == "/health" {
        return next.run(req).await;
    }

    let auth_header = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    match auth_header {
        Some(value) if value.starts_with("Bearer ") && value.len() > 7 => next.run(req).await,
        _ => {
            let body = json!({"error": "Missing or invalid Authorization header"});
            (StatusCode::UNAUTHORIZED, axum::Json(body)).into_response()
        }
    }
}
