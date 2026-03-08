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
