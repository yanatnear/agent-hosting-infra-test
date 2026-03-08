use std::env;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json};
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, DeleteParams, ListParams, LogParams, Patch, PatchParams, PostParams};
use kube::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::crd::{Agent, AgentSpec, AgentState};
use crate::error::AppError;

fn agent_namespace() -> String {
    env::var("AGENT_NAMESPACE").unwrap_or_else(|_| "agents".to_string())
}

#[derive(Clone)]
pub struct AppState {
    pub client: Client,
}

#[derive(Deserialize)]
pub struct CreateInstanceRequest {
    pub name: String,
    pub image: String,
    pub cpu: Option<String>,
    pub memory: Option<String>,
    pub disk: Option<String>,
    pub volume_mount: Option<String>,
    pub security_profile: Option<String>,
    #[serde(default)]
    pub env: Vec<crate::crd::EnvVar>,
    #[serde(default)]
    pub ports: Vec<crate::crd::PortSpec>,
}

#[derive(Serialize)]
pub struct InstanceResponse {
    pub name: String,
    pub image: String,
    pub cpu: String,
    pub memory: String,
    pub disk: String,
    pub state: String,
    pub phase: Option<String>,
    pub pod_ip: Option<String>,
    pub host_node: Option<String>,
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
        let status = agent.status.unwrap_or_default();
        let spec = agent.spec;
        let name = agent.metadata.name.unwrap_or_default();

        InstanceResponse {
            name,
            image: spec.image,
            cpu: spec.cpu,
            memory: spec.memory,
            disk: spec.disk,
            state: serde_json::to_value(&spec.state)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_else(|| "running".to_string()),
            phase: status.phase,
            pod_ip: status.pod_ip,
            host_node: status.host_node,
            restart_count: status.restart_count,
            node_ports: status.node_ports.into_iter().map(|np| NodePortResponse {
                name: np.name,
                port: np.port,
                node_port: np.node_port,
            }).collect(),
        }
    }
}

/// POST /instances — Create a new agent instance (synchronous).
pub async fn create_instance(
    State(state): State<AppState>,
    Json(req): Json<CreateInstanceRequest>,
) -> Result<(StatusCode, Json<InstanceResponse>), AppError> {
    let ns = agent_namespace();
    let api: Api<Agent> = Api::namespaced(state.client, &ns);

    if api.get_opt(&req.name).await?.is_some() {
        return Err(AppError::Conflict(format!(
            "instance '{}' already exists",
            req.name
        )));
    }

    let agent = Agent::new(
        &req.name,
        AgentSpec {
            image: req.image,
            cpu: req.cpu.unwrap_or_else(|| "1".to_string()),
            memory: req.memory.unwrap_or_else(|| "4Gi".to_string()),
            disk: req.disk.unwrap_or_else(|| "10Gi".to_string()),
            state: AgentState::Running,
            volume_mount: req.volume_mount.unwrap_or_else(|| "/home/agent".to_string()),
            security_profile: req.security_profile.unwrap_or_else(|| "restricted".to_string()),
            env: req.env,
            ports: if req.ports.is_empty() {
                vec![
                    crate::crd::PortSpec { name: "ssh".to_string(), port: 22 },
                    crate::crd::PortSpec { name: "http".to_string(), port: 80 },
                ]
            } else {
                req.ports
            },
        },
    );

    let created = api.create(&PostParams::default(), &agent).await?;
    Ok((StatusCode::CREATED, Json(created.into())))
}

/// GET /instances — List all agent instances.
pub async fn list_instances(
    State(state): State<AppState>,
) -> Result<Json<Vec<InstanceResponse>>, AppError> {
    let ns = agent_namespace();
    let api: Api<Agent> = Api::namespaced(state.client, &ns);
    let agents = api.list(&ListParams::default()).await?;

    let instances: Vec<InstanceResponse> = agents.items.into_iter().map(Into::into).collect();
    Ok(Json(instances))
}

/// GET /instances/:name — Get a single agent instance.
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

/// POST /instances/:name/start — Start a stopped instance.
pub async fn start_instance(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<InstanceResponse>, AppError> {
    let ns = agent_namespace();
    let api: Api<Agent> = Api::namespaced(state.client, &ns);

    api.get_opt(&name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("instance '{}' not found", name)))?;

    let patch = json!({ "spec": { "state": "running" } });
    let agent = api
        .patch(&name, &PatchParams::apply("nearai-api"), &Patch::Merge(&patch))
        .await?;

    Ok(Json(agent.into()))
}

/// POST /instances/:name/stop — Stop a running instance.
pub async fn stop_instance(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<InstanceResponse>, AppError> {
    let ns = agent_namespace();
    let api: Api<Agent> = Api::namespaced(state.client, &ns);

    api.get_opt(&name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("instance '{}' not found", name)))?;

    let patch = json!({ "spec": { "state": "stopped" } });
    let agent = api
        .patch(&name, &PatchParams::apply("nearai-api"), &Patch::Merge(&patch))
        .await?;

    Ok(Json(agent.into()))
}

/// POST /instances/:name/restart — Restart instance (delete pod, operator recreates).
pub async fn restart_instance(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<InstanceResponse>, AppError> {
    let ns = agent_namespace();
    let agent_api: Api<Agent> = Api::namespaced(state.client.clone(), &ns);

    let agent = agent_api
        .get_opt(&name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("instance '{}' not found", name)))?;

    let pod_api: Api<Pod> = Api::namespaced(state.client, &ns);
    let pod_name = format!("agent-{}", name);
    // Best-effort delete; pod may not exist yet
    let _ = pod_api.delete(&pod_name, &DeleteParams::default()).await;

    Ok(Json(agent.into()))
}

/// DELETE /instances/:name — Delete an instance and all its resources.
pub async fn delete_instance(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let ns = agent_namespace();
    let api: Api<Agent> = Api::namespaced(state.client, &ns);

    api.get_opt(&name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("instance '{}' not found", name)))?;

    api.delete(&name, &DeleteParams::default()).await?;

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct LogsQuery {
    /// Number of lines to return from the end of the log (default: 100).
    pub tail: Option<i64>,
}

/// GET /instances/:name/logs — Tail logs from the agent's pod.
pub async fn get_logs(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<LogsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let ns = agent_namespace();

    // Verify instance exists
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
            "lines": logs.lines().collect::<Vec<_>>(),
        }))),
        Err(kube::Error::Api(err)) if err.code == 404 => Err(AppError::NotFound(format!(
            "pod for instance '{}' not found (instance may be stopped)",
            name
        ))),
        Err(e) => Err(AppError::KubeError(e)),
    }
}

/// GET /health — Health check.
pub async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok" }))
}
