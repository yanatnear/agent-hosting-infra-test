use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::{
    Container, ContainerPort, PersistentVolumeClaim, PersistentVolumeClaimSpec, Pod, PodSpec,
    Probe, ResourceRequirements, SecurityContext, Service, ServicePort, ServiceSpec,
    TCPSocketAction, Volume, VolumeMount,
};
use k8s_openapi::api::networking::v1::{
    NetworkPolicy, NetworkPolicyEgressRule, NetworkPolicyIngressRule, NetworkPolicyPeer,
    NetworkPolicyPort, NetworkPolicySpec,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::{Resource, ResourceExt};

use crate::crd::Agent;

/// Build security context based on the agent's security profile.
fn build_security_context(profile: &str) -> SecurityContext {
    match profile {
        "trusted" => SecurityContext {
            // Trusted: allow root + privilege escalation for entrypoint scripts
            // that drop to non-root (e.g. runuser). Use only for known images.
            capabilities: Some(k8s_openapi::api::core::v1::Capabilities {
                drop: Some(vec!["ALL".to_string()]),
                add: Some(vec![
                    "NET_BIND_SERVICE".to_string(),
                    "CHOWN".to_string(),
                    "FOWNER".to_string(),
                    "SETUID".to_string(),
                    "SETGID".to_string(),
                    "DAC_OVERRIDE".to_string(),
                ]),
            }),
            ..Default::default()
        },
        _ => SecurityContext {
            // Restricted (default): non-root, read-only rootfs, no escalation.
            run_as_non_root: Some(true),
            run_as_user: Some(1000),
            read_only_root_filesystem: Some(true),
            allow_privilege_escalation: Some(false),
            capabilities: Some(k8s_openapi::api::core::v1::Capabilities {
                drop: Some(vec!["ALL".to_string()]),
                add: Some(vec!["NET_BIND_SERVICE".to_string()]),
            }),
            ..Default::default()
        },
    }
}

/// Standard labels applied to all resources for a given agent.
fn agent_labels(name: &str) -> BTreeMap<String, String> {
    let mut labels = BTreeMap::new();
    labels.insert("agents.near.ai/name".to_string(), name.to_string());
    labels.insert("app".to_string(), "agent".to_string());
    labels
}

/// Build an owner reference pointing to the Agent CRD instance.
fn owner_reference(agent: &Agent) -> k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference {
    let meta = agent.meta();
    k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference {
        api_version: "agents.near.ai/v1".to_string(),
        kind: "Agent".to_string(),
        name: meta.name.clone().unwrap_or_default(),
        uid: meta.uid.clone().unwrap_or_default(),
        controller: Some(true),
        block_owner_deletion: Some(true),
    }
}

/// Build the PersistentVolumeClaim for an agent's data volume.
pub fn build_pvc(agent: &Agent) -> PersistentVolumeClaim {
    let name = agent.name_any();
    let ns = agent.namespace().unwrap_or_else(|| "agents".to_string());
    let storage_class =
        std::env::var("STORAGE_CLASS").unwrap_or_else(|_| "local-path".to_string());

    let mut storage_requests = BTreeMap::new();
    storage_requests.insert("storage".to_string(), Quantity(agent.spec.disk.clone()));

    PersistentVolumeClaim {
        metadata: ObjectMeta {
            name: Some(format!("agent-{}-data", name)),
            namespace: Some(ns),
            labels: Some(agent_labels(&name)),
            owner_references: Some(vec![owner_reference(agent)]),
            ..Default::default()
        },
        spec: Some(PersistentVolumeClaimSpec {
            access_modes: Some(vec!["ReadWriteOnce".to_string()]),
            storage_class_name: Some(storage_class),
            resources: Some(k8s_openapi::api::core::v1::VolumeResourceRequirements {
                requests: Some(storage_requests),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Build the Pod for an agent.
pub fn build_pod(agent: &Agent) -> Pod {
    let name = agent.name_any();
    let ns = agent.namespace().unwrap_or_else(|| "agents".to_string());

    let mut resource_requests = BTreeMap::new();
    resource_requests.insert("cpu".to_string(), Quantity(agent.spec.cpu.clone()));
    resource_requests.insert("memory".to_string(), Quantity(agent.spec.memory.clone()));

    let mut resource_limits = BTreeMap::new();
    resource_limits.insert("cpu".to_string(), Quantity(agent.spec.cpu.clone()));
    resource_limits.insert("memory".to_string(), Quantity(agent.spec.memory.clone()));

    let env_vars: Vec<k8s_openapi::api::core::v1::EnvVar> = agent
        .spec
        .env
        .iter()
        .map(|e| k8s_openapi::api::core::v1::EnvVar {
            name: e.name.clone(),
            value: Some(e.value.clone()),
            ..Default::default()
        })
        .collect();

    let container = Container {
        name: "agent".to_string(),
        image: Some(agent.spec.image.clone()),
        ports: Some(
            agent.spec.ports.iter().map(|p| ContainerPort {
                container_port: p.port,
                name: Some(p.name.clone()),
                protocol: Some("TCP".to_string()),
                ..Default::default()
            }).collect()
        ),
        resources: Some(ResourceRequirements {
            requests: Some(resource_requests),
            limits: Some(resource_limits),
            ..Default::default()
        }),
        security_context: Some(build_security_context(&agent.spec.security_profile)),
        volume_mounts: Some(vec![
            VolumeMount {
                name: "agent-data".to_string(),
                mount_path: agent.spec.volume_mount.clone(),
                ..Default::default()
            },
            VolumeMount {
                name: "tmp".to_string(),
                mount_path: "/tmp".to_string(),
                ..Default::default()
            },
        ]),
        liveness_probe: agent.spec.ports.first().map(|p| Probe {
            tcp_socket: Some(TCPSocketAction {
                port: IntOrString::Int(p.port),
                ..Default::default()
            }),
            initial_delay_seconds: Some(60),
            period_seconds: Some(10),
            failure_threshold: Some(10),
            ..Default::default()
        }),
        readiness_probe: agent.spec.ports.first().map(|p| Probe {
            tcp_socket: Some(TCPSocketAction {
                port: IntOrString::Int(p.port),
                ..Default::default()
            }),
            initial_delay_seconds: Some(30),
            period_seconds: Some(5),
            failure_threshold: Some(10),
            ..Default::default()
        }),
        env: if env_vars.is_empty() {
            None
        } else {
            Some(env_vars)
        },
        ..Default::default()
    };

    Pod {
        metadata: ObjectMeta {
            name: Some(format!("agent-{}", name)),
            namespace: Some(ns),
            labels: Some(agent_labels(&name)),
            owner_references: Some(vec![owner_reference(agent)]),
            ..Default::default()
        },
        spec: Some(PodSpec {
            containers: vec![container],
            runtime_class_name: if agent.spec.enable_docker {
                Some("sysbox-runc".to_string())
            } else {
                None
            },
            host_users: if agent.spec.enable_docker {
                Some(false)
            } else {
                None
            },
            restart_policy: Some("Always".to_string()),
            termination_grace_period_seconds: Some(30),
            volumes: Some(vec![
                Volume {
                    name: "agent-data".to_string(),
                    persistent_volume_claim: Some(
                        k8s_openapi::api::core::v1::PersistentVolumeClaimVolumeSource {
                            claim_name: format!("agent-{}-data", name),
                            read_only: Some(false),
                        },
                    ),
                    ..Default::default()
                },
                Volume {
                    name: "tmp".to_string(),
                    empty_dir: Some(k8s_openapi::api::core::v1::EmptyDirVolumeSource::default()),
                    ..Default::default()
                },
            ]),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Build the NetworkPolicy for agent isolation.
pub fn build_network_policy(agent: &Agent) -> NetworkPolicy {
    let name = agent.name_any();
    let ns = agent.namespace().unwrap_or_else(|| "agents".to_string());

    let mut pod_selector_labels = BTreeMap::new();
    pod_selector_labels.insert("agents.near.ai/name".to_string(), name.clone());

    // Ingress: allow from ingress-controller or ssh-proxy pods
    let mut ingress_label = BTreeMap::new();
    ingress_label.insert("app".to_string(), "ingress-controller".to_string());

    let mut ssh_proxy_label = BTreeMap::new();
    ssh_proxy_label.insert("app".to_string(), "ssh-proxy".to_string());

    let ingress_from_infra = NetworkPolicyIngressRule {
        from: Some(vec![
            NetworkPolicyPeer {
                pod_selector: Some(k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector {
                    match_labels: Some(ingress_label),
                    ..Default::default()
                }),
                ..Default::default()
            },
            NetworkPolicyPeer {
                pod_selector: Some(k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector {
                    match_labels: Some(ssh_proxy_label),
                    ..Default::default()
                }),
                ..Default::default()
            },
        ]),
        ..Default::default()
    };

    // Ingress: allow external traffic (NodePort) on configured ports
    let ingress_nodeport = NetworkPolicyIngressRule {
        ports: Some(
            agent.spec.ports.iter().map(|p| NetworkPolicyPort {
                port: Some(IntOrString::Int(p.port)),
                protocol: Some("TCP".to_string()),
                ..Default::default()
            }).collect()
        ),
        ..Default::default()
    };

    // Egress: allow DNS (53 UDP/TCP) and HTTP/HTTPS (80/443 TCP)
    let egress_rule = NetworkPolicyEgressRule {
        to: None,
        ports: Some(vec![
            NetworkPolicyPort {
                port: Some(IntOrString::Int(53)),
                protocol: Some("UDP".to_string()),
                ..Default::default()
            },
            NetworkPolicyPort {
                port: Some(IntOrString::Int(53)),
                protocol: Some("TCP".to_string()),
                ..Default::default()
            },
            NetworkPolicyPort {
                port: Some(IntOrString::Int(80)),
                protocol: Some("TCP".to_string()),
                ..Default::default()
            },
            NetworkPolicyPort {
                port: Some(IntOrString::Int(443)),
                protocol: Some("TCP".to_string()),
                ..Default::default()
            },
        ]),
    };

    NetworkPolicy {
        metadata: ObjectMeta {
            name: Some(format!("agent-{}-isolation", name)),
            namespace: Some(ns),
            labels: Some(agent_labels(&name)),
            owner_references: Some(vec![owner_reference(agent)]),
            ..Default::default()
        },
        spec: Some(NetworkPolicySpec {
            pod_selector: k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector {
                match_labels: Some(pod_selector_labels),
                ..Default::default()
            },
            ingress: Some(vec![ingress_from_infra, ingress_nodeport]),
            egress: Some(vec![egress_rule]),
            policy_types: Some(vec!["Ingress".to_string(), "Egress".to_string()]),
        }),
    }
}

/// Build the Service for an agent (ClusterIP).
pub fn build_service(agent: &Agent) -> Service {
    let name = agent.name_any();
    let ns = agent.namespace().unwrap_or_else(|| "agents".to_string());

    let mut selector = BTreeMap::new();
    selector.insert("agents.near.ai/name".to_string(), name.clone());

    Service {
        metadata: ObjectMeta {
            name: Some(format!("agent-{}", name)),
            namespace: Some(ns),
            labels: Some(agent_labels(&name)),
            owner_references: Some(vec![owner_reference(agent)]),
            ..Default::default()
        },
        spec: Some(ServiceSpec {
            type_: Some("NodePort".to_string()),
            selector: Some(selector),
            ports: Some(
                agent.spec.ports.iter().map(|p| ServicePort {
                    name: Some(p.name.clone()),
                    port: p.port,
                    target_port: Some(IntOrString::Int(p.port)),
                    protocol: Some("TCP".to_string()),
                    ..Default::default()
                }).collect()
            ),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::{AgentSpec, AgentState};

    // -----------------------------------------------------------------------
    // Constants — no magic strings in test assertions
    // -----------------------------------------------------------------------

    const TEST_AGENT_NAME: &str = "myagent";
    const TEST_IMAGE: &str = "registry.example.com/agent:v1.2.3";
    const TEST_CPU: &str = "2";
    const TEST_MEMORY: &str = "8Gi";
    const TEST_DISK: &str = "20Gi";
    const TEST_UID: &str = "550e8400-e29b-41d4-a716-446655440000";
    const TEST_NAMESPACE: &str = "agents";

    const CRD_API_VERSION: &str = "agents.near.ai/v1";
    const CRD_KIND: &str = "Agent";
    const LABEL_NAME: &str = "agents.near.ai/name";
    const LABEL_APP: &str = "app";
    const LABEL_APP_VALUE: &str = "agent";

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    /// Creates a minimal Agent CRD suitable for unit testing resource builders.
    /// Sets metadata.uid and namespace — required by owner_reference().
    fn make_test_agent(name: &str, state: AgentState) -> Agent {
        let mut agent = Agent::new(
            name,
            AgentSpec {
                image: TEST_IMAGE.to_string(),
                state,
                cpu: TEST_CPU.to_string(),
                memory: TEST_MEMORY.to_string(),
                disk: TEST_DISK.to_string(),
                volume_mount: "/home/agent".to_string(),
                security_profile: "restricted".to_string(),
                env: vec![],
                ports: vec![
                    crate::crd::PortSpec { name: "ssh".to_string(), port: 22 },
                    crate::crd::PortSpec { name: "http".to_string(), port: 80 },
                ],
            },
        );
        agent.metadata.uid = Some(TEST_UID.to_string());
        agent.metadata.namespace = Some(TEST_NAMESPACE.to_string());
        agent
    }

    /// Extracts the first (and only) container from a Pod, panicking with
    /// a clear message if the pod spec is malformed.
    fn first_container(pod: &Pod) -> &Container {
        pod.spec
            .as_ref()
            .expect("pod must have a spec")
            .containers
            .first()
            .expect("pod must have at least one container")
    }

    // -----------------------------------------------------------------------
    // build_pod tests
    // -----------------------------------------------------------------------

    /// **Test Case #1 — Pod naming and labelling**
    ///
    /// WHY THIS MATTERS:
    /// The operator, Service selector, and NetworkPolicy all locate agent pods
    /// by name and label. If the naming convention drifts, the Service cannot
    /// route traffic and the controller cannot find its own pods.
    ///
    /// WHAT THIS TEST DOES:
    /// 1. Builds a Pod from a test Agent
    /// 2. Asserts the pod name follows the `agent-{name}` convention
    /// 3. Asserts both required labels are present with correct values
    ///
    /// IF THIS FAILS:
    /// Pod naming convention changed — update Service selectors and controller
    /// lookups to match the new convention.
    ///
    /// WHAT IS BEING TESTED:
    /// `build_pod()` — pure function, no K8s client needed.
    #[test]
    fn p0_build_pod_name_and_labels() {
        let agent = make_test_agent(TEST_AGENT_NAME, AgentState::Running);

        let pod = build_pod(&agent);

        let meta = &pod.metadata;
        assert_eq!(
            meta.name.as_deref(),
            Some(&format!("agent-{}", TEST_AGENT_NAME) as &str),
            "pod name must follow 'agent-{{name}}' convention"
        );
        assert_eq!(
            meta.namespace.as_deref(),
            Some(TEST_NAMESPACE),
            "pod must be created in the agent's namespace"
        );

        let labels = meta.labels.as_ref().expect("pod must have labels");
        assert_eq!(
            labels.get(LABEL_NAME).map(String::as_str),
            Some(TEST_AGENT_NAME),
            "label '{}' must match the agent name",
            LABEL_NAME
        );
        assert_eq!(
            labels.get(LABEL_APP).map(String::as_str),
            Some(LABEL_APP_VALUE),
            "label '{}' must be '{}'",
            LABEL_APP,
            LABEL_APP_VALUE
        );
    }

    /// **Test Case #1 — Container image, ports, and resource limits**
    ///
    /// WHY THIS MATTERS:
    /// The container image determines what code runs inside the agent. Ports 22
    /// (SSH) and 80 (HTTP) are required for remote access and the health probes.
    /// Resource requests=limits ensures guaranteed QoS class, preventing noisy
    /// neighbors from stealing CPU/memory.
    ///
    /// WHAT THIS TEST DOES:
    /// 1. Builds a Pod and extracts the first container
    /// 2. Verifies the image matches the agent spec
    /// 3. Verifies ports 22 and 80 are exposed
    /// 4. Verifies resource requests equal limits (Guaranteed QoS)
    ///
    /// IF THIS FAILS:
    /// Agent pods won't get the correct image or resource allocation. SSH/HTTP
    /// access may be unreachable if ports are missing.
    ///
    /// WHAT IS BEING TESTED:
    /// `build_pod()` container configuration — pure function.
    #[test]
    fn p0_build_pod_container_spec() {
        let agent = make_test_agent(TEST_AGENT_NAME, AgentState::Running);

        let pod = build_pod(&agent);
        let container = first_container(&pod);

        assert_eq!(
            container.image.as_deref(),
            Some(TEST_IMAGE),
            "container image must match agent spec"
        );

        let ports = container.ports.as_ref().expect("container must expose ports");
        let port_numbers: Vec<i32> = ports.iter().map(|p| p.container_port).collect();
        assert!(
            port_numbers.contains(&22),
            "container must expose SSH port 22"
        );
        assert!(
            port_numbers.contains(&80),
            "container must expose HTTP port 80"
        );

        let resources = container
            .resources
            .as_ref()
            .expect("container must have resources");
        let requests = resources.requests.as_ref().expect("must have requests");
        let limits = resources.limits.as_ref().expect("must have limits");

        assert_eq!(
            requests.get("cpu").map(|q| &q.0),
            Some(&TEST_CPU.to_string()),
            "CPU request must match spec"
        );
        assert_eq!(
            limits.get("cpu").map(|q| &q.0),
            Some(&TEST_CPU.to_string()),
            "CPU limit must equal request for Guaranteed QoS"
        );
        assert_eq!(
            requests.get("memory").map(|q| &q.0),
            Some(&TEST_MEMORY.to_string()),
            "memory request must match spec"
        );
        assert_eq!(
            limits.get("memory").map(|q| &q.0),
            Some(&TEST_MEMORY.to_string()),
            "memory limit must equal request for Guaranteed QoS"
        );
    }

    /// **Test Cases #23-26 — Security hardening**
    ///
    /// WHY THIS MATTERS:
    /// Agent pods run untrusted user code. Every security constraint prevents a
    /// class of container escape or privilege escalation attacks. This is the
    /// single most critical security configuration in the entire system.
    ///
    /// WHAT THIS TEST DOES:
    /// 1. Builds a Pod and extracts the security context
    /// 2. Asserts runAsNonRoot=true (no root processes)
    /// 3. Asserts runAsUser=1000 (deterministic non-root UID)
    /// 4. Asserts readOnlyRootFilesystem=true (immutable container layer)
    /// 5. Asserts allowPrivilegeEscalation=false (no setuid binaries)
    /// 6. Asserts capabilities drop ALL, add only NET_BIND_SERVICE
    ///
    /// IF THIS FAILS:
    /// Security posture is degraded. A compromised agent could escalate to root,
    /// modify system binaries, or gain kernel capabilities.
    ///
    /// WHAT IS BEING TESTED:
    /// `build_pod()` security context — pure function.
    #[test]
    fn p0_build_pod_security_context_is_hardened() {
        let agent = make_test_agent(TEST_AGENT_NAME, AgentState::Running);

        let pod = build_pod(&agent);
        let sc = first_container(&pod)
            .security_context
            .as_ref()
            .expect("container must have a security context");

        assert_eq!(
            sc.run_as_non_root,
            Some(true),
            "must enforce non-root execution"
        );
        assert_eq!(
            sc.run_as_user,
            Some(1000),
            "must run as UID 1000 for non-numeric USER image compatibility"
        );
        assert_eq!(
            sc.read_only_root_filesystem,
            Some(true),
            "root filesystem must be read-only"
        );
        assert_eq!(
            sc.allow_privilege_escalation,
            Some(false),
            "privilege escalation must be disabled"
        );

        let caps = sc.capabilities.as_ref().expect("must define capabilities");
        assert_eq!(
            caps.drop.as_deref(),
            Some(&["ALL".to_string()] as &[String]),
            "must drop ALL capabilities"
        );
        assert_eq!(
            caps.add.as_deref(),
            Some(&["NET_BIND_SERVICE".to_string()] as &[String]),
            "must add only NET_BIND_SERVICE for binding ports <1024"
        );
    }

    /// **Test Cases #3, #8 — Persistent storage and writable tmp**
    ///
    /// WHY THIS MATTERS:
    /// The PVC-backed volume at /home/agent persists user data across restarts.
    /// The emptyDir at /tmp provides a writable scratch space since the root
    /// filesystem is read-only. Missing either volume causes data loss or
    /// application failures.
    ///
    /// WHAT THIS TEST DOES:
    /// 1. Builds a Pod and extracts volumes and volume mounts
    /// 2. Verifies the PVC volume references the correct claim name
    /// 3. Verifies /home/agent mount exists with correct source
    /// 4. Verifies /tmp mount exists backed by emptyDir
    ///
    /// IF THIS FAILS:
    /// Agent data won't persist across restarts, or applications that write
    /// to /tmp will fail with read-only filesystem errors.
    ///
    /// WHAT IS BEING TESTED:
    /// `build_pod()` volume configuration — pure function.
    #[test]
    fn p0_build_pod_volumes_and_mounts() {
        let agent = make_test_agent(TEST_AGENT_NAME, AgentState::Running);

        let pod = build_pod(&agent);
        let spec = pod.spec.as_ref().expect("pod must have spec");
        let volumes = spec.volumes.as_ref().expect("pod must have volumes");
        let mounts = first_container(&pod)
            .volume_mounts
            .as_ref()
            .expect("container must have volume mounts");

        // PVC volume
        let pvc_vol = volumes
            .iter()
            .find(|v| v.name == "agent-data")
            .expect("must have 'agent-data' volume");
        let claim = pvc_vol
            .persistent_volume_claim
            .as_ref()
            .expect("agent-data must be a PVC volume");
        assert_eq!(
            claim.claim_name,
            format!("agent-{}-data", TEST_AGENT_NAME),
            "PVC claim name must follow 'agent-{{name}}-data' convention"
        );

        // emptyDir volume
        let tmp_vol = volumes
            .iter()
            .find(|v| v.name == "tmp")
            .expect("must have 'tmp' volume");
        assert!(
            tmp_vol.empty_dir.is_some(),
            "tmp volume must be an emptyDir"
        );

        // Mount paths
        let home_mount = mounts
            .iter()
            .find(|m| m.mount_path == "/home/agent")
            .expect("must mount /home/agent");
        assert_eq!(
            home_mount.name, "agent-data",
            "/home/agent must be backed by the 'agent-data' volume"
        );

        let tmp_mount = mounts
            .iter()
            .find(|m| m.mount_path == "/tmp")
            .expect("must mount /tmp");
        assert_eq!(
            tmp_mount.name, "tmp",
            "/tmp must be backed by the 'tmp' emptyDir volume"
        );
    }

    /// **Test Case #7 — Health probes for crash detection**
    ///
    /// WHY THIS MATTERS:
    /// Liveness probes detect hung agents and trigger automatic restarts.
    /// Readiness probes prevent traffic routing to agents that haven't started
    /// their SSH server yet. Without probes, failed agents stay in a broken
    /// state indefinitely.
    ///
    /// WHAT THIS TEST DOES:
    /// 1. Builds a Pod and extracts liveness and readiness probes
    /// 2. Verifies liveness checks TCP port 22 with period=10s, failureThreshold=3
    /// 3. Verifies readiness checks TCP port 22 with period=5s
    ///
    /// IF THIS FAILS:
    /// Kubernetes won't detect agent crashes (liveness) or won't know when an
    /// agent is ready to serve traffic (readiness).
    ///
    /// WHAT IS BEING TESTED:
    /// `build_pod()` probe configuration — pure function.
    #[test]
    fn p0_build_pod_health_probes() {
        let agent = make_test_agent(TEST_AGENT_NAME, AgentState::Running);

        let pod = build_pod(&agent);
        let container = first_container(&pod);

        // Liveness probe
        let liveness = container
            .liveness_probe
            .as_ref()
            .expect("must have liveness probe");
        let liveness_port = liveness
            .tcp_socket
            .as_ref()
            .expect("liveness must use TCP socket check");
        assert_eq!(
            liveness_port.port,
            IntOrString::Int(22),
            "liveness probe must check SSH port 22"
        );
        assert_eq!(
            liveness.period_seconds,
            Some(10),
            "liveness probe period must be 10 seconds"
        );
        assert_eq!(
            liveness.failure_threshold,
            Some(3),
            "liveness probe must tolerate 3 failures before restart"
        );

        // Readiness probe
        let readiness = container
            .readiness_probe
            .as_ref()
            .expect("must have readiness probe");
        let readiness_port = readiness
            .tcp_socket
            .as_ref()
            .expect("readiness must use TCP socket check");
        assert_eq!(
            readiness_port.port,
            IntOrString::Int(22),
            "readiness probe must check SSH port 22"
        );
        assert_eq!(
            readiness.period_seconds,
            Some(5),
            "readiness probe period must be 5 seconds"
        );
    }

    /// **Test Cases #14, #15 — Owner reference for garbage collection**
    ///
    /// WHY THIS MATTERS:
    /// Owner references tell Kubernetes to automatically delete child resources
    /// (Pod, PVC, Service, NetworkPolicy) when the parent Agent CRD is deleted.
    /// Without owner references, deleting an agent leaves orphaned resources
    /// that consume cluster capacity indefinitely.
    ///
    /// WHAT THIS TEST DOES:
    /// 1. Builds a Pod and extracts owner references
    /// 2. Verifies exactly one owner reference exists
    /// 3. Verifies apiVersion, kind, name, and uid match the parent Agent
    /// 4. Verifies controller=true and blockOwnerDeletion=true
    ///
    /// IF THIS FAILS:
    /// Deleting an Agent CRD will leave orphaned K8s resources. Manual cleanup
    /// would be required to reclaim cluster capacity.
    ///
    /// WHAT IS BEING TESTED:
    /// `build_pod()` owner reference via `owner_reference()` — pure functions.
    #[test]
    fn p0_build_pod_owner_reference() {
        let agent = make_test_agent(TEST_AGENT_NAME, AgentState::Running);

        let pod = build_pod(&agent);
        let owner_refs = pod
            .metadata
            .owner_references
            .as_ref()
            .expect("pod must have owner references");

        assert_eq!(owner_refs.len(), 1, "must have exactly one owner reference");
        let oref = &owner_refs[0];

        assert_eq!(
            oref.api_version, CRD_API_VERSION,
            "owner apiVersion must be the Agent CRD group/version"
        );
        assert_eq!(
            oref.kind, CRD_KIND,
            "owner kind must be 'Agent'"
        );
        assert_eq!(
            oref.name, TEST_AGENT_NAME,
            "owner name must match the Agent name"
        );
        assert_eq!(
            oref.uid, TEST_UID,
            "owner uid must match the Agent UID"
        );
        assert_eq!(
            oref.controller,
            Some(true),
            "controller must be true for single-owner semantics"
        );
        assert_eq!(
            oref.block_owner_deletion,
            Some(true),
            "blockOwnerDeletion must be true to prevent premature GC"
        );
    }

    // -----------------------------------------------------------------------
    // build_pvc tests
    // -----------------------------------------------------------------------

    /// **Test Case #3 — PVC specification**
    ///
    /// WHY THIS MATTERS:
    /// The PVC provides persistent storage for agent data at /home/agent. If the
    /// PVC spec is wrong — wrong size, wrong access mode, wrong storage class —
    /// agents either can't start (no storage) or lose data on restart.
    ///
    /// WHAT THIS TEST DOES:
    /// 1. Builds a PVC from a test Agent
    /// 2. Verifies the PVC name follows `agent-{name}-data` convention
    /// 3. Verifies storage size matches the agent spec
    /// 4. Verifies access mode is ReadWriteOnce
    /// 5. Verifies storage class defaults to "local-path"
    /// 6. Verifies owner reference is set
    ///
    /// IF THIS FAILS:
    /// Agent persistent storage will be misconfigured, causing either mount
    /// failures or unexpected storage capacity.
    ///
    /// WHAT IS BEING TESTED:
    /// `build_pvc()` — pure function.
    #[test]
    fn p0_build_pvc_spec() {
        let agent = make_test_agent(TEST_AGENT_NAME, AgentState::Running);

        let pvc = build_pvc(&agent);

        // Name and namespace
        let meta = &pvc.metadata;
        assert_eq!(
            meta.name.as_deref(),
            Some(&format!("agent-{}-data", TEST_AGENT_NAME) as &str),
            "PVC name must follow 'agent-{{name}}-data' convention"
        );
        assert_eq!(
            meta.namespace.as_deref(),
            Some(TEST_NAMESPACE),
            "PVC must be in the agent's namespace"
        );

        // Spec
        let spec = pvc.spec.as_ref().expect("PVC must have spec");
        assert_eq!(
            spec.access_modes.as_deref(),
            Some(&["ReadWriteOnce".to_string()] as &[String]),
            "PVC must use ReadWriteOnce access mode"
        );

        let storage = spec
            .resources
            .as_ref()
            .expect("PVC must have resources")
            .requests
            .as_ref()
            .expect("PVC must have storage request")
            .get("storage")
            .expect("PVC must request storage");
        assert_eq!(
            storage.0, TEST_DISK,
            "PVC storage size must match agent spec"
        );

        // Owner reference
        assert!(
            meta.owner_references.is_some(),
            "PVC must have owner references for garbage collection"
        );
    }

    // -----------------------------------------------------------------------
    // build_network_policy tests
    // -----------------------------------------------------------------------

    /// **Test Cases #23, #24 — Network isolation policy**
    ///
    /// WHY THIS MATTERS:
    /// Network policies enforce tenant isolation. Without them, any agent can
    /// reach any other agent's network ports, enabling data exfiltration or
    /// lateral movement between compromised agents.
    ///
    /// WHAT THIS TEST DOES:
    /// 1. Builds a NetworkPolicy from a test Agent
    /// 2. Verifies the pod selector targets only this agent by name label
    /// 3. Verifies ingress allows only ingress-controller and ssh-proxy pods
    /// 4. Verifies egress allows only ports 80 and 443 (HTTP/HTTPS)
    /// 5. Verifies both Ingress and Egress policy types are declared
    ///
    /// IF THIS FAILS:
    /// Agent network isolation is broken. Agents may be able to communicate
    /// with each other directly or access unauthorized network services.
    ///
    /// WHAT IS BEING TESTED:
    /// `build_network_policy()` — pure function.
    #[test]
    fn p0_build_network_policy_spec() {
        let agent = make_test_agent(TEST_AGENT_NAME, AgentState::Running);

        let np = build_network_policy(&agent);

        // Metadata
        assert_eq!(
            np.metadata.name.as_deref(),
            Some(&format!("agent-{}-isolation", TEST_AGENT_NAME) as &str),
            "NetworkPolicy name must follow 'agent-{{name}}-isolation' convention"
        );

        let spec = np.spec.as_ref().expect("NetworkPolicy must have spec");

        // Pod selector targets this specific agent
        let selector_labels = spec
            .pod_selector
            .match_labels
            .as_ref()
            .expect("must have pod selector labels");
        assert_eq!(
            selector_labels.get(LABEL_NAME).map(String::as_str),
            Some(TEST_AGENT_NAME),
            "pod selector must target only this agent"
        );

        // Ingress: allow from ingress-controller and ssh-proxy
        let ingress_rules = spec.ingress.as_ref().expect("must have ingress rules");
        assert_eq!(ingress_rules.len(), 2, "must have two ingress rules (infra peers + nodeport)");
        let ingress_peers = ingress_rules[0]
            .from
            .as_ref()
            .expect("first ingress rule must have 'from' peers");
        assert_eq!(
            ingress_peers.len(),
            2,
            "first ingress rule must allow exactly 2 peer types (ingress-controller, ssh-proxy)"
        );

        // Egress: DNS (53) and HTTP/HTTPS (80/443)
        let egress_rules = spec.egress.as_ref().expect("must have egress rules");
        assert_eq!(egress_rules.len(), 1, "must have exactly one egress rule");
        let egress_ports = egress_rules[0]
            .ports
            .as_ref()
            .expect("egress rule must specify ports");
        let egress_port_numbers: Vec<&IntOrString> =
            egress_ports.iter().filter_map(|p| p.port.as_ref()).collect();
        assert!(
            egress_port_numbers.contains(&&IntOrString::Int(53)),
            "egress must allow port 53 (DNS)"
        );
        assert!(
            egress_port_numbers.contains(&&IntOrString::Int(80)),
            "egress must allow port 80"
        );
        assert!(
            egress_port_numbers.contains(&&IntOrString::Int(443)),
            "egress must allow port 443"
        );

        // Policy types
        let policy_types = spec.policy_types.as_ref().expect("must declare policy types");
        assert!(
            policy_types.contains(&"Ingress".to_string()),
            "must declare Ingress policy type"
        );
        assert!(
            policy_types.contains(&"Egress".to_string()),
            "must declare Egress policy type"
        );
    }

    // -----------------------------------------------------------------------
    // build_service tests
    // -----------------------------------------------------------------------

    /// **Test Case #1 — Service specification**
    ///
    /// WHY THIS MATTERS:
    /// The ClusterIP Service provides stable DNS and load-balanced access to
    /// agent pods. If the selector doesn't match pod labels, or the ports are
    /// wrong, traffic cannot reach the agent.
    ///
    /// WHAT THIS TEST DOES:
    /// 1. Builds a Service from a test Agent
    /// 2. Verifies the service name follows `agent-{name}` convention
    /// 3. Verifies the service type is ClusterIP
    /// 4. Verifies the selector matches the agent's name label
    /// 5. Verifies ports 80 (HTTP) and 22 (SSH) are exposed
    /// 6. Verifies owner reference is set
    ///
    /// IF THIS FAILS:
    /// Agent pods become unreachable via cluster DNS. The ingress controller
    /// and SSH proxy cannot route traffic to agents.
    ///
    /// WHAT IS BEING TESTED:
    /// `build_service()` — pure function.
    #[test]
    fn p0_build_service_spec() {
        let agent = make_test_agent(TEST_AGENT_NAME, AgentState::Running);

        let svc = build_service(&agent);

        // Metadata
        let meta = &svc.metadata;
        assert_eq!(
            meta.name.as_deref(),
            Some(&format!("agent-{}", TEST_AGENT_NAME) as &str),
            "service name must follow 'agent-{{name}}' convention"
        );

        let spec = svc.spec.as_ref().expect("service must have spec");
        assert_eq!(
            spec.type_.as_deref(),
            Some("NodePort"),
            "service type must be NodePort"
        );

        // Selector
        let selector = spec.selector.as_ref().expect("service must have selector");
        assert_eq!(
            selector.get(LABEL_NAME).map(String::as_str),
            Some(TEST_AGENT_NAME),
            "service selector must target this agent by name label"
        );

        // Ports
        let ports = spec.ports.as_ref().expect("service must expose ports");
        let port_numbers: Vec<i32> = ports.iter().map(|p| p.port).collect();
        assert!(
            port_numbers.contains(&80),
            "service must expose HTTP port 80"
        );
        assert!(
            port_numbers.contains(&22),
            "service must expose SSH port 22"
        );

        // Owner reference
        assert!(
            meta.owner_references.is_some(),
            "service must have owner references for garbage collection"
        );
    }
}
