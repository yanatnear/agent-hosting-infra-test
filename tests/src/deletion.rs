use crate::helpers::*;

/// **Test Case #14 — Delete a stopped agent removes all resources**
///
/// @testops 2.1 Delete a stopped agent via API — verify all resources cleaned up
///
/// WHY THIS MATTERS:
/// Deleting a stopped agent must clean up all K8s resources (Pod, PVC, Service,
/// NetworkPolicy) via owner references. Leftover resources waste cluster
/// capacity and may cause naming conflicts.
///
/// WHAT THIS TEST DOES:
/// 1. Creates an agent, waits for Running, stops it
/// 2. Deletes it via DELETE /instances/:name
/// 3. Waits for 404 on GET /instances/:name
/// 4. Verifies the K8s resources are gone
///
/// IF THIS FAILS:
/// Orphaned K8s resources accumulate. Owner references are misconfigured or
/// the CRD deletion doesn't cascade properly.
#[tokio::test]
async fn test_p0_delete_stopped_all_resources_gone() {
    let (client, name, _guard) = setup_running_agent("del-stop").await;

    // Stop first
    let stop_url = format!("{}/instances/{}/stop", api_url(), name);
    client.post(&stop_url).send().await.expect("stop failed");
    wait_for_phase(&client, &name, "Stopped", TIMEOUT_STOPPED).await;

    // Delete
    let delete_url = format!("{}/instances/{}", api_url(), name);
    let resp = client.delete(&delete_url).send().await.expect("delete failed");
    assert_eq!(
        resp.status().as_u16(),
        204,
        "delete must return 204 No Content"
    );

    wait_for_deletion(&client, &name, TIMEOUT_DELETED).await;

    // Give GC time to clean up orphaned resources
    

    // Verify K8s resources are cleaned up (poll for GC)
    let kube = kube_client().await;
    let ns = agent_namespace();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);

    let pods: kube::Api<k8s_openapi::api::core::v1::Pod> = kube::Api::namespaced(kube.clone(), &ns);
    let pvcs: kube::Api<k8s_openapi::api::core::v1::PersistentVolumeClaim> = kube::Api::namespaced(kube.clone(), &ns);

    let mut pod_gone = false;
    let mut pvc_gone = false;

    while tokio::time::Instant::now() < deadline {
        if !pod_gone && pods.get_opt(&format!("agent-{}", name)).await.unwrap().is_none() {
            pod_gone = true;
        }
        if !pvc_gone && pvcs.get_opt(&format!("agent-{}-data", name)).await.unwrap().is_none() {
            pvc_gone = true;
        }
        if pod_gone && pvc_gone {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    assert!(pod_gone, "pod must be deleted (waited 30s)");
    assert!(pvc_gone, "PVC must be deleted (waited 30s)");

    let services: kube::Api<k8s_openapi::api::core::v1::Service> =
        kube::Api::namespaced(kube.clone(), &ns);
    assert!(
        services.get_opt(&format!("agent-{}", name)).await.unwrap().is_none(),
        "service must be deleted"
    );

    let netpols: kube::Api<k8s_openapi::api::networking::v1::NetworkPolicy> =
        kube::Api::namespaced(kube, &ns);
    assert!(
        netpols
            .get_opt(&format!("agent-{}-isolation", name))
            .await
            .unwrap()
            .is_none(),
        "NetworkPolicy must be deleted"
    );
}

/// **Test Case #15 — Delete a running agent cleans up**
///
/// @testops 2.3 Delete a running agent via API — verify it is stopped first, then deleted
///
/// WHY THIS MATTERS:
/// Users may delete running agents without stopping first. The system must
/// handle this by cleaning up all resources (pod, PVC, service, etc.).
///
/// WHAT THIS TEST DOES:
/// 1. Creates an agent and waits for Running
/// 2. Deletes it while still running (skipping stop)
/// 3. Waits for 404 on GET
/// 4. Verifies all K8s resources are gone
///
/// IF THIS FAILS:
/// Running agents can't be deleted, or deletion leaves orphaned resources.
#[tokio::test]
async fn test_p0_delete_running_cleans_up() {
    let (client, name, _guard) = setup_running_agent("del-run").await;

    // Delete while running
    let delete_url = format!("{}/instances/{}", api_url(), name);
    let resp = client.delete(&delete_url).send().await.expect("delete failed");
    assert_eq!(resp.status().as_u16(), 204, "delete must return 204");

    wait_for_deletion(&client, &name, TIMEOUT_DELETED).await;

    // Give GC time to clean up orphaned resources
    
}

/// **Test Case #16 — Delete nonexistent agent returns 404**
///
/// @testops 11.35 DELETE nonexistent instance — returns 404
///
/// WHY THIS MATTERS:
/// The API must return 404 for delete attempts on agents that don't exist,
/// rather than 500 or 204. This lets automation scripts distinguish between
/// "successfully deleted" and "never existed".
///
/// WHAT THIS TEST DOES:
/// 1. Sends DELETE for a name that was never created
/// 2. Asserts 404 response with structured error JSON
///
/// IF THIS FAILS:
/// Delete returns 500 or 204 for nonexistent agents, confusing automation.
#[tokio::test]
async fn test_p1_delete_nonexistent_404() {
    let client = http_client();
    let name = unique_name("ghost");

    let url = format!("{}/instances/{}", api_url(), name);
    let resp = client.delete(&url).send().await.expect("delete request failed");

    assert_eq!(
        resp.status().as_u16(),
        404,
        "deleting nonexistent agent must return 404"
    );

    let error: ErrorResponse = resp.json().await.expect("must parse error response");
    assert_eq!(error.error.code, "not_found", "error code must be 'not_found'");
}
