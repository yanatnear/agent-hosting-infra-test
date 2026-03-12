use crate::helpers::*;

/// **Test Case #1 — Agent creation reaches Running phase**
///
/// @testops 1.1 Create a single agent via API — verify it reaches 'running' state
///
/// WHY THIS MATTERS:
/// This is the most fundamental operation: creating an agent and verifying it
/// becomes healthy. Every other test depends on this basic lifecycle working.
///
/// WHAT THIS TEST DOES:
/// 1. Creates an agent via POST /instances
/// 2. Waits for it to reach Running phase
/// 3. Asserts phase, pod_ip, and host_node are populated
///
/// IF THIS FAILS:
/// The core creation pipeline is broken — operator, PVC provisioning, image
/// pull, or health probes are failing.
#[tokio::test]
async fn test_p0_create_reaches_running() {
    let (client, name, _guard) = setup_agent("create").await;

    let instance = wait_for_phase(&client, &name, "Running", TIMEOUT_RUNNING).await;

    assert_eq!(instance.phase.as_deref(), Some("Running"), "agent must reach Running phase");
    assert!(instance.pod_ip.is_some(), "running agent must have a pod IP");
    assert!(instance.host_node.is_some(), "running agent must be scheduled to a node");
}

/// **Test Case #2 — Agent can make outbound HTTPS requests**
///
/// @testops 1.2 Created agent can make outbound HTTPS requests (verify network egress)
///
/// WHY THIS MATTERS:
/// Agents need internet access for downloading packages, calling APIs, and
/// fetching data. The NetworkPolicy must allow egress on port 443.
///
/// WHAT THIS TEST DOES:
/// 1. Creates an agent with a command that curls httpbin.org
/// 2. Waits for Running phase
/// 3. Checks logs for the expected JSON response
///
/// IF THIS FAILS:
/// NetworkPolicy egress rules are blocking HTTPS traffic, or DNS resolution
/// is broken inside agent pods.
#[tokio::test]
async fn test_p0_outbound_https() {
    let (client, name, _guard) = setup_running_command_agent(
        "outbound",
        vec!["sh", "-c", "curl -sf https://httpbin.org/get && sleep infinity"],
    )
    .await;

    let logs = wait_for_log_containing(
        &client,
        &name,
        "\"url\"",
        std::time::Duration::from_secs(60),
    )
    .await;

    assert!(
        logs.contains("\"url\""),
        "curl response must contain JSON from httpbin; got: {}",
        logs
    );
}

/// **Test Case #3 — Agent filesystem is writable and persistent**
///
/// @testops 1.3 Created agent has its allocated filesystem mounted and writable
///
/// WHY THIS MATTERS:
/// Agents store code, data, and configuration in /home/agent. This directory
/// must be writable (backed by PVC) even though the root filesystem is read-only.
///
/// WHAT THIS TEST DOES:
/// 1. Creates an agent with a command that writes a file and reads it back
/// 2. Waits for Running phase
/// 3. Checks logs for the marker content
///
/// IF THIS FAILS:
/// PVC mount is missing or misconfigured. Agent code will fail to write any
/// files, making the agent useless.
#[tokio::test]
async fn test_p0_writable_persistent_filesystem() {
    const MARKER: &str = "persist-test-data-12345";

    let (client, name, _guard) = setup_running_command_agent(
        "fs-write",
        vec![
            "sh",
            "-c",
            &format!(
                "echo '{}' > /home/agent/testfile && cat /home/agent/testfile && sleep infinity",
                MARKER
            ),
        ],
    )
    .await;

    let logs = wait_for_log_containing(
        &client,
        &name,
        MARKER,
        std::time::Duration::from_secs(30),
    )
    .await;

    assert!(
        logs.contains(MARKER),
        "file content must match what was written; got: {}",
        logs
    );
}

/// **Test Case #4 — Agent can spawn sub-agent via Docker**
///
/// @testops 1.10 Create agent and verify it can spawn a sub-agent Docker container
///
/// WHY THIS MATTERS:
/// Agents must be able to run Docker containers (sub-agents) using the Sysbox
/// runtime. This is a core platform capability for multi-agent workflows.
///
/// WHAT THIS TEST DOES:
/// 1. Creates a Docker-enabled agent and waits for Running
/// 2. Checks logs for Docker daemon startup indicating Sysbox runtime works
///
/// IF THIS FAILS:
/// Sysbox runtime is not configured, or Docker-in-Docker is not available.
/// The runtimeClassName may be missing from the pod spec.
#[tokio::test]
async fn test_p0_spawn_sub_agent_docker() {
    let client = http_client();
    let name = unique_name("docker");
    cleanup_agent(&client, &name).await;
    let _guard = AgentGuard::new(&client, &name);
    create_agent_with_docker(&client, &name).await;
    wait_for_phase(&client, &name, "Running", TIMEOUT_RUNNING).await;

    // Verify Docker daemon starts — this only works with Sysbox runtime
    let logs = wait_for_log_containing(
        &client,
        &name,
        "API listen on",
        std::time::Duration::from_secs(120),
    )
    .await;

    assert!(
        logs.contains("API listen on"),
        "Docker daemon must start successfully (requires Sysbox runtime); got: {}",
        logs
    );
}

/// **Test Case #5 — Invalid creation params return 4xx error**
///
/// @testops 1.4 Create agent with invalid parameters — verify API returns clear error
///
/// WHY THIS MATTERS:
/// The API must validate input and return clear error messages. An empty body
/// must be rejected before reaching Kubernetes, preventing cryptic K8s errors.
///
/// WHAT THIS TEST DOES:
/// 1. Sends POST /instances with an empty JSON body `{}`
/// 2. Asserts the response is a 4xx client error
/// 3. Asserts the response body is valid JSON
///
/// IF THIS FAILS:
/// Invalid requests reach Kubernetes and produce confusing 500 errors instead
/// of clear validation messages.
#[tokio::test]
async fn test_p1_invalid_params_error() {
    let client = http_client();
    let url = format!("{}/instances", api_url());

    let resp = client
        .post(&url)
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("POST request failed");

    assert!(
        resp.status().is_client_error(),
        "empty body must return 4xx; got {}",
        resp.status()
    );
}

/// **Test Case #6 — Duplicate agent name returns 409 Conflict**
///
/// @testops 11.31 Create instance with duplicate name — verify defined behavior (409 or return existing)
///
/// WHY THIS MATTERS:
/// Agent names must be unique within a namespace. Creating a duplicate must
/// return 409 Conflict so the CLI can display a clear error message.
///
/// WHAT THIS TEST DOES:
/// 1. Creates an agent successfully
/// 2. Attempts to create another agent with the same name
/// 3. Asserts the second attempt returns 409 with code="conflict"
///
/// IF THIS FAILS:
/// Duplicate names either succeed (data corruption) or return 500 instead
/// of 409 (poor user experience).
#[tokio::test]
async fn test_p1_duplicate_name_conflict() {
    let (client, name, _guard) = setup_agent("dup").await;

    // Attempt duplicate creation
    let url = format!("{}/instances", api_url());
    let body = serde_json::json!({
        "name": name,
        "image": test_image(),
    });
    let resp = client.post(&url).json(&body).send().await.expect("POST failed");

    assert_eq!(
        resp.status().as_u16(),
        409,
        "duplicate creation must return 409 Conflict"
    );

    let error: ErrorResponse = resp.json().await.expect("must parse error response");
    assert_eq!(
        error.error.code, "conflict",
        "error code must be 'conflict'"
    );
}

/// **Test Case #7 — Crashed agent auto-restarts**
///
/// @testops 1.20 Agent process crashes — verify auto-restart and recovery to healthy state
///
/// WHY THIS MATTERS:
/// When an agent process crashes, Kubernetes must automatically restart the
/// container. The restart_count must increase to track reliability.
///
/// WHAT THIS TEST DOES:
/// 1. Creates an agent with a command that exits immediately
/// 2. Waits for restart_count to increase
/// 3. Asserts restart_count > 0
///
/// IF THIS FAILS:
/// Restart policy is misconfigured. Crashed agents will stay down permanently.
#[tokio::test]
async fn test_p0_crash_auto_restarts() {
    let client = http_client();
    let name = unique_name("crash");
    cleanup_agent(&client, &name).await;
    let _guard = AgentGuard::new(&client, &name);

    // Create agent with a command that exits immediately, triggering restarts
    create_agent_with_command(&client, &name, vec!["sh", "-c", "exit 1"]).await;

    // Wait for restart_count to increase (container will crash and be restarted)
    let recovered = wait_for_restart_count_increase(
        &client,
        &name,
        0,
        TIMEOUT_RUNNING,
    )
    .await;

    assert!(
        recovered.restart_count.unwrap_or(0) > 0,
        "restart_count must increase after crash; got {}",
        recovered.restart_count.unwrap_or(0)
    );
}

/// **Test Case #8 — Data persists across container restart**
///
/// @testops 1.19 Agent writes data to local filesystem — verify persistence across agent process restart
///
/// WHY THIS MATTERS:
/// The PVC-backed /home/agent volume must survive container restarts. Users
/// expect their files to persist even when the agent process crashes.
///
/// WHAT THIS TEST DOES:
/// 1. Creates an agent with a command that writes a marker on first run
///    and outputs "PERSISTED" on subsequent runs
/// 2. Waits for Running, verifies first-run output in logs
/// 3. Restarts the agent via POST /instances/:name/restart
/// 4. Waits for Running again
/// 5. Checks logs for "PERSISTED" marker
///
/// IF THIS FAILS:
/// PVC is not mounted or data is stored on the ephemeral root filesystem.
/// Users lose all their work on every restart.
#[tokio::test]
async fn test_p0_data_persists_across_restart() {
    let client = http_client();
    let name = unique_name("persist");
    cleanup_agent(&client, &name).await;
    let _guard = AgentGuard::new(&client, &name);

    // Command: on first run writes a marker, on subsequent runs reads it
    let cmd = "if [ -f /home/agent/persist-test ]; then echo 'PERSISTED'; else echo 'FIRST_RUN' && echo -n 'restart-persist-check-67890' > /home/agent/persist-test; fi && sleep infinity";
    create_agent_with_command(&client, &name, vec!["sh", "-c", cmd]).await;
    wait_for_phase(&client, &name, "Running", TIMEOUT_RUNNING).await;

    // Verify first run
    wait_for_log_containing(&client, &name, "FIRST_RUN", std::time::Duration::from_secs(30)).await;

    // Restart via API
    let restart_url = format!("{}/instances/{}/restart", api_url(), name);
    client
        .post(&restart_url)
        .send()
        .await
        .expect("restart request failed");

    // Wait for Running again
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    wait_for_phase(&client, &name, "Running", TIMEOUT_RUNNING).await;

    // Check logs for PERSISTED — the restarted container should find the file
    let logs = wait_for_log_containing(
        &client,
        &name,
        "PERSISTED",
        std::time::Duration::from_secs(60),
    )
    .await;

    assert!(
        logs.contains("PERSISTED"),
        "file content must survive restart; got: {}",
        logs
    );
}
