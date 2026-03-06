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
    #[serde(default)]
    pub env: Vec<EnvVar>,
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

pub fn generate_crd() -> k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition {
    use kube::CustomResourceExt;
    Agent::crd()
}
