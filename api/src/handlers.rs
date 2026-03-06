use std::env;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::Sse;
use axum::response::{IntoResponse, Json};
use chrono::Utc;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, DeleteParams, ListParams, Patch, PatchParams, PostParams};
use kube::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::crd::{Agent, AgentSpec, AgentState};
use crate::error::AppError;
use crate::sse::watch_agent_status;

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
    pub ssh_port: Option<i32>,
    pub host_node: Option<String>,
    pub restart_count: Option<i32>,
    pub last_backup: Option<String>,
    pub message: Option<String>,
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
            ssh_port: status.ssh_port,
            host_node: status.host_node,
            restart_count: status.restart_count,
            last_backup: status.last_backup,
            message: status.message,
        }
    }
}

/// POST /instances - Create a new agent instance. Returns SSE stream of status updates.
pub async fn create_instance(
    State(state): State<AppState>,
    Json(req): Json<CreateInstanceRequest>,
) -> Result<impl IntoResponse, AppError> {
    let ns = agent_namespace();
    let api: Api<Agent> = Api::namespaced(state.client.clone(), &ns);

    // Check if already exists
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
            env: Vec::new(),
        },
    );

    api.create(&PostParams::default(), &agent).await?;

    let watch_api: Api<Agent> = Api::namespaced(state.client.clone(), &ns);
    let stream = watch_agent_status(watch_api, req.name);

    Ok(Sse::new(stream))
}

/// GET /instances - List all agent instances.
pub async fn list_instances(
    State(state): State<AppState>,
) -> Result<Json<Vec<InstanceResponse>>, AppError> {
    let ns = agent_namespace();
    let api: Api<Agent> = Api::namespaced(state.client, &ns);
    let agents = api.list(&ListParams::default()).await?;

    let instances: Vec<InstanceResponse> = agents.items.into_iter().map(Into::into).collect();
    Ok(Json(instances))
}

/// GET /instances/:name - Get a single agent instance.
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

/// POST /instances/:name/start - Set agent state to running.
pub async fn start_instance(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<InstanceResponse>, AppError> {
    let ns = agent_namespace();
    let api: Api<Agent> = Api::namespaced(state.client, &ns);

    // Verify it exists
    api.get_opt(&name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("instance '{}' not found", name)))?;

    let patch = json!({
        "spec": {
            "state": "running"
        }
    });
    let agent = api
        .patch(
            &name,
            &PatchParams::apply("nearai-api"),
            &Patch::Merge(&patch),
        )
        .await?;

    Ok(Json(agent.into()))
}

/// POST /instances/:name/stop - Set agent state to stopped.
pub async fn stop_instance(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<InstanceResponse>, AppError> {
    let ns = agent_namespace();
    let api: Api<Agent> = Api::namespaced(state.client, &ns);

    api.get_opt(&name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("instance '{}' not found", name)))?;

    let patch = json!({
        "spec": {
            "state": "stopped"
        }
    });
    let agent = api
        .patch(
            &name,
            &PatchParams::apply("nearai-api"),
            &Patch::Merge(&patch),
        )
        .await?;

    Ok(Json(agent.into()))
}

/// POST /instances/:name/restart - Delete the agent's pod so the operator recreates it.
pub async fn restart_instance(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let ns = agent_namespace();
    let agent_api: Api<Agent> = Api::namespaced(state.client.clone(), &ns);

    agent_api
        .get_opt(&name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("instance '{}' not found", name)))?;

    // Delete the pod associated with this agent. The operator will recreate it.
    let pod_api: Api<Pod> = Api::namespaced(state.client, &ns);
    match pod_api.delete(&name, &DeleteParams::default()).await {
        Ok(_) => Ok(Json(json!({
            "message": format!("Pod for instance '{}' deleted; operator will recreate it", name)
        }))),
        Err(kube::Error::Api(err)) if err.code == 404 => Ok(Json(json!({
            "message": format!("No pod found for instance '{}'; it may already be restarting", name)
        }))),
        Err(e) => Err(AppError::KubeError(e)),
    }
}

/// DELETE /instances/:name - Delete the Agent CRD.
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

/// POST /instances/:name/backup - Trigger a backup by annotating the Agent CRD. Returns SSE stream.
pub async fn trigger_backup(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let ns = agent_namespace();
    let api: Api<Agent> = Api::namespaced(state.client.clone(), &ns);

    api.get_opt(&name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("instance '{}' not found", name)))?;

    let timestamp = Utc::now().to_rfc3339();
    let patch = json!({
        "metadata": {
            "annotations": {
                "agents.near.ai/backup-requested": timestamp
            }
        }
    });
    api.patch(
        &name,
        &PatchParams::apply("nearai-api"),
        &Patch::Merge(&patch),
    )
    .await?;

    let watch_api: Api<Agent> = Api::namespaced(state.client.clone(), &ns);
    let stream = watch_agent_status(watch_api, name);

    Ok(Sse::new(stream))
}

/// GET /instances/:name/backups - Return backup list from Agent status.
pub async fn list_backups(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let ns = agent_namespace();
    let api: Api<Agent> = Api::namespaced(state.client, &ns);

    let agent = api
        .get_opt(&name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("instance '{}' not found", name)))?;

    let last_backup = agent
        .status
        .as_ref()
        .and_then(|s| s.last_backup.clone());

    Ok(Json(json!({
        "name": name,
        "last_backup": last_backup,
    })))
}

/// GET /health - Health check endpoint.
pub async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok" }))
}
