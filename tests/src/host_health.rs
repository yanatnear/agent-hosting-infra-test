/// **Test Case #27 — Node exporter is running**
///
/// @testops 8.1 Prometheus scrapes host-level metrics (CPU, memory, disk, network) successfully
///
/// WHY THIS MATTERS:
/// Node exporter provides host-level metrics (CPU, memory, disk, network)
/// to Prometheus. Without it, operators have no visibility into host health.
///
/// NOTE: This test is ignored because it requires a configured Prometheus
/// endpoint. Set PROMETHEUS_URL to enable.
#[tokio::test]
#[ignore = "requires Prometheus URL — set PROMETHEUS_URL env var"]
async fn test_p1_node_exporter_running() {
    let prom_url = std::env::var("PROMETHEUS_URL")
        .expect("PROMETHEUS_URL must be set for this test");

    let client = reqwest::Client::new();
    let query_url = format!("{}/api/v1/query?query=up{{job=\"node-exporter\"}}", prom_url);
    let resp = client.get(&query_url).send().await.expect("Prometheus query failed");

    assert!(resp.status().is_success(), "Prometheus query must succeed");

    let body: serde_json::Value = resp.json().await.expect("must parse Prometheus response");
    let results = body["data"]["result"]
        .as_array()
        .expect("must have result array");

    assert!(
        !results.is_empty(),
        "node-exporter must have at least one 'up' metric"
    );
}

/// **Test Case #28 — Log rotation is configured**
///
/// WHY THIS MATTERS:
/// Without log rotation, container logs fill the host disk, eventually
/// causing node-level failures that affect all agents on that host.
///
/// NOTE: This test is ignored because it requires host-level access to
/// check Docker daemon configuration.
#[tokio::test]
#[ignore = "requires host access to check Docker daemon.json"]
async fn test_p0_log_rotation_configured() {
    // This test would need to:
    // 1. Access the host's /etc/docker/daemon.json
    // 2. Verify log-driver and log-opts are set
    // 3. Check max-size and max-file are configured
    //
    // Since we can't access host files from inside a pod (by design),
    // this test requires a separate host-level test harness.
    todo!("requires host access — run via ansible or SSH to the node");
}
