use axum::routing::{delete, get, post};
use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

use agent_api::handlers;
use agent_api::handlers::AppState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    tracing::info!("Initializing Kubernetes client");
    let client = kube::Client::try_default().await.map_err(|e| {
        tracing::error!(error = %e, "Failed to create Kubernetes client");
        e
    })?;

    let state = AppState { client };

    let app = Router::new()
        .route("/health", get(handlers::health))
        .route("/instances", post(handlers::create_instance))
        .route("/instances", get(handlers::list_instances))
        .route("/instances/{name}", get(handlers::get_instance))
        .route("/instances/{name}", delete(handlers::delete_instance))
        .route("/instances/{name}/start", post(handlers::start_instance))
        .route("/instances/{name}/stop", post(handlers::stop_instance))
        .route(
            "/instances/{name}/restart",
            post(handlers::restart_instance),
        )
        .route("/instances/{name}/logs", get(handlers::get_logs))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state);

    let bind_addr = "0.0.0.0:8080";
    tracing::info!("Starting API server on {}", bind_addr);
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
