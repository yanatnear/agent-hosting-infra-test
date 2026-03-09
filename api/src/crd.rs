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
    #[serde(default)]
    pub enable_docker: bool,
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

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Constants
    // -----------------------------------------------------------------------

    const DEFAULT_CPU: &str = "1";
    const DEFAULT_MEMORY: &str = "4Gi";
    const DEFAULT_DISK: &str = "10Gi";

    // -----------------------------------------------------------------------
    // AgentSpec serde tests
    // -----------------------------------------------------------------------

    /// **Test Case #5 — Serde defaults for minimal JSON input**
    ///
    /// WHY THIS MATTERS:
    /// The API accepts a minimal creation payload with just `image`. All other
    /// fields must use sensible defaults. If serde defaults break, agents get
    /// created with empty strings for CPU/memory/disk, causing pod scheduling
    /// failures with cryptic Kubernetes errors.
    ///
    /// WHAT THIS TEST DOES:
    /// 1. Deserializes a minimal JSON with only `{"image": "test:v1"}`
    /// 2. Asserts all defaulted fields have the expected values
    /// 3. Asserts env defaults to an empty vec
    ///
    /// IF THIS FAILS:
    /// Agents created without explicit resource specs will have invalid
    /// configuration, failing to schedule on any node.
    ///
    /// WHAT IS BEING TESTED:
    /// AgentSpec `#[serde(default)]` annotations — pure deserialization.
    #[test]
    fn p1_agent_spec_serde_defaults_applied_for_minimal_json() {
        let json = r#"{"image": "test:v1"}"#;

        let spec: AgentSpec = serde_json::from_str(json).expect("minimal JSON must deserialize");

        assert_eq!(spec.image, "test:v1", "image must match input");
        assert_eq!(spec.cpu, DEFAULT_CPU, "CPU must default to '{}'", DEFAULT_CPU);
        assert_eq!(
            spec.memory, DEFAULT_MEMORY,
            "memory must default to '{}'",
            DEFAULT_MEMORY
        );
        assert_eq!(
            spec.disk, DEFAULT_DISK,
            "disk must default to '{}'",
            DEFAULT_DISK
        );
        assert_eq!(
            spec.state,
            AgentState::Running,
            "state must default to Running"
        );
        assert!(
            spec.env.is_empty(),
            "env must default to empty vec"
        );
    }

    /// **Test Cases #9, #10 — AgentState serialization roundtrip**
    ///
    /// WHY THIS MATTERS:
    /// The operator patches Agent state via JSON merge patches using lowercase
    /// strings ("running", "stopped"). If serde rename_all breaks, the operator
    /// writes "Running" but reads "running", causing deserialization failures
    /// that prevent all state transitions.
    ///
    /// WHAT THIS TEST DOES:
    /// 1. Serializes Running and Stopped variants to JSON
    /// 2. Asserts they produce lowercase strings
    /// 3. Deserializes the lowercase strings back
    /// 4. Asserts roundtrip equality
    ///
    /// IF THIS FAILS:
    /// All start/stop operations will fail. The API sends "running"/"stopped"
    /// but the CRD expects a different casing, causing 422 errors from the
    /// Kubernetes API server.
    ///
    /// WHAT IS BEING TESTED:
    /// AgentState `#[serde(rename_all = "lowercase")]` — pure serde logic.
    #[test]
    fn p1_agent_state_serializes_lowercase_and_roundtrips() {
        // Serialize
        let running_json = serde_json::to_string(&AgentState::Running)
            .expect("Running must serialize");
        let stopped_json = serde_json::to_string(&AgentState::Stopped)
            .expect("Stopped must serialize");

        assert_eq!(running_json, r#""running""#, "Running must serialize to lowercase");
        assert_eq!(stopped_json, r#""stopped""#, "Stopped must serialize to lowercase");

        // Roundtrip
        let running: AgentState = serde_json::from_str(&running_json)
            .expect("must deserialize 'running'");
        let stopped: AgentState = serde_json::from_str(&stopped_json)
            .expect("must deserialize 'stopped'");

        assert_eq!(running, AgentState::Running, "roundtrip must preserve Running");
        assert_eq!(stopped, AgentState::Stopped, "roundtrip must preserve Stopped");
    }
}
