use std::convert::Infallible;
use std::pin::Pin;

use axum::response::sse::Event;
use futures::stream::Stream;
use futures::StreamExt;
use kube::api::Api;
use kube::runtime::watcher::{self, Event as WatcherEvent};
use serde_json::json;

use crate::crd::Agent;

/// Watch an Agent CRD by name and produce SSE events for status transitions.
/// The stream completes when the agent reaches a terminal phase ("Running", "Stopped")
/// or when a timeout is reached.
pub fn watch_agent_status(
    api: Api<Agent>,
    name: String,
) -> Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>> {
    let watch_stream = watcher::watcher(
        api,
        watcher::Config::default()
            .fields(&format!("metadata.name={}", name))
            .timeout(120),
    );

    let event_stream = watch_stream.filter_map(move |event| {
        let name = name.clone();
        async move {
            match event {
                Ok(WatcherEvent::Apply(agent)) | Ok(WatcherEvent::InitApply(agent)) => {
                    let phase = agent
                        .status
                        .as_ref()
                        .and_then(|s| s.phase.clone())
                        .unwrap_or_default();
                    let pod_ip = agent.status.as_ref().and_then(|s| s.pod_ip.clone());
                    let ssh_port = agent.status.as_ref().and_then(|s| s.ssh_port);
                    let message = agent.status.as_ref().and_then(|s| s.message.clone());

                    let data = json!({
                        "name": name,
                        "phase": phase,
                        "pod_ip": pod_ip,
                        "ssh_port": ssh_port,
                        "message": message,
                    });

                    let is_terminal =
                        matches!(phase.as_str(), "Running" | "Stopped" | "Failed");

                    if is_terminal {
                        let event_type = if phase == "Failed" { "error" } else { "complete" };
                        Some(Ok(Event::default()
                            .event(event_type)
                            .json_data(data)
                            .unwrap()))
                    } else {
                        Some(Ok(Event::default()
                            .event("status")
                            .json_data(data)
                            .unwrap()))
                    }
                }
                Ok(WatcherEvent::Delete(_)) => {
                    let data = json!({
                        "name": name,
                        "phase": "Deleted",
                        "message": "Agent resource was deleted",
                    });
                    Some(Ok(Event::default()
                        .event("complete")
                        .json_data(data)
                        .unwrap()))
                }
                Ok(WatcherEvent::Init) | Ok(WatcherEvent::InitDone) => None,
                Err(e) => {
                    let data = json!({
                        "name": name,
                        "error": e.to_string(),
                    });
                    Some(Ok(Event::default()
                        .event("error")
                        .json_data(data)
                        .unwrap()))
                }
            }
        }
    });

    // Use scan to stop after terminal event, boxed for Unpin
    let terminal_stream = event_stream
        .scan(false, |done, item| {
            if *done {
                return futures::future::ready(None);
            }
            if let Ok(ref event) = item {
                let event_str = format!("{:?}", event);
                if event_str.contains("\"complete\"") || event_str.contains("\"error\"") {
                    *done = true;
                }
            }
            futures::future::ready(Some(item))
        });

    Box::pin(terminal_stream)
}
