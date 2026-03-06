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
    /// Environment variables to set on the agent pod.
    #[serde(default)]
    pub env: Vec<EnvVar>,
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
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
}
