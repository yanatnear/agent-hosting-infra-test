use std::convert::Infallible;
use std::env;

use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Json};
use futures::stream::Stream;
use k8s_openapi::api::core::v1::{Node, Pod, Service};
use kube::api::{Api, DeleteParams, ListParams, LogParams, Patch, PatchParams, PostParams};
use kube::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio_stream::StreamExt;

use crate::crd::{Agent, AgentSpec, AgentState};
use crate::error::AppError;
use crate::sse;

pub fn agent_namespace() -> String {
    env::var("AGENT_NAMESPACE").unwrap_or_else(|_| "agents".to_string())
}

#[derive(Clone)]
pub struct AppState {
    pub client: Client,
}

// ---------------------------------------------------------------------------
// Request/Response types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct CreateInstanceRequest {
    // CrabShack fields
    pub nearai_api_key: Option<String>,
    pub nearai_api_url: Option<String>,
    pub image: String,
    pub service_type: Option<String>,
    pub mem_limit: Option<String>,
    pub cpus: Option<String>,
    pub storage_size: Option<String>,
    pub ssh_pubkey: Option<String>,
    pub node_id: Option<String>,
    // Backward-compat fields
    pub name: Option<String>,
    pub cpu: Option<String>,
    pub memory: Option<String>,
    pub disk: Option<String>,
    pub volume_mount: Option<String>,
    pub security_profile: Option<String>,
    #[serde(default)]
    pub env: Vec<crate::crd::EnvVar>,
    #[serde(default)]
    pub ports: Vec<crate::crd::PortSpec>,
    #[serde(default)]
    pub enable_docker: bool,
    #[serde(default)]
    pub command: Vec<String>,
}

#[derive(Serialize)]
pub struct InstanceResponse {
    pub name: String,
    pub status: String,
    pub image: String,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pod_ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_node: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restart_count: Option<i32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub node_ports: Vec<NodePortResponse>,
}

#[derive(Serialize)]
pub struct NodePortResponse {
    pub name: String,
    pub port: i32,
    pub node_port: i32,
}

impl From<Agent> for InstanceResponse {
    fn from(agent: Agent) -> Self {
        let agent_status = agent.status.unwrap_or_default();
        let spec = agent.spec;
        let name = agent.metadata.name.clone().unwrap_or_default();

        let status = sse::derive_status_from_spec_and_status(&spec, &agent_status);

        let created_at = agent
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|ts| ts.0.to_rfc3339())
            .unwrap_or_default();

        InstanceResponse {
            name,
            status,
            image: spec.image,
            created_at,
            service_type: None,
            node_id: agent_status.host_node.clone(),
            pod_ip: agent_status.pod_ip,
            host_node: agent_status.host_node,
            restart_count: agent_status.restart_count,
            node_ports: agent_status
                .node_ports
                .into_iter()
                .map(|np| NodePortResponse {
                    name: np.name,
                    port: np.port,
                    node_port: np.node_port,
                })
                .collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Normalize memory/storage strings: "2G" → "2Gi", "512M" → "512Mi"
fn normalize_resource(s: &str) -> String {
    if s.ends_with('G') && !s.ends_with("Gi") {
        format!("{}i", s)
    } else if s.ends_with('M') && !s.ends_with("Mi") {
        format!("{}i", s)
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------------------
// POST /instances — Create (SSE)
// ---------------------------------------------------------------------------

pub async fn create_instance(
    State(state): State<AppState>,
    Json(req): Json<CreateInstanceRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    // Validate: if no backward-compat `name` field, require CrabShack fields
    if req.name.is_none() && req.service_type.is_none() {
        return Err(AppError::BadRequest(
            "missing required field: service_type".to_string(),
        ));
    }

    let ns = agent_namespace();
    let api: Api<Agent> = Api::namespaced(state.client.clone(), &ns);

    // Auto-generate name if not provided
    let name = req
        .name
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()[..8].to_string());

    if api.get_opt(&name).await?.is_some() {
        return Err(AppError::Conflict(format!(
            "instance '{}' already exists",
            name
        )));
    }

    // Map CrabShack fields to CRD spec, with backward compat
    let cpu = req.cpus.or(req.cpu).unwrap_or_else(|| "1".to_string());
    let memory = req
        .mem_limit
        .map(|m| normalize_resource(&m))
        .or(req.memory)
        .unwrap_or_else(|| "4Gi".to_string());
    let disk = req
        .storage_size
        .map(|s| normalize_resource(&s))
        .or(req.disk)
        .unwrap_or_else(|| "10Gi".to_string());

    // Build env vars — include nearai config if provided
    let mut env_vars = req.env;
    if let Some(key) = &req.nearai_api_key {
        env_vars.push(crate::crd::EnvVar {
            name: "NEARAI_API_KEY".to_string(),
            value: key.clone(),
        });
    }
    if let Some(url) = &req.nearai_api_url {
        env_vars.push(crate::crd::EnvVar {
            name: "NEARAI_API_URL".to_string(),
            value: url.clone(),
        });
    }

    // Default volume mount to /workspace for CrabShack payloads
    let volume_mount = req.volume_mount.unwrap_or_else(|| {
        if req.service_type.is_some() {
            "/workspace".to_string()
        } else {
            "/home/agent".to_string()
        }
    });

    let agent = Agent::new(
        &name,
        AgentSpec {
            image: req.image,
            cpu,
            memory,
            disk,
            state: AgentState::Running,
            volume_mount,
            security_profile: req
                .security_profile
                .unwrap_or_else(|| "trusted".to_string()),
            env: env_vars,
            enable_docker: req.enable_docker,
            command: req.command,
            ports: if req.ports.is_empty() {
                if req.service_type.is_some() {
                    // CrabShack payloads: default to 8080 (common worker port)
                    vec![crate::crd::PortSpec {
                        name: "http".to_string(),
                        port: 8080,
                    }]
                } else {
                    vec![
                        crate::crd::PortSpec {
                            name: "ssh".to_string(),
                            port: 22,
                        },
                        crate::crd::PortSpec {
                            name: "http".to_string(),
                            port: 80,
                        },
                    ]
                }
            } else {
                req.ports
            },
        },
    );

    api.create(&PostParams::default(), &agent).await?;

    let client = state.client.clone();
    let name_clone = name.clone();

    let stream = async_stream::stream! {
        yield sse::sse_event("created", json!({"name": name_clone}));

        let mut poll = sse::poll_status_stream(
            client,
            name_clone.clone(),
            "running",
            std::time::Duration::from_secs(600),
        );
        while let Some(event) = poll.next().await {
            yield event;
        }

        yield sse::sse_event("ready", json!({"name": name_clone}));
    };

    Ok(Sse::new(stream))
}

// ---------------------------------------------------------------------------
// GET /instances
// ---------------------------------------------------------------------------

pub async fn list_instances(
    State(state): State<AppState>,
) -> Result<Json<Vec<InstanceResponse>>, AppError> {
    let ns = agent_namespace();
    let api: Api<Agent> = Api::namespaced(state.client, &ns);
    let agents = api.list(&ListParams::default()).await?;
    let instances: Vec<InstanceResponse> = agents.items.into_iter().map(Into::into).collect();
    Ok(Json(instances))
}

// ---------------------------------------------------------------------------
// GET /instances/:name
// ---------------------------------------------------------------------------

pub async fn get_instance(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<InstanceResponse>, AppError> {
    let ns = agent_namespace();
    let api: Api<Agent> = Api::namespaced(state.client, &ns);
    let agent = api
        .get_opt(&name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("instance '{}' not found", name)))?;
    Ok(Json(agent.into()))
}

// ---------------------------------------------------------------------------
// POST /instances/:name/start (SSE)
// ---------------------------------------------------------------------------

pub async fn start_instance(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let ns = agent_namespace();
    let api: Api<Agent> = Api::namespaced(state.client.clone(), &ns);

    api.get_opt(&name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("instance '{}' not found", name)))?;

    let patch = json!({ "spec": { "state": "running" } });
    api.patch(
        &name,
        &PatchParams::apply("nearai-api"),
        &Patch::Merge(&patch),
    )
    .await?;

    let client = state.client.clone();
    let name_clone = name.clone();

    let stream = async_stream::stream! {
        let mut poll = sse::poll_status_stream(
            client, name_clone.clone(), "running",
            std::time::Duration::from_secs(600),
        );
        while let Some(event) = poll.next().await {
            yield event;
        }
        yield sse::sse_event("ready", json!({"name": name_clone}));
    };

    Ok(Sse::new(stream))
}

// ---------------------------------------------------------------------------
// POST /instances/:name/stop (SSE)
// ---------------------------------------------------------------------------

pub async fn stop_instance(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let ns = agent_namespace();
    let api: Api<Agent> = Api::namespaced(state.client.clone(), &ns);

    api.get_opt(&name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("instance '{}' not found", name)))?;

    let patch = json!({ "spec": { "state": "stopped" } });
    api.patch(
        &name,
        &PatchParams::apply("nearai-api"),
        &Patch::Merge(&patch),
    )
    .await?;

    let client = state.client.clone();
    let name_clone = name.clone();

    let stream = async_stream::stream! {
        let mut poll = sse::poll_status_stream(
            client, name_clone.clone(), "stopped",
            std::time::Duration::from_secs(120),
        );
        while let Some(event) = poll.next().await {
            yield event;
        }
        yield sse::sse_event("stopped", json!({"name": name_clone}));
    };

    Ok(Sse::new(stream))
}

// ---------------------------------------------------------------------------
// POST /instances/:name/restart (SSE)
// ---------------------------------------------------------------------------

pub async fn restart_instance(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let ns = agent_namespace();
    let agent_api: Api<Agent> = Api::namespaced(state.client.clone(), &ns);
    let pod_api: Api<Pod> = Api::namespaced(state.client.clone(), &ns);

    agent_api
        .get_opt(&name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("instance '{}' not found", name)))?;

    let pod_name = format!("agent-{}", name);
    let _ = pod_api.delete(&pod_name, &DeleteParams::default()).await;

    let client = state.client.clone();
    let name_clone = name.clone();

    let stream = async_stream::stream! {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        let mut poll = sse::poll_status_stream(
            client, name_clone.clone(), "running",
            std::time::Duration::from_secs(600),
        );
        while let Some(event) = poll.next().await {
            yield event;
        }
        yield sse::sse_event("ready", json!({"name": name_clone}));
    };

    Ok(Sse::new(stream))
}

// ---------------------------------------------------------------------------
// DELETE /instances/:name (SSE — always returns 200 stream)
// ---------------------------------------------------------------------------

pub async fn delete_instance(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let client = state.client.clone();
    let ns = agent_namespace();

    let stream = async_stream::stream! {
        let api: Api<Agent> = Api::namespaced(client.clone(), &ns);

        match api.get_opt(&name).await {
            Ok(Some(_)) => {
                match api.delete(&name, &DeleteParams::default()).await {
                    Ok(_) => {
                        let deadline = tokio::time::Instant::now()
                            + std::time::Duration::from_secs(60);
                        loop {
                            if tokio::time::Instant::now() > deadline {
                                yield sse::sse_error("timeout waiting for deletion");
                                break;
                            }
                            match api.get_opt(&name).await {
                                Ok(None) => {
                                    yield sse::sse_event("deleted", json!({"name": name}));
                                    break;
                                }
                                Ok(Some(_)) => {
                                    yield sse::sse_event("status", json!({"status": "deleting"}));
                                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                                }
                                Err(_) => {
                                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        yield sse::sse_error(&format!("delete failed: {}", e));
                    }
                }
            }
            Ok(None) => {
                yield sse::sse_error(&format!("instance '{}' not found", name));
            }
            Err(e) => {
                yield sse::sse_error(&format!("error: {}", e));
            }
        }
    };

    Sse::new(stream)
}

// ---------------------------------------------------------------------------
// GET /instances/:name/logs
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct LogsQuery {
    pub tail: Option<i64>,
}

pub async fn get_logs(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<LogsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let ns = agent_namespace();

    let agent_api: Api<Agent> = Api::namespaced(state.client.clone(), &ns);
    agent_api
        .get_opt(&name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("instance '{}' not found", name)))?;

    let pod_api: Api<Pod> = Api::namespaced(state.client, &ns);
    let pod_name = format!("agent-{}", name);

    let tail_lines = query.tail.unwrap_or(100);
    let log_params = LogParams {
        tail_lines: Some(tail_lines),
        ..Default::default()
    };

    match pod_api.logs(&pod_name, &log_params).await {
        Ok(logs) => Ok(Json(json!({
            "name": name,
            "logs": logs,
        }))),
        Err(kube::Error::Api(err)) if err.code == 404 => Err(AppError::NotFound(format!(
            "pod for instance '{}' not found (instance may be stopped)",
            name
        ))),
        Err(e) => Err(AppError::KubeError(e)),
    }
}

// ---------------------------------------------------------------------------
// GET /instances/:name/ssh
// ---------------------------------------------------------------------------

pub async fn get_ssh_info(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let ns = agent_namespace();
    let agent_api: Api<Agent> = Api::namespaced(state.client.clone(), &ns);
    let services: Api<Service> = Api::namespaced(state.client, &ns);

    agent_api
        .get_opt(&name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("instance '{}' not found", name)))?;

    let svc_name = format!("agent-{}", name);
    let svc = services
        .get_opt(&svc_name)
        .await?
        .ok_or_else(|| AppError::NotFound("service not found".to_string()))?;

    let ssh_port = svc
        .spec
        .as_ref()
        .and_then(|s| s.ports.as_ref())
        .and_then(|ports| ports.iter().find(|p| p.name.as_deref() == Some("ssh")))
        .and_then(|p| p.node_port)
        .unwrap_or(22);

    let host =
        env::var("NODE_SSH_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());

    Ok(Json(json!({
        "host": host,
        "port": ssh_port,
        "user": "agent",
    })))
}

// ---------------------------------------------------------------------------
// GET /instances/:name/stats
// ---------------------------------------------------------------------------

pub async fn get_stats(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let ns = agent_namespace();
    let agent_api: Api<Agent> = Api::namespaced(state.client, &ns);

    let agent = agent_api
        .get_opt(&name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("instance '{}' not found", name)))?;

    let status = agent.status.unwrap_or_default();

    Ok(Json(json!({
        "name": name,
        "stats": {
            "cpu_requested": agent.spec.cpu,
            "memory_requested": agent.spec.memory,
            "restart_count": status.restart_count.unwrap_or(0),
            "phase": status.phase.unwrap_or_else(|| "unknown".to_string()),
        }
    })))
}

// ---------------------------------------------------------------------------
// GET /nodes
// ---------------------------------------------------------------------------

pub async fn list_nodes(
    State(state): State<AppState>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    let nodes_api: Api<Node> = Api::all(state.client);
    let nodes = nodes_api.list(&ListParams::default()).await?;

    let ssh_host =
        env::var("NODE_SSH_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let ssh_port: i32 = env::var("NODE_SSH_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(22);
    let ssh_user =
        env::var("NODE_SSH_USER").unwrap_or_else(|_| "root".to_string());

    let result: Vec<serde_json::Value> = nodes
        .items
        .iter()
        .map(|node| {
            let name = node.metadata.name.clone().unwrap_or_default();
            json!({
                "id": name,
                "hostname": name,
                "status": "active",
                "ssh_host": ssh_host,
                "ssh_port": ssh_port,
                "ssh_user": ssh_user,
                "vcpu_total": 8,
                "ram_mb_total": 16384,
                "disk_gb_total": 200,
                "port_range_start": 30000,
                "port_range_end": 32767,
            })
        })
        .collect();

    Ok(Json(result))
}

// ---------------------------------------------------------------------------
// GET /health
// ---------------------------------------------------------------------------

pub async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok" }))
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::{AgentSpec, AgentState, AgentStatus};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
    use k8s_openapi::chrono::Utc;

    fn make_agent_with_status(
        name: &str,
        state: AgentState,
        status: Option<AgentStatus>,
    ) -> Agent {
        Agent {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some("agents".to_string()),
                creation_timestamp: Some(Time(Utc::now())),
                ..Default::default()
            },
            spec: AgentSpec {
                enable_docker: false,
                image: "registry.example.com/agent:v3.0.0".to_string(),
                state,
                cpu: "4".to_string(),
                memory: "16Gi".to_string(),
                disk: "100Gi".to_string(),
                volume_mount: "/home/agent".to_string(),
                security_profile: "restricted".to_string(),
                env: vec![],
                ports: vec![],
                command: vec![],
            },
            status,
        }
    }

    #[test]
    fn p0_from_agent_running_maps_all_fields() {
        let status = AgentStatus {
            phase: Some("Running".to_string()),
            pod_ip: Some("10.42.1.99".to_string()),
            host_node: Some("gpu-worker-03".to_string()),
            restart_count: Some(3),
            ..Default::default()
        };
        let agent = make_agent_with_status("prod-analyst", AgentState::Running, Some(status));
        let response: InstanceResponse = agent.into();

        assert_eq!(response.name, "prod-analyst");
        assert_eq!(response.status, "running");
        assert_eq!(response.image, "registry.example.com/agent:v3.0.0");
        assert!(!response.created_at.is_empty());
        assert_eq!(response.pod_ip.as_deref(), Some("10.42.1.99"));
        assert_eq!(response.host_node.as_deref(), Some("gpu-worker-03"));
        assert_eq!(response.restart_count, Some(3));
    }

    #[test]
    fn p0_from_agent_stopped_handles_missing_status() {
        let agent = make_agent_with_status("prod-analyst", AgentState::Stopped, None);
        let response: InstanceResponse = agent.into();

        assert_eq!(response.status, "stopped");
        assert_eq!(response.pod_ip, None);
        assert_eq!(response.host_node, None);
    }

    #[test]
    fn p0_from_agent_creating_phase() {
        let status = AgentStatus {
            phase: Some("Creating".to_string()),
            ..Default::default()
        };
        let agent = make_agent_with_status("new-agent", AgentState::Running, Some(status));
        let response: InstanceResponse = agent.into();
        assert_eq!(response.status, "creating");
    }

    #[test]
    fn p0_from_agent_crashloop_maps_to_error() {
        let status = AgentStatus {
            phase: Some("CrashLoopBackOff".to_string()),
            ..Default::default()
        };
        let agent = make_agent_with_status("broken", AgentState::Running, Some(status));
        let response: InstanceResponse = agent.into();
        assert_eq!(response.status, "error");
    }

    #[test]
    fn p0_normalize_resource_converts_g_to_gi() {
        assert_eq!(normalize_resource("2G"), "2Gi");
        assert_eq!(normalize_resource("512M"), "512Mi");
        assert_eq!(normalize_resource("4Gi"), "4Gi");
        assert_eq!(normalize_resource("1"), "1");
    }

    #[test]
    fn p0_logs_query_default_tail_is_none() {
        let query: LogsQuery =
            serde_json::from_str("{}").expect("empty JSON must deserialize");
        assert_eq!(query.tail, None);
    }

    #[test]
    fn p0_logs_query_explicit_tail_preserved() {
        let query: LogsQuery =
            serde_json::from_str(r#"{"tail": 50}"#).expect("must deserialize");
        assert_eq!(query.tail, Some(50));
    }
}
