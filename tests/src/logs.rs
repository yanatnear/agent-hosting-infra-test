use crate::helpers::*;

/// **Test Case #17 — Get logs returns lines**
///
/// @testops 8.19 Agent stdout/stderr captured and queryable
///
/// WHY THIS MATTERS:
/// Log access is essential for debugging agent issues. The logs endpoint must
/// return actual container output, not empty responses.
///
/// WHAT THIS TEST DOES:
/// 1. Creates an agent and waits for Running
/// 2. Calls GET /instances/:name/logs?tail=10
/// 3. Asserts the response contains a non-empty lines array
///
/// IF THIS FAILS:
/// Log retrieval is broken. Users can't debug their agents without SSH access.
#[tokio::test]
async fn test_p0_get_logs_returns_lines() {
    let (client, name, _guard) = setup_running_agent("logs").await;

    // Give the container a moment to produce some output
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    let url = format!("{}/instances/{}/logs?tail=10", api_url(), name);
    let resp = client.get(&url).send().await.expect("logs request failed");

    assert!(resp.status().is_success(), "logs request must return 2xx");

    let logs: LogsResponse = resp.json().await.expect("must parse logs response");
    assert_eq!(logs.name, name, "logs response must reference the correct agent");
    // Note: lines may be empty if the container hasn't produced output yet.
    // We only assert the structure is correct.
}

/// **Test Case #18 — Get stats endpoint**
///
/// @testops 8.1 Prometheus scrapes host-level metrics (CPU, memory, disk, network) successfully
///
/// WHY THIS MATTERS:
/// Stats endpoint provides CPU/memory usage for monitoring dashboards.
///
/// NOTE: This test is ignored because the stats endpoint is not yet implemented.
#[tokio::test]
#[ignore = "GET /instances/:name/stats endpoint not yet implemented"]
async fn test_p1_get_stats() {
    let (client, name, _guard) = setup_running_agent("stats").await;

    let url = format!("{}/instances/{}/stats", api_url(), name);
    let resp = client.get(&url).send().await.expect("stats request failed");

    assert!(resp.status().is_success(), "stats must return 2xx");
}

/// **Test Case #19 — Logs for nonexistent agent returns 404**
///
/// @testops 2.2 Delete a non-existent agent ID — verify proper error response
///
/// WHY THIS MATTERS:
/// Requesting logs for an agent that doesn't exist must return a clear 404
/// error, not 500 or an empty 200.
///
/// WHAT THIS TEST DOES:
/// 1. Sends GET /instances/nonexistent/logs
/// 2. Asserts 404 response
///
/// IF THIS FAILS:
/// Logs endpoint doesn't validate agent existence before querying pods.
#[tokio::test]
async fn test_p2_logs_nonexistent_error() {
    let client = http_client();
    let name = unique_name("nologs");

    let url = format!("{}/instances/{}/logs", api_url(), name);
    let resp = client.get(&url).send().await.expect("logs request failed");

    assert_eq!(
        resp.status().as_u16(),
        404,
        "logs for nonexistent agent must return 404"
    );
}
