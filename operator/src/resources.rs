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
        ports: Some(vec![
            ContainerPort {
                container_port: 22,
                name: Some("ssh".to_string()),
                protocol: Some("TCP".to_string()),
                ..Default::default()
            },
            ContainerPort {
                container_port: 80,
                name: Some("http".to_string()),
                protocol: Some("TCP".to_string()),
                ..Default::default()
            },
        ]),
        resources: Some(ResourceRequirements {
            requests: Some(resource_requests),
            limits: Some(resource_limits),
            ..Default::default()
        }),
        security_context: Some(SecurityContext {
            run_as_non_root: Some(true),
            read_only_root_filesystem: Some(true),
            allow_privilege_escalation: Some(false),
            capabilities: Some(k8s_openapi::api::core::v1::Capabilities {
                drop: Some(vec!["ALL".to_string()]),
                add: Some(vec!["NET_BIND_SERVICE".to_string()]),
            }),
            ..Default::default()
        }),
        volume_mounts: Some(vec![
            VolumeMount {
                name: "agent-data".to_string(),
                mount_path: "/home/agent".to_string(),
                ..Default::default()
            },
            VolumeMount {
                name: "tmp".to_string(),
                mount_path: "/tmp".to_string(),
                ..Default::default()
            },
        ]),
        liveness_probe: Some(Probe {
            tcp_socket: Some(TCPSocketAction {
                port: IntOrString::Int(22),
                ..Default::default()
            }),
            period_seconds: Some(10),
            failure_threshold: Some(3),
            ..Default::default()
        }),
        readiness_probe: Some(Probe {
            tcp_socket: Some(TCPSocketAction {
                port: IntOrString::Int(22),
                ..Default::default()
            }),
            period_seconds: Some(5),
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

    let ingress_rule = NetworkPolicyIngressRule {
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

    // Egress: allow all on ports 80 and 443
    let egress_rule = NetworkPolicyEgressRule {
        to: None,
        ports: Some(vec![
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
            ingress: Some(vec![ingress_rule]),
            egress: Some(vec![egress_rule]),
            policy_types: Some(vec!["Ingress".to_string(), "Egress".to_string()]),
        }),
    }
}

/// Build the Service for an agent.
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
            type_: Some("ClusterIP".to_string()),
            selector: Some(selector),
            ports: Some(vec![
                ServicePort {
                    name: Some("http".to_string()),
                    port: 80,
                    target_port: Some(IntOrString::Int(80)),
                    protocol: Some("TCP".to_string()),
                    ..Default::default()
                },
                ServicePort {
                    name: Some("ssh".to_string()),
                    port: 22,
                    target_port: Some(IntOrString::Int(22)),
                    protocol: Some("TCP".to_string()),
                    ..Default::default()
                },
            ]),
            ..Default::default()
        }),
        ..Default::default()
    }
}
