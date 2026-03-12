use crate::helpers::*;

/// **Test Case #9 — Stop a running agent**
///
/// @testops 11.7 POST /instances/{name}/stop — stops running instance
///
/// WHY THIS MATTERS:
/// Stopping an agent should delete its pod (freeing compute) while preserving
/// the PVC (retaining data). This is the primary mechanism for users to
/// reduce costs when an agent is not needed.
///
/// WHAT THIS TEST DOES:
/// 1. Creates an agent and waits for Running
/// 2. Sends POST /instances/:name/stop
/// 3. Waits for Stopped phase
/// 4. Verifies the agent shows as stopped via GET
///
/// IF THIS FAILS:
/// Agents can't be stopped, or stopping fails to delete the pod, wasting
/// cluster compute resources.
#[tokio::test]
async fn test_p0_stop_agent() {
    let (client, name, _guard) = setup_running_agent("stop").await;

    // Stop
    let stop_url = format!("{}/instances/{}/stop", api_url(), name);
    let resp = client.post(&stop_url).send().await.expect("stop request failed");
    assert!(resp.status().is_success(), "stop must return 2xx");

    let instance = wait_for_phase(&client, &name, "Stopped", TIMEOUT_STOPPED).await;
    assert_eq!(instance.state, "stopped", "agent state must be 'stopped'");
}

/// **Test Case #10 — Start a stopped agent with data intact**
///
/// @testops 11.8 POST /instances/{name}/start — starts stopped instance
///
/// WHY THIS MATTERS:
/// Starting a stopped agent must recreate its pod and mount the existing PVC.
/// User data written before stopping must still be available after starting.
///
/// WHAT THIS TEST DOES:
/// 1. Creates an agent with a command that writes a marker on first run
///    and outputs "PERSISTED" on subsequent runs
/// 2. Stops the agent, waits for Stopped
/// 3. Starts the agent, waits for Running
/// 4. Checks logs for "PERSISTED" marker
///
/// IF THIS FAILS:
/// Data is lost when agents are stopped and started, or the start operation
/// doesn't properly remount the PVC.
#[tokio::test]
async fn test_p0_start_stopped_agent_data_intact() {
    let client = http_client();
    let name = unique_name("start");
    cleanup_agent(&client, &name).await;
    let _guard = AgentGuard::new(&client, &name);

    // Command: writes marker on first run, outputs PERSISTED on subsequent runs
    let cmd = "if [ -f /home/agent/stop-start-test ]; then echo 'PERSISTED'; else echo -n 'stop-start-persist-99999' > /home/agent/stop-start-test && echo 'FIRST_RUN'; fi && sleep infinity";
    create_agent_with_command(&client, &name, vec!["sh", "-c", cmd]).await;
    wait_for_phase(&client, &name, "Running", TIMEOUT_RUNNING).await;

    // Verify first run
    wait_for_log_containing(&client, &name, "FIRST_RUN", std::time::Duration::from_secs(30)).await;

    // Stop
    let stop_url = format!("{}/instances/{}/stop", api_url(), name);
    client.post(&stop_url).send().await.expect("stop failed");
    wait_for_phase(&client, &name, "Stopped", TIMEOUT_STOPPED).await;

    // Start
    let start_url = format!("{}/instances/{}/start", api_url(), name);
    client.post(&start_url).send().await.expect("start failed");
    wait_for_phase(&client, &name, "Running", TIMEOUT_RUNNING).await;

    // Check logs for PERSISTED
    let logs = wait_for_log_containing(
        &client,
        &name,
        "PERSISTED",
        std::time::Duration::from_secs(60),
    )
    .await;

    assert!(
        logs.contains("PERSISTED"),
        "data must survive stop/start cycle; got: {}",
        logs
    );
}

/// **Test Case #11 — Restart preserves data**
///
/// @testops 11.9 POST /instances/{name}/restart — restarts running instance
///
/// WHY THIS MATTERS:
/// Restart (delete pod, operator recreates) is used for applying config changes.
/// User data on the PVC must survive the restart.
///
/// WHAT THIS TEST DOES:
/// 1. Creates agent with a command that writes marker on first run,
///    outputs PERSISTED on subsequent runs
/// 2. Restarts via POST /instances/:name/restart
/// 3. Waits for Running, checks logs for PERSISTED
///
/// IF THIS FAILS:
/// Restart destroys PVC data, or the operator doesn't properly recreate the
/// pod with the same PVC mount.
#[tokio::test]
async fn test_p1_restart_data_intact() {
    let client = http_client();
    let name = unique_name("restart");
    cleanup_agent(&client, &name).await;
    let _guard = AgentGuard::new(&client, &name);

    let cmd = "if [ -f /home/agent/restart-test ]; then echo 'PERSISTED'; else echo -n 'restart-data-intact-55555' > /home/agent/restart-test && echo 'FIRST_RUN'; fi && sleep infinity";
    create_agent_with_command(&client, &name, vec!["sh", "-c", cmd]).await;
    wait_for_phase(&client, &name, "Running", TIMEOUT_RUNNING).await;

    // Verify first run
    wait_for_log_containing(&client, &name, "FIRST_RUN", std::time::Duration::from_secs(30)).await;

    // Restart
    let restart_url = format!("{}/instances/{}/restart", api_url(), name);
    client.post(&restart_url).send().await.expect("restart failed");

    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    wait_for_phase(&client, &name, "Running", TIMEOUT_RUNNING).await;

    // Check logs for PERSISTED
    let logs = wait_for_log_containing(
        &client,
        &name,
        "PERSISTED",
        std::time::Duration::from_secs(60),
    )
    .await;

    assert!(
        logs.contains("PERSISTED"),
        "data must survive restart; got: {}",
        logs
    );
}

/// **Test Case #12 — Stop an already-stopped agent is idempotent**
///
/// @testops 11.33 POST stop on already-stopped instance — verify defined behavior
///
/// WHY THIS MATTERS:
/// Automation scripts and UI may send redundant stop commands. The API must
/// handle this gracefully without errors.
///
/// WHAT THIS TEST DOES:
/// 1. Creates an agent, waits for Running, stops it
/// 2. Stops it again
/// 3. Asserts the second stop returns 200 without error
///
/// IF THIS FAILS:
/// Double-stop causes errors, making automation scripts fragile.
#[tokio::test]
async fn test_p2_stop_already_stopped() {
    let (client, name, _guard) = setup_running_agent("dblstop").await;

    let stop_url = format!("{}/instances/{}/stop", api_url(), name);

    // First stop
    client.post(&stop_url).send().await.expect("first stop failed");
    wait_for_phase(&client, &name, "Stopped", TIMEOUT_STOPPED).await;

    // Second stop — must not error
    let resp = client.post(&stop_url).send().await.expect("second stop failed");
    assert!(
        resp.status().is_success(),
        "stopping an already-stopped agent must return 2xx; got {}",
        resp.status()
    );
}

/// **Test Case #13 — Start an already-running agent is idempotent**
///
/// @testops 11.32 POST start on already-running instance — verify defined behavior
///
/// WHY THIS MATTERS:
/// Similar to double-stop, double-start must be harmless. The agent should
/// remain running without disruption.
///
/// WHAT THIS TEST DOES:
/// 1. Creates an agent and waits for Running
/// 2. Sends start again
/// 3. Asserts 200 and agent is still running
///
/// IF THIS FAILS:
/// Starting a running agent causes errors or restarts the agent unnecessarily.
#[tokio::test]
async fn test_p2_start_already_running() {
    let (client, name, _guard) = setup_running_agent("dblstart").await;

    let start_url = format!("{}/instances/{}/start", api_url(), name);
    let resp = client.post(&start_url).send().await.expect("start failed");

    assert!(
        resp.status().is_success(),
        "starting an already-running agent must return 2xx; got {}",
        resp.status()
    );

    let instance: InstanceResponse = resp.json().await.expect("must parse response");
    assert_eq!(
        instance.state, "running",
        "agent must still be in running state"
    );
}
