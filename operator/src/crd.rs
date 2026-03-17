use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(CustomResource, Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "agents.near.ai",
    version = "v1",
    kind = "Agent",
    plural = "agents",
    namespaced,
    status = "AgentStatus",
    printcolumn = r#"{"name":"State","type":"string","jsonPath":".spec.state"}"#,
    printcolumn = r#"{"name":"Phase","type":"string","jsonPath":".status.phase"}"#,
    printcolumn = r#"{"name":"Image","type":"string","jsonPath":".spec.image"}"#
)]
pub struct AgentSpec {
    pub image: String,
    #[serde(default = "default_state")]
    pub state: AgentState,
    #[serde(default = "default_cpu")]
    pub cpu: String,
    #[serde(default = "default_memory")]
    pub memory: String,
    #[serde(default = "default_disk")]
    pub disk: String,
    #[serde(default = "default_volume_mount")]
    pub volume_mount: String,
    #[serde(default = "default_security_profile")]
    pub security_profile: String,
    #[serde(default)]
    pub env: Vec<EnvVar>,
    #[serde(default)]
    pub enable_docker: bool,
    #[serde(default = "default_ports")]
    pub ports: Vec<PortSpec>,
    /// Optional command override for the container entrypoint.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<String>,
    /// SSH public key to inject into the agent container.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_pubkey: Option<String>,
    /// CrabShack service type (e.g. "ironclaw").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_type: Option<String>,
    /// Target node ID for scheduling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AgentState {
    Running,
    Stopped,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PortSpec {
    pub name: String,
    pub port: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NodePort {
    pub name: String,
    pub port: i32,
    pub node_port: i32,
}

fn default_state() -> AgentState {
    AgentState::Running
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

pub fn generate_crd() -> k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition {
    use kube::CustomResourceExt;
    Agent::crd()
}
