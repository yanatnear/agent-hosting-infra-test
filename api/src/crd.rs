use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(CustomResource, Deserialize, Serialize, Clone, Debug, JsonSchema)]
#[kube(
    group = "agents.near.ai",
    version = "v1",
    kind = "Agent",
    plural = "agents",
    namespaced,
    status = "AgentStatus",
    printcolumn = r#"{"name":"State", "type":"string", "jsonPath":".spec.state"}"#,
    printcolumn = r#"{"name":"Phase", "type":"string", "jsonPath":".status.phase"}"#
)]
pub struct AgentSpec {
    pub image: String,
    #[serde(default = "default_cpu")]
    pub cpu: String,
    #[serde(default = "default_memory")]
    pub memory: String,
    #[serde(default = "default_disk")]
    pub disk: String,
    #[serde(default = "default_state")]
    pub state: AgentState,
    /// Mount path for the persistent volume.
    #[serde(default = "default_volume_mount")]
    pub volume_mount: String,
    /// Security profile: "restricted" (default) or "trusted".
    #[serde(default = "default_security_profile")]
    pub security_profile: String,
    /// Environment variables to set on the agent pod.
    #[serde(default)]
    pub env: Vec<EnvVar>,
    /// Ports to expose on the agent pod and service.
    #[serde(default = "default_ports")]
    pub ports: Vec<PortSpec>,
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema)]
pub struct PortSpec {
    pub name: String,
    pub port: i32,
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AgentState {
    Running,
    Stopped,
}

fn default_cpu() -> String {
    "1".to_string()
}
fn default_memory() -> String {
    "4Gi".to_string()
}
fn default_disk() -> String {
    "10Gi".to_string()
}
fn default_state() -> AgentState {
    AgentState::Running
}
fn default_volume_mount() -> String {
    "/home/agent".to_string()
}
fn default_security_profile() -> String {
    "restricted".to_string()
}

fn default_ports() -> Vec<PortSpec> {
    vec![
        PortSpec { name: "ssh".to_string(), port: 22 },
        PortSpec { name: "http".to_string(), port: 80 },
    ]
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema, Default)]
pub struct AgentStatus {
    #[serde(default)]
    pub phase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_node: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pod_ip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_port: Option<i32>,
    #[serde(default)]
    pub restart_count: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_backup: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub node_ports: Vec<NodePort>,
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema)]
pub struct NodePort {
    pub name: String,
    pub port: i32,
    pub node_port: i32,
}
