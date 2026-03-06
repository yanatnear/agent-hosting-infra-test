use std::sync::Arc;

use futures::StreamExt;
use k8s_openapi::api::core::v1::{PersistentVolumeClaim, Pod, Service};
use k8s_openapi::api::networking::v1::NetworkPolicy;
use kube::api::{Api, Patch, PatchParams};
use kube::runtime::Controller;
use kube::Client;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

mod controller;
mod crd;
mod error;
mod resources;

use crd::Agent;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    info!("Starting NEAR AI Agent Operator");

    let namespace =
        std::env::var("AGENT_NAMESPACE").unwrap_or_else(|_| "agents".to_string());
    info!("Watching namespace: {}", namespace);

    // Create Kubernetes client
    let client = Client::try_default().await?;

    // Register / apply the Agent CRD to the cluster
    register_crd(&client).await?;

    // Build shared context
    let ctx = Arc::new(controller::Context {
        client: client.clone(),
        namespace: namespace.clone(),
    });

    // Set up the controller
    let agents: Api<Agent> = Api::namespaced(client.clone(), &namespace);
    let pods: Api<Pod> = Api::namespaced(client.clone(), &namespace);
    let pvcs: Api<PersistentVolumeClaim> = Api::namespaced(client.clone(), &namespace);
    let services: Api<Service> = Api::namespaced(client.clone(), &namespace);
    let netpols: Api<NetworkPolicy> = Api::namespaced(client.clone(), &namespace);

    info!("Starting controller");

    Controller::new(agents, kube::runtime::watcher::Config::default())
        .owns(pods, kube::runtime::watcher::Config::default())
        .owns(pvcs, kube::runtime::watcher::Config::default())
        .owns(services, kube::runtime::watcher::Config::default())
        .owns(netpols, kube::runtime::watcher::Config::default())
        .run(controller::reconcile, controller::error_policy, ctx)
        .for_each(|result| async move {
            match result {
                Ok((_obj, _action)) => {}
                Err(e) => {
                    error!("Controller error: {:?}", e);
                }
            }
        })
        .await;

    Ok(())
}

/// Apply the Agent CRD definition to the cluster (server-side apply).
async fn register_crd(client: &Client) -> anyhow::Result<()> {
    use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;

    let crds: Api<CustomResourceDefinition> = Api::all(client.clone());
    let crd = crd::generate_crd();
    let name = crd
        .metadata
        .name
        .as_deref()
        .expect("CRD must have a name");

    info!("Applying Agent CRD: {}", name);

    let params = PatchParams::apply("nearai-agent-operator").force();
    crds.patch(name, &params, &Patch::Apply(&crd)).await?;

    info!("Agent CRD applied successfully");
    Ok(())
}
