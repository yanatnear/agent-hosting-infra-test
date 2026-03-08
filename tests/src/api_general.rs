use crate::helpers::*;

/// **Test Case #20 — List instances includes created agent**
///
/// @testops 11.4 GET /instances — returns list containing the created instance
///
/// WHY THIS MATTERS:
/// The list endpoint is the primary discovery mechanism. It must return all
/// agents with their current status, supporting the CLI's `list` command
/// and web UI's dashboard view.
///
/// WHAT THIS TEST DOES:
/// 1. Creates an agent and waits for Running
/// 2. Calls GET /instances
/// 3. Finds the agent in the list
/// 4. Asserts the listed agent has correct fields
///
/// IF THIS FAILS:
/// Agents are invisible in the list view. Users can't discover or manage
/// their agents through the standard interface.
#[tokio::test]
async fn test_p0_list_instances() {
    let (client, name, _guard) = setup_running_agent("list").await;

    let url = format!("{}/instances", api_url());
    let resp = client.get(&url).send().await.expect("list request failed");
    assert!(resp.status().is_success(), "list must return 2xx");

    let instances: Vec<InstanceResponse> = resp.json().await.expect("must parse list response");
    let found = instances.iter().find(|i| i.name == name);

    assert!(found.is_some(), "created agent must appear in list");
    let agent = found.unwrap();
    assert_eq!(agent.state, "running", "listed agent must show running state");
}

/// **Test Case #21 — Get instance details returns all fields**
///
/// @testops 11.2 GET /instances/{name} — returns current status and details
///
/// WHY THIS MATTERS:
/// The detail endpoint provides complete agent information for the CLI's
/// `status` command and web UI's detail view. Every field must be present
/// and correctly populated.
///
/// WHAT THIS TEST DOES:
/// 1. Creates an agent and waits for Running
/// 2. Calls GET /instances/:name
/// 3. Asserts all expected fields are populated
///
/// IF THIS FAILS:
/// Agent detail view is incomplete. Users can't see CPU, memory, disk,
/// phase, or node information.
#[tokio::test]
async fn test_p0_get_instance_details() {
    let (client, name, _guard) = setup_running_agent("detail").await;

    let instance = wait_for_phase(&client, &name, "Running", TIMEOUT_RUNNING).await;

    assert_eq!(instance.name, name, "name must match");
    assert!(!instance.image.is_empty(), "image must be populated");
    assert_eq!(instance.cpu, TEST_CPU, "cpu must match creation request");
    assert_eq!(instance.memory, TEST_MEMORY, "memory must match creation request");
    assert_eq!(instance.disk, TEST_DISK, "disk must match creation request");
    assert_eq!(instance.state, "running", "state must be 'running'");
    assert_eq!(
        instance.phase.as_deref(),
        Some("Running"),
        "phase must be 'Running'"
    );
    assert!(instance.pod_ip.is_some(), "pod_ip must be populated for running agent");
    assert!(instance.host_node.is_some(), "host_node must be populated for running agent");
}

/// **Test Case #22 — Error responses have consistent JSON structure**
///
/// @testops 11.29 All error responses follow consistent format (error code, message, request ID)
///
/// WHY THIS MATTERS:
/// All API errors must use the same JSON envelope `{error: {code, message,
/// request_id}}` so clients can implement a single error-handling path.
///
/// WHAT THIS TEST DOES:
/// 1. Triggers a 404 error (GET nonexistent agent)
/// 2. Triggers a 409 error (create duplicate)
/// 3. Asserts both responses have identical JSON structure
///
/// IF THIS FAILS:
/// Different error types produce different JSON shapes, breaking client-side
/// error parsing.
#[tokio::test]
async fn test_p2_error_format_consistent() {
    let client = http_client();

    // Trigger 404
    let ghost = unique_name("ghost");
    let url_404 = format!("{}/instances/{}", api_url(), ghost);
    let resp_404 = client.get(&url_404).send().await.expect("GET failed");
    assert_eq!(resp_404.status().as_u16(), 404);
    let err_404: ErrorResponse = resp_404.json().await.expect("must parse 404 error");

    // Trigger 409
    let (client, name, _guard) = setup_agent("errfmt").await;

    let create_url = format!("{}/instances", api_url());
    let body = serde_json::json!({
        "name": name,
        "image": test_image(),
    });
    let resp_409 = client.post(&create_url).json(&body).send().await.expect("POST failed");
    assert_eq!(resp_409.status().as_u16(), 409);
    let err_409: ErrorResponse = resp_409.json().await.expect("must parse 409 error");

    // Both must have the same structure
    assert!(
        !err_404.error.code.is_empty(),
        "404 error must have non-empty code"
    );
    assert!(
        !err_404.error.message.is_empty(),
        "404 error must have non-empty message"
    );
    assert!(
        !err_404.error.request_id.is_empty(),
        "404 error must have non-empty request_id"
    );

    assert!(
        !err_409.error.code.is_empty(),
        "409 error must have non-empty code"
    );
    assert!(
        !err_409.error.message.is_empty(),
        "409 error must have non-empty message"
    );
    assert!(
        !err_409.error.request_id.is_empty(),
        "409 error must have non-empty request_id"
    );
}
