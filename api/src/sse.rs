use axum::response::sse::Event;
use futures::stream::Stream;
use serde_json::json;
use std::convert::Infallible;
use std::pin::Pin;
use std::time::Duration;

use crate::crd::{Agent, AgentState};
use crate::handlers::agent_namespace;
use kube::api::Api;
use kube::Client;

/// Create an SSE event with the given name and JSON data.
pub fn sse_event(name: &str, data: serde_json::Value) -> Result<Event, Infallible> {
    Ok(Event::default().event(name).data(data.to_string()))
}

/// Create an SSE error event.
pub fn sse_error(message: &str) -> Result<Event, Infallible> {
    sse_event("error", json!({"error": message}))
}

use crate::crd::AgentStatus;

/// Derive a CrabShack-compatible status string from an Agent CRD.
pub fn derive_status(agent: &Agent) -> String {
    derive_status_from_spec_and_status(&agent.spec, agent.status.as_ref().unwrap_or(&AgentStatus::default()))
}

/// Derive status from spec and status fields (used by both SSE polling and InstanceResponse).
pub fn derive_status_from_spec_and_status(spec: &crate::crd::AgentSpec, status: &AgentStatus) -> String {
    if spec.state == AgentState::Stopped {
        return "stopped".to_string();
    }
    match status.phase.as_deref() {
        Some("Running") => "running".to_string(),
        Some("Creating") | Some("Pending") | None => "creating".to_string(),
        Some("Stopped") => "stopped".to_string(),
        Some("CrashLoopBackOff") | Some("Failed") => "error".to_string(),
        Some(other) => other.to_lowercase(),
    }
}

/// Poll until agent reaches target status or timeout.
/// Returns a stream that yields nothing on success, or an error event on timeout.
pub fn poll_status_stream(
    client: Client,
    name: String,
    target_status: &'static str,
    timeout: Duration,
) -> Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>> {
    let stream = async_stream::stream! {
        let ns = agent_namespace();
        let api: Api<Agent> = Api::namespaced(client, &ns);
        let deadline = tokio::time::Instant::now() + timeout;
        let poll_interval = Duration::from_secs(3);

        loop {
            if tokio::time::Instant::now() > deadline {
                yield sse_error("timeout waiting for status transition");
                break;
            }
            match api.get_opt(&name).await {
                Ok(Some(agent)) => {
                    let current = derive_status(&agent);
                    if current.as_str() == target_status {
                        break;
                    }
                    // Emit status event to keep connection alive during long polls
                    yield sse_event("status", json!({"status": current}));
                }
                Ok(None) => {
                    if target_status == "deleted" {
                        break;
                    }
                }
                Err(e) => {
                    yield sse_error(&format!("polling error: {}", e));
                    break;
                }
            }
            tokio::time::sleep(poll_interval).await;
        }
    };
    Box::pin(stream)
}
