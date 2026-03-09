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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::{AgentSpec, AgentState, AgentStatus};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    // -----------------------------------------------------------------------
    // Constants
    // -----------------------------------------------------------------------

    const TEST_AGENT_NAME: &str = "prod-analyst";
    const TEST_IMAGE: &str = "registry.example.com/agent:v3.0.0";
    const TEST_CPU: &str = "4";
    const TEST_MEMORY: &str = "16Gi";
    const TEST_DISK: &str = "100Gi";
    const TEST_POD_IP: &str = "10.42.1.99";
    const TEST_HOST_NODE: &str = "gpu-worker-03";

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    /// Creates a fully-populated Agent CRD for testing From<Agent> conversion.
    fn make_agent_with_status(
        name: &str,
        state: AgentState,
        status: Option<AgentStatus>,
    ) -> Agent {
        Agent {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some("agents".to_string()),
                ..Default::default()
            },
            spec: AgentSpec {
                image: TEST_IMAGE.to_string(),
                state,
                cpu: TEST_CPU.to_string(),
                memory: TEST_MEMORY.to_string(),
                disk: TEST_DISK.to_string(),
                volume_mount: "/home/agent".to_string(),
                security_profile: "restricted".to_string(),
                env: vec![],
                ports: vec![],
            },
            status,
        }
    }

    // -----------------------------------------------------------------------
    // From<Agent> for InstanceResponse tests
    // -----------------------------------------------------------------------

    /// **Test Case #21 — Running agent with full status**
    ///
    /// WHY THIS MATTERS:
    /// The InstanceResponse is the primary data contract between the API and
    /// its consumers (CLI, web UI, external integrations). Every field must be
    /// correctly mapped from the Agent CRD, or users see wrong information.
    ///
    /// WHAT THIS TEST DOES:
    /// 1. Creates an Agent with Running state and a populated AgentStatus
    /// 2. Converts to InstanceResponse via From trait
    /// 3. Asserts every field is correctly mapped
    ///
    /// IF THIS FAILS:
    /// API responses will contain incorrect agent details. Users won't be able
    /// to trust the data returned by GET /instances/:name.
    ///
    /// WHAT IS BEING TESTED:
    /// `From<Agent> for InstanceResponse` — pure conversion, no K8s client.
    #[test]
    fn p0_from_agent_running_maps_all_fields_correctly() {
        let status = AgentStatus {
            phase: Some("Running".to_string()),
            pod_ip: Some(TEST_POD_IP.to_string()),
            host_node: Some(TEST_HOST_NODE.to_string()),
            restart_count: Some(3),
            ..Default::default()
        };
        let agent = make_agent_with_status(TEST_AGENT_NAME, AgentState::Running, Some(status));

        let response: InstanceResponse = agent.into();

        assert_eq!(response.name, TEST_AGENT_NAME, "name must match metadata.name");
        assert_eq!(response.image, TEST_IMAGE, "image must match spec.image");
        assert_eq!(response.cpu, TEST_CPU, "cpu must match spec.cpu");
        assert_eq!(response.memory, TEST_MEMORY, "memory must match spec.memory");
        assert_eq!(response.disk, TEST_DISK, "disk must match spec.disk");
        assert_eq!(response.state, "running", "state must be lowercase 'running'");
        assert_eq!(
            response.phase.as_deref(),
            Some("Running"),
            "phase must come from status.phase"
        );
        assert_eq!(
            response.pod_ip.as_deref(),
            Some(TEST_POD_IP),
            "pod_ip must come from status.pod_ip"
        );
        assert_eq!(
            response.host_node.as_deref(),
            Some(TEST_HOST_NODE),
            "host_node must come from status.host_node"
        );
        assert_eq!(
            response.restart_count,
            Some(3),
            "restart_count must come from status.restart_count"
        );
    }

    /// **Test Case #21 — Stopped agent with no status subresource**
    ///
    /// WHY THIS MATTERS:
    /// A freshly stopped agent may not have a status subresource yet. The
    /// conversion must handle None status gracefully, defaulting all status
    /// fields rather than panicking.
    ///
    /// WHAT THIS TEST DOES:
    /// 1. Creates an Agent with Stopped state and status=None
    /// 2. Converts to InstanceResponse
    /// 3. Asserts status fields are None/default (not panic)
    ///
    /// IF THIS FAILS:
    /// GET /instances/:name will panic with a None unwrap when an agent
    /// has no status subresource, causing 500 errors.
    ///
    /// WHAT IS BEING TESTED:
    /// `From<Agent> for InstanceResponse` None-status path — pure conversion.
    #[test]
    fn p0_from_agent_stopped_handles_missing_status_gracefully() {
        let agent = make_agent_with_status(TEST_AGENT_NAME, AgentState::Stopped, None);

        let response: InstanceResponse = agent.into();

        assert_eq!(response.state, "stopped", "state must be lowercase 'stopped'");
        assert_eq!(
            response.phase, None,
            "phase must be None when status is absent"
        );
        assert_eq!(
            response.pod_ip, None,
            "pod_ip must be None when status is absent"
        );
        assert_eq!(
            response.host_node, None,
            "host_node must be None when status is absent"
        );
    }

    /// **Test Case #21 — Edge case: missing metadata.name**
    ///
    /// WHY THIS MATTERS:
    /// In rare cases (e.g., during CRD migration), an Agent may have
    /// metadata.name=None. The conversion must not panic — it should default
    /// to an empty string.
    ///
    /// WHAT THIS TEST DOES:
    /// 1. Creates an Agent with metadata.name=None
    /// 2. Converts to InstanceResponse
    /// 3. Asserts name is empty string (not panic)
    ///
    /// IF THIS FAILS:
    /// The API will panic on agents with missing names during list operations,
    /// causing the entire list endpoint to fail.
    ///
    /// WHAT IS BEING TESTED:
    /// `From<Agent> for InstanceResponse` edge case — pure conversion.
    #[test]
    fn p0_from_agent_missing_name_defaults_to_empty_string() {
        let agent = Agent {
            metadata: ObjectMeta {
                name: None,
                ..Default::default()
            },
            spec: AgentSpec {
                image: TEST_IMAGE.to_string(),
                state: AgentState::Running,
                cpu: TEST_CPU.to_string(),
                memory: TEST_MEMORY.to_string(),
                disk: TEST_DISK.to_string(),
                volume_mount: "/home/agent".to_string(),
                security_profile: "restricted".to_string(),
                env: vec![],
                ports: vec![],
            },
            status: None,
        };

        let response: InstanceResponse = agent.into();

        assert_eq!(
            response.name, "",
            "missing metadata.name must default to empty string, not panic"
        );
    }

    // -----------------------------------------------------------------------
    // LogsQuery tests
    // -----------------------------------------------------------------------

    /// **Test Case #17 — LogsQuery default tail**
    ///
    /// WHY THIS MATTERS:
    /// When users call GET /instances/:name/logs without ?tail=N, the query
    /// parameter must default to None so the handler applies the 100-line
    /// default. If deserialization fails on empty query, the endpoint returns
    /// 400 instead of logs.
    ///
    /// WHAT THIS TEST DOES:
    /// 1. Deserializes an empty JSON object to LogsQuery
    /// 2. Asserts tail is None
    ///
    /// IF THIS FAILS:
    /// The logs endpoint will reject requests without explicit ?tail parameter,
    /// breaking the CLI's default "show recent logs" behavior.
    ///
    /// WHAT IS BEING TESTED:
    /// LogsQuery serde deserialization — pure logic.
    #[test]
    fn p0_logs_query_default_tail_is_none() {
        let query: LogsQuery =
            serde_json::from_str("{}").expect("empty JSON must deserialize to LogsQuery");

        assert_eq!(
            query.tail, None,
            "tail must be None when not specified, letting the handler apply its default"
        );
    }

    /// **Test Case #17 — LogsQuery explicit tail**
    ///
    /// WHY THIS MATTERS:
    /// Users should be able to request a specific number of log lines via
    /// ?tail=50. The value must be preserved exactly through deserialization.
    ///
    /// WHAT THIS TEST DOES:
    /// 1. Deserializes `{"tail": 50}` to LogsQuery
    /// 2. Asserts tail is Some(50)
    ///
    /// IF THIS FAILS:
    /// The ?tail parameter is silently ignored, and users always get the
    /// default number of lines regardless of what they requested.
    ///
    /// WHAT IS BEING TESTED:
    /// LogsQuery serde deserialization with explicit value — pure logic.
    #[test]
    fn p0_logs_query_explicit_tail_is_preserved() {
        let query: LogsQuery =
            serde_json::from_str(r#"{"tail": 50}"#).expect("tail JSON must deserialize");

        assert_eq!(
            query.tail,
            Some(50),
            "explicit tail value must be preserved through deserialization"
        );
    }
}
