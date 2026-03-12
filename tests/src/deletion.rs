use crate::helpers::*;

/// **Test Case #14 — Delete a stopped agent removes all resources**
///
/// @testops 2.1 Delete a stopped agent via API — verify all resources cleaned up
///
/// WHY THIS MATTERS:
/// Deleting a stopped agent must clean up all K8s resources via owner
/// references. The API must return 404 after deletion completes.
///
/// WHAT THIS TEST DOES:
/// 1. Creates an agent, waits for Running, stops it
/// 2. Deletes it via DELETE /instances/:name
/// 3. Waits for 404 on GET /instances/:name (confirms full cleanup)
///
/// IF THIS FAILS:
/// The CRD deletion doesn't cascade properly, or the API still returns
/// stale data after deletion.
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

    // Wait for full deletion — API returns 404
    wait_for_deletion(&client, &name, TIMEOUT_DELETED).await;
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
