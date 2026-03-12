use std::sync::Arc;

use k8s_openapi::api::core::v1::{PersistentVolumeClaim, Pod, Service};
use k8s_openapi::api::networking::v1::NetworkPolicy;
use kube::api::{Api, Patch, PatchParams, PostParams};
use kube::runtime::controller::Action;
use kube::{Client, ResourceExt};
use tracing::{info, warn};

use crate::crd::{Agent, AgentState, AgentStatus, NodePort};
use crate::error::{Error, Result};
use crate::resources;

/// Shared state for the controller.
pub struct Context {
    pub client: Client,
    pub namespace: String,
}

/// Main reconciliation function called by the controller runtime.
pub async fn reconcile(agent: Arc<Agent>, ctx: Arc<Context>) -> Result<Action> {
    let client = &ctx.client;
    let ns = agent
        .namespace()
        .unwrap_or_else(|| ctx.namespace.clone());
    let name = agent.name_any();

    info!("Reconciling agent: {}/{}", ns, name);

    // 1. Ensure PVC exists
    reconcile_pvc(client, &ns, &agent).await?;

    // 2. Reconcile Pod based on desired state
    if agent.spec.state == AgentState::Stopped {
        // Delete the pod if it exists, but keep the PVC
        delete_pod_if_exists(client, &ns, &name).await?;
    } else {
        // Ensure the pod exists and matches the spec
        reconcile_pod(client, &ns, &agent).await?;
    }

    // 3. Ensure NetworkPolicy exists
    reconcile_network_policy(client, &ns, &agent).await?;

    // 4. Ensure Service exists
    reconcile_service(client, &ns, &agent).await?;

    // 5. Update Agent status from Pod status
    update_status(client, &ns, &agent).await?;

    // Requeue after 30 seconds for periodic reconciliation
    Ok(Action::requeue(std::time::Duration::from_secs(30)))
}

/// Error policy: requeue after a delay on error.
pub fn error_policy(agent: Arc<Agent>, error: &Error, _ctx: Arc<Context>) -> Action {
    warn!(
        "Reconciliation error for agent {}: {:?}",
        agent.name_any(),
        error
    );
    Action::requeue(std::time::Duration::from_secs(15))
}

// ---------------------------------------------------------------------------
// PVC
// ---------------------------------------------------------------------------

async fn reconcile_pvc(client: &Client, ns: &str, agent: &Agent) -> Result<()> {
    let pvcs: Api<PersistentVolumeClaim> = Api::namespaced(client.clone(), ns);
    let pvc_name = format!("agent-{}-data", agent.name_any());

    match pvcs.get_opt(&pvc_name).await? {
        Some(_) => {
            info!("PVC {} already exists", pvc_name);
        }
        None => {
            info!("Creating PVC {}", pvc_name);
            let pvc = resources::build_pvc(agent);
            pvcs.create(&PostParams::default(), &pvc).await?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Pod
// ---------------------------------------------------------------------------

async fn reconcile_pod(client: &Client, ns: &str, agent: &Agent) -> Result<()> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), ns);
    let pod_name = format!("agent-{}", agent.name_any());
    let desired = resources::build_pod(agent);

    match pods.get_opt(&pod_name).await? {
        Some(existing) => {
            // Check if the pod spec has diverged (image or resources changed)
            if pod_needs_update(&existing, agent) {
                info!("Pod {} spec changed, recreating", pod_name);
                pods.delete(&pod_name, &Default::default()).await?;
                // Wait briefly for deletion to propagate, then create
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                pods.create(&PostParams::default(), &desired).await?;
            } else {
                info!("Pod {} is up to date", pod_name);
            }
        }
        None => {
            info!("Creating Pod {}", pod_name);
            pods.create(&PostParams::default(), &desired).await?;
        }
    }
    Ok(())
}

fn pod_needs_update(existing: &Pod, agent: &Agent) -> bool {
    if let Some(spec) = &existing.spec {
        if let Some(container) = spec.containers.first() {
            // Check image
            if container.image.as_deref() != Some(&agent.spec.image) {
                return true;
            }
            // Check resource limits
            if let Some(resources) = &container.resources {
                if let Some(limits) = &resources.limits {
                    let cpu_matches = limits
                        .get("cpu")
                        .map(|q| q.0 == agent.spec.cpu)
                        .unwrap_or(false);
                    let mem_matches = limits
                        .get("memory")
                        .map(|q| q.0 == agent.spec.memory)
                        .unwrap_or(false);
                    if !cpu_matches || !mem_matches {
                        return true;
                    }
                } else {
                    return true;
                }
            } else {
                return true;
            }
        }
    }
    false
}

async fn delete_pod_if_exists(client: &Client, ns: &str, agent_name: &str) -> Result<()> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), ns);
    let pod_name = format!("agent-{}", agent_name);

    match pods.get_opt(&pod_name).await? {
        Some(_) => {
            info!("Deleting Pod {} (agent stopped)", pod_name);
            pods.delete(&pod_name, &Default::default()).await?;
        }
        None => {
            info!("Pod {} already absent (agent stopped)", pod_name);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// NetworkPolicy
// ---------------------------------------------------------------------------

async fn reconcile_network_policy(client: &Client, ns: &str, agent: &Agent) -> Result<()> {
    let netpols: Api<NetworkPolicy> = Api::namespaced(client.clone(), ns);
    let np_name = format!("agent-{}-isolation", agent.name_any());

    match netpols.get_opt(&np_name).await? {
        Some(_) => {
            info!("NetworkPolicy {} already exists", np_name);
        }
        None => {
            info!("Creating NetworkPolicy {}", np_name);
            let np = resources::build_network_policy(agent);
            netpols.create(&PostParams::default(), &np).await?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

async fn reconcile_service(client: &Client, ns: &str, agent: &Agent) -> Result<()> {
    let services: Api<Service> = Api::namespaced(client.clone(), ns);
    let svc_name = format!("agent-{}", agent.name_any());

    match services.get_opt(&svc_name).await? {
        Some(_) => {
            info!("Service {} already exists", svc_name);
        }
        None => {
            info!("Creating Service {}", svc_name);
            let svc = resources::build_service(agent);
            services.create(&PostParams::default(), &svc).await?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

async fn update_status(client: &Client, ns: &str, agent: &Agent) -> Result<()> {
    let agents: Api<Agent> = Api::namespaced(client.clone(), ns);
    let pods: Api<Pod> = Api::namespaced(client.clone(), ns);
    let services: Api<Service> = Api::namespaced(client.clone(), ns);
    let agent_name = agent.name_any();
    let pod_name = format!("agent-{}", agent_name);
    let svc_name = format!("agent-{}", agent_name);

    let mut status = if agent.spec.state == AgentState::Stopped {
        AgentStatus {
            phase: Some("Stopped".to_string()),
            host_node: None,
            pod_ip: None,
            restart_count: Some(0),
            ..Default::default()
        }
    } else {
        match pods.get_opt(&pod_name).await? {
            Some(pod) => derive_status_from_pod(&pod),
            None => AgentStatus {
                phase: Some("Creating".to_string()),
                ..Default::default()
            },
        }
    };

    // Read NodePort assignments from the Service
    if let Ok(Some(svc)) = services.get_opt(&svc_name).await {
        if let Some(spec) = &svc.spec {
            if let Some(ports) = &spec.ports {
                status.node_ports = ports
                    .iter()
                    .filter_map(|sp| {
                        Some(NodePort {
                            name: sp.name.clone().unwrap_or_default(),
                            port: sp.port,
                            node_port: sp.node_port?,
                        })
                    })
                    .collect();
            }
        }
    }

    let phase_str = status.phase.clone().unwrap_or_default();
    let patch = serde_json::json!({ "status": status });
    agents
        .patch_status(
            &agent_name,
            &PatchParams::apply("nearai-agent-operator"),
            &Patch::Merge(&patch),
        )
        .await?;

    info!("Updated status for agent {}: phase={}", agent_name, phase_str);
    Ok(())
}

fn derive_status_from_pod(pod: &Pod) -> AgentStatus {
    let pod_status = pod.status.as_ref();

    // Calculate restart count from container statuses
    let restart_count = pod_status
        .and_then(|s| s.container_statuses.as_ref())
        .map(|statuses| statuses.iter().map(|cs| cs.restart_count).sum())
        .unwrap_or(0);

    // Detect CrashLoopBackOff
    let is_crash_loop = pod_status
        .and_then(|s| s.container_statuses.as_ref())
        .map(|statuses| {
            statuses.iter().any(|cs| {
                cs.state
                    .as_ref()
                    .and_then(|s| s.waiting.as_ref())
                    .and_then(|w| w.reason.as_ref())
                    .map(|r| r == "CrashLoopBackOff")
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false);

    let phase = if is_crash_loop {
        "CrashLoopBackOff".to_string()
    } else {
        match pod_status.and_then(|s| s.phase.as_deref()) {
            Some("Pending") => "Creating".to_string(),
            Some("Running") => "Running".to_string(),
            Some("Succeeded") => "Stopped".to_string(),
            Some("Failed") => "Failed".to_string(),
            Some(other) => other.to_string(),
            None => "Unknown".to_string(),
        }
    };

    let host_node = pod
        .spec
        .as_ref()
        .and_then(|s| s.node_name.clone());

    let pod_ip = pod_status.and_then(|s| s.pod_ip.clone());

    AgentStatus {
        phase: Some(phase),
        host_node,
        pod_ip,
        restart_count: Some(restart_count),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::{AgentSpec, AgentState};
    use k8s_openapi::api::core::v1::{
        Container, ContainerState, ContainerStateWaiting, ContainerStatus, Pod, PodSpec,
        PodStatus, ResourceRequirements,
    };
    use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
    use std::collections::BTreeMap;

    // -----------------------------------------------------------------------
    // Constants
    // -----------------------------------------------------------------------

    const TEST_IMAGE: &str = "registry.example.com/agent:v1.2.3";
    const TEST_IMAGE_UPDATED: &str = "registry.example.com/agent:v2.0.0";
    const TEST_CPU: &str = "2";
    const TEST_CPU_UPDATED: &str = "4";
    const TEST_MEMORY: &str = "8Gi";
    const TEST_MEMORY_UPDATED: &str = "16Gi";
    const TEST_DISK: &str = "20Gi";
    const TEST_POD_IP: &str = "10.42.0.15";
    const TEST_NODE: &str = "worker-01";

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    /// Creates a minimal Agent for testing pod_needs_update().
    fn make_agent(image: &str, cpu: &str, memory: &str) -> Agent {
        let mut agent = Agent::new(
            "test",
            AgentSpec {
                enable_docker: false,
                image: image.to_string(),
                state: AgentState::Running,
                cpu: cpu.to_string(),
                memory: memory.to_string(),
                disk: TEST_DISK.to_string(),
                volume_mount: "/home/agent".to_string(),
                security_profile: "restricted".to_string(),
                env: vec![],
                ports: vec![],
                command: vec![],
            },
        );
        agent.metadata.uid = Some("test-uid".to_string());
        agent.metadata.namespace = Some("agents".to_string());
        agent
    }

    /// Creates a Pod with the given container image and resource limits.
    /// Mimics what Kubernetes returns for a running pod.
    fn make_pod_with_resources(image: &str, cpu: &str, memory: &str) -> Pod {
        let mut limits = BTreeMap::new();
        limits.insert("cpu".to_string(), Quantity(cpu.to_string()));
        limits.insert("memory".to_string(), Quantity(memory.to_string()));

        Pod {
            spec: Some(PodSpec {
                containers: vec![Container {
                    name: "agent".to_string(),
                    image: Some(image.to_string()),
                    resources: Some(ResourceRequirements {
                        limits: Some(limits),
                        ..Default::default()
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// Creates a Pod with realistic status fields for testing derive_status_from_pod().
    fn make_pod_with_status(
        phase: &str,
        pod_ip: Option<&str>,
        node_name: Option<&str>,
        restart_count: i32,
    ) -> Pod {
        Pod {
            spec: Some(PodSpec {
                node_name: node_name.map(String::from),
                containers: vec![Container {
                    name: "agent".to_string(),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            status: Some(PodStatus {
                phase: Some(phase.to_string()),
                pod_ip: pod_ip.map(String::from),
                container_statuses: Some(vec![ContainerStatus {
                    name: "agent".to_string(),
                    restart_count,
                    ready: phase == "Running",
                    image: TEST_IMAGE.to_string(),
                    image_id: String::new(),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// Creates a Pod in CrashLoopBackOff state for testing crash detection.
    fn make_crashloop_pod(restart_count: i32) -> Pod {
        Pod {
            spec: Some(PodSpec {
                node_name: Some(TEST_NODE.to_string()),
                containers: vec![Container {
                    name: "agent".to_string(),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            status: Some(PodStatus {
                phase: Some("Running".to_string()),
                pod_ip: Some(TEST_POD_IP.to_string()),
                container_statuses: Some(vec![ContainerStatus {
                    name: "agent".to_string(),
                    restart_count,
                    ready: false,
                    image: TEST_IMAGE.to_string(),
                    image_id: String::new(),
                    state: Some(ContainerState {
                        waiting: Some(ContainerStateWaiting {
                            reason: Some("CrashLoopBackOff".to_string()),
                            message: Some("back-off 5m0s restarting".to_string()),
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    // -----------------------------------------------------------------------
    // pod_needs_update tests
    // -----------------------------------------------------------------------

    /// **Test Cases #11, #13 — No update needed when spec matches**
    ///
    /// WHY THIS MATTERS:
    /// The controller must not needlessly delete and recreate pods when nothing
    /// has changed. Unnecessary recreation causes downtime and restarts the
    /// agent's SSH session.
    ///
    /// WHAT THIS TEST DOES:
    /// 1. Creates a pod and agent with identical image, CPU, and memory
    /// 2. Calls pod_needs_update()
    /// 3. Asserts it returns false
    ///
    /// IF THIS FAILS:
    /// Pods will be recreated on every reconciliation loop (every 30s), causing
    /// constant agent restarts and an unusable system.
    ///
    /// WHAT IS BEING TESTED:
    /// `pod_needs_update()` — private function, tested via inline module.
    #[test]
    fn p1_pod_needs_update_returns_false_when_spec_matches() {
        let agent = make_agent(TEST_IMAGE, TEST_CPU, TEST_MEMORY);
        let pod = make_pod_with_resources(TEST_IMAGE, TEST_CPU, TEST_MEMORY);

        let needs_update = pod_needs_update(&pod, &agent);

        assert!(
            !needs_update,
            "pod_needs_update must return false when image, CPU, and memory all match"
        );
    }

    /// **Test Case #11 — Detect image change**
    ///
    /// WHY THIS MATTERS:
    /// When a user updates their agent image (e.g., new version), the controller
    /// must detect the drift and recreate the pod with the new image.
    ///
    /// WHAT THIS TEST DOES:
    /// 1. Creates a pod with the old image and an agent with the new image
    /// 2. Calls pod_needs_update()
    /// 3. Asserts it returns true
    ///
    /// IF THIS FAILS:
    /// Image updates won't take effect until the pod is manually deleted.
    /// Users will think their deployment is stuck on the old version.
    ///
    /// WHAT IS BEING TESTED:
    /// `pod_needs_update()` image comparison — private function.
    #[test]
    fn p1_pod_needs_update_detects_image_change() {
        let agent = make_agent(TEST_IMAGE_UPDATED, TEST_CPU, TEST_MEMORY);
        let pod = make_pod_with_resources(TEST_IMAGE, TEST_CPU, TEST_MEMORY);

        let needs_update = pod_needs_update(&pod, &agent);

        assert!(
            needs_update,
            "pod_needs_update must return true when image has changed"
        );
    }

    /// **Test Case #11 — Detect CPU change**
    ///
    /// WHY THIS MATTERS:
    /// CPU resource changes require pod recreation (Kubernetes does not support
    /// in-place resource updates for most fields). The controller must detect
    /// this drift.
    ///
    /// WHAT THIS TEST DOES:
    /// 1. Creates a pod with old CPU and an agent with new CPU
    /// 2. Calls pod_needs_update()
    /// 3. Asserts it returns true
    ///
    /// IF THIS FAILS:
    /// CPU changes won't take effect. The pod continues running with the old
    /// resource allocation, potentially causing OOM or throttling.
    ///
    /// WHAT IS BEING TESTED:
    /// `pod_needs_update()` CPU comparison — private function.
    #[test]
    fn p1_pod_needs_update_detects_cpu_change() {
        let agent = make_agent(TEST_IMAGE, TEST_CPU_UPDATED, TEST_MEMORY);
        let pod = make_pod_with_resources(TEST_IMAGE, TEST_CPU, TEST_MEMORY);

        let needs_update = pod_needs_update(&pod, &agent);

        assert!(
            needs_update,
            "pod_needs_update must return true when CPU has changed"
        );
    }

    /// **Test Case #11 — Detect memory change**
    ///
    /// WHY THIS MATTERS:
    /// Memory resource changes require pod recreation. The controller must
    /// detect this drift to apply the new memory allocation.
    ///
    /// WHAT THIS TEST DOES:
    /// 1. Creates a pod with old memory and an agent with new memory
    /// 2. Calls pod_needs_update()
    /// 3. Asserts it returns true
    ///
    /// IF THIS FAILS:
    /// Memory changes won't take effect. The pod runs with old memory limits,
    /// risking OOMKilled or wasted cluster resources.
    ///
    /// WHAT IS BEING TESTED:
    /// `pod_needs_update()` memory comparison — private function.
    #[test]
    fn p1_pod_needs_update_detects_memory_change() {
        let agent = make_agent(TEST_IMAGE, TEST_CPU, TEST_MEMORY_UPDATED);
        let pod = make_pod_with_resources(TEST_IMAGE, TEST_CPU, TEST_MEMORY);

        let needs_update = pod_needs_update(&pod, &agent);

        assert!(
            needs_update,
            "pod_needs_update must return true when memory has changed"
        );
    }

    // -----------------------------------------------------------------------
    // derive_status_from_pod tests
    // -----------------------------------------------------------------------

    /// **Test Cases #12, #13 — Running pod produces correct status**
    ///
    /// WHY THIS MATTERS:
    /// The Agent CRD's status subresource drives the API response and kubectl
    /// output. If the status mapping is wrong, users see incorrect phase, IP,
    /// or node information — making debugging impossible.
    ///
    /// WHAT THIS TEST DOES:
    /// 1. Creates a pod with phase="Running", a pod IP, and a node name
    /// 2. Calls derive_status_from_pod()
    /// 3. Asserts phase="Running", pod_ip and host_node are populated,
    ///    restart_count is correct
    ///
    /// IF THIS FAILS:
    /// Running agents will show wrong status in the API. Users cannot determine
    /// which node their agent is on or what its IP address is.
    ///
    /// WHAT IS BEING TESTED:
    /// `derive_status_from_pod()` — private function.
    #[test]
    fn p0_derive_status_running_pod_extracts_all_fields() {
        let pod = make_pod_with_status("Running", Some(TEST_POD_IP), Some(TEST_NODE), 2);

        let status = derive_status_from_pod(&pod);

        assert_eq!(
            status.phase.as_deref(),
            Some("Running"),
            "Running pod phase must map to 'Running'"
        );
        assert_eq!(
            status.pod_ip.as_deref(),
            Some(TEST_POD_IP),
            "must extract pod IP from pod status"
        );
        assert_eq!(
            status.host_node.as_deref(),
            Some(TEST_NODE),
            "must extract node name from pod spec"
        );
        assert_eq!(
            status.restart_count,
            Some(2),
            "must sum restart counts from container statuses"
        );
    }

    /// **Test Case #7 — CrashLoopBackOff detection**
    ///
    /// WHY THIS MATTERS:
    /// When an agent crashes repeatedly, Kubernetes puts it in CrashLoopBackOff.
    /// The operator must detect this and surface it in the Agent status so users
    /// can see their agent is broken rather than silently restarting.
    ///
    /// WHAT THIS TEST DOES:
    /// 1. Creates a pod with a container in CrashLoopBackOff waiting state
    /// 2. Calls derive_status_from_pod()
    /// 3. Asserts phase="CrashLoopBackOff" and restart_count is populated
    ///
    /// IF THIS FAILS:
    /// Crash-looping agents will incorrectly show as "Running" in the API,
    /// hiding a critical failure from users.
    ///
    /// WHAT IS BEING TESTED:
    /// `derive_status_from_pod()` CrashLoopBackOff path — private function.
    #[test]
    fn p0_derive_status_crashloop_detected_from_waiting_reason() {
        let pod = make_crashloop_pod(5);

        let status = derive_status_from_pod(&pod);

        assert_eq!(
            status.phase.as_deref(),
            Some("CrashLoopBackOff"),
            "CrashLoopBackOff waiting reason must override the pod phase"
        );
        assert_eq!(
            status.restart_count,
            Some(5),
            "restart count must reflect the container's restart count"
        );
    }

    /// **Test Case #12 — Succeeded pod maps to Stopped**
    ///
    /// WHY THIS MATTERS:
    /// A pod that exits successfully (phase=Succeeded) should be surfaced as
    /// "Stopped" in the Agent status, since the agent is no longer running
    /// but completed normally.
    ///
    /// WHAT THIS TEST DOES:
    /// 1. Creates a pod with phase="Succeeded"
    /// 2. Calls derive_status_from_pod()
    /// 3. Asserts phase="Stopped"
    ///
    /// IF THIS FAILS:
    /// Completed agents will show as "Succeeded" instead of "Stopped", creating
    /// inconsistency between the Agent CRD states and the status display.
    ///
    /// WHAT IS BEING TESTED:
    /// `derive_status_from_pod()` Succeeded→Stopped mapping — private function.
    #[test]
    fn p2_derive_status_succeeded_maps_to_stopped() {
        let pod = make_pod_with_status("Succeeded", None, Some(TEST_NODE), 0);

        let status = derive_status_from_pod(&pod);

        assert_eq!(
            status.phase.as_deref(),
            Some("Stopped"),
            "Succeeded pod phase must map to 'Stopped'"
        );
    }
}
