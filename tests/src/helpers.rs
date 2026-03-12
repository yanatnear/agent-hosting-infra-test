use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Environment configuration
// ---------------------------------------------------------------------------

/// Base URL for the agent management API.
/// Override with AGENT_API_URL env var for non-default deployments.
pub fn api_url() -> String {
    std::env::var("AGENT_API_URL").unwrap_or_else(|_| "http://localhost:30080".to_string())
}

/// Container image used for test agents.
/// Override with TEST_AGENT_IMAGE for custom test images.
pub fn test_image() -> String {
    std::env::var("TEST_AGENT_IMAGE")
        .unwrap_or_else(|_| "nearaidev/ironclaw-nearai-worker:latest".to_string())
}

/// Namespace where agents are created.
/// Override with AGENT_NAMESPACE for non-default namespaces.
pub fn agent_namespace() -> String {
    std::env::var("AGENT_NAMESPACE").unwrap_or_else(|_| "agents".to_string())
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Resource specs for test agents — small to conserve cluster capacity.
pub const TEST_CPU: &str = "100m";
pub const TEST_MEMORY: &str = "128Mi";
pub const TEST_DISK: &str = "1Gi";

/// Timeout for an agent to reach Running phase from creation.
pub const TIMEOUT_RUNNING: Duration = Duration::from_secs(600);

/// Timeout for an agent to reach Stopped phase after a stop command.
pub const TIMEOUT_STOPPED: Duration = Duration::from_secs(60);

/// Timeout for an agent to be fully deleted (404 on GET).
pub const TIMEOUT_DELETED: Duration = Duration::from_secs(60);

/// Polling interval between status checks.
pub const POLL_INTERVAL: Duration = Duration::from_secs(3);

// ---------------------------------------------------------------------------
// HTTP client
// ---------------------------------------------------------------------------

/// Creates an HTTP client with a 30-second timeout for all requests.
pub fn http_client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("failed to create HTTP client")
}

// ---------------------------------------------------------------------------
// Name generation
// ---------------------------------------------------------------------------

/// Generates a unique agent name for test isolation.
/// Format: `{prefix}-{8-char-uuid}` to avoid collisions between parallel tests.
pub fn unique_name(prefix: &str) -> String {
    let short_id = &Uuid::new_v4().to_string()[..8];
    format!("{}-{}", prefix, short_id)
}

// ---------------------------------------------------------------------------
// Response types (deserialized from API JSON)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize)]
pub struct InstanceResponse {
    pub name: String,
    pub image: String,
    pub cpu: String,
    pub memory: String,
    pub disk: String,
    pub state: String,
    pub phase: Option<String>,
    pub pod_ip: Option<String>,
    pub host_node: Option<String>,
    pub restart_count: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct ErrorResponse {
    pub error: ErrorDetail,
}

#[derive(Debug, Deserialize)]
pub struct ErrorDetail {
    pub code: String,
    pub message: String,
    pub request_id: String,
}

#[derive(Debug, Deserialize)]
pub struct LogsResponse {
    pub name: String,
    pub lines: Vec<String>,
}

// ---------------------------------------------------------------------------
// Agent CRUD helpers
// ---------------------------------------------------------------------------

/// Creates an agent via the API and asserts the response is 201 Created.
/// Uses small resource specs to conserve cluster capacity.
pub async fn create_agent(client: &Client, name: &str) -> InstanceResponse {
    let url = format!("{}/instances", api_url());
    let body = serde_json::json!({
        "name": name,
        "image": test_image(),
        "cpu": TEST_CPU,
        "memory": TEST_MEMORY,
        "disk": TEST_DISK,
        "security_profile": "trusted",  // ironclaw needs chown capabilities
    });

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .expect("POST /instances request failed");

    assert_eq!(
        resp.status().as_u16(),
        201,
        "create_agent must return 201 Created for name '{}'",
        name
    );

    resp.json::<InstanceResponse>()
        .await
        .expect("failed to parse create response")
}

/// Creates an agent with a custom command override.
/// The container runs the given command instead of the image's default entrypoint.
/// Health probes are automatically skipped for command agents.
pub async fn create_agent_with_command(
    client: &Client,
    name: &str,
    command: Vec<&str>,
) -> InstanceResponse {
    let url = format!("{}/instances", api_url());
    let body = serde_json::json!({
        "name": name,
        "image": test_image(),
        "cpu": TEST_CPU,
        "memory": TEST_MEMORY,
        "disk": TEST_DISK,
        "security_profile": "trusted",
        "command": command,
    });

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .expect("POST /instances request failed");

    assert_eq!(
        resp.status().as_u16(),
        201,
        "create_agent_with_command must return 201 Created for name '{}'",
        name
    );

    resp.json::<InstanceResponse>()
        .await
        .expect("failed to parse create response")
}

/// Deletes an agent, ignoring errors (for cleanup).
pub async fn cleanup_agent(client: &Client, name: &str) {
    let url = format!("{}/instances/{}", api_url(), name);
    let _ = client.delete(&url).send().await;
    // Give K8s time to clean up resources
    tokio::time::sleep(Duration::from_secs(5)).await;
}

/// Polls GET /instances/:name until the phase matches or the timeout expires.
/// Returns the final InstanceResponse on success, panics on timeout.
pub async fn wait_for_phase(
    client: &Client,
    name: &str,
    expected_phase: &str,
    timeout: Duration,
) -> InstanceResponse {
    let url = format!("{}/instances/{}", api_url(), name);
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        if tokio::time::Instant::now() > deadline {
            panic!(
                "Timed out waiting for agent '{}' to reach phase '{}' (waited {:?})",
                name, expected_phase, timeout
            );
        }

        let resp = client
            .get(&url)
            .send()
            .await
            .expect("GET /instances/:name request failed");

        if resp.status().is_success() {
            let instance: InstanceResponse = resp
                .json()
                .await
                .expect("failed to parse instance response");

            if instance.phase.as_deref() == Some(expected_phase) {
                return instance;
            }
        }

        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Polls GET /instances/:name until it returns 404, indicating full deletion.
/// Panics on timeout.
pub async fn wait_for_deletion(client: &Client, name: &str, timeout: Duration) {
    let url = format!("{}/instances/{}", api_url(), name);
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        if tokio::time::Instant::now() > deadline {
            panic!(
                "Timed out waiting for agent '{}' to be fully deleted (waited {:?})",
                name, timeout
            );
        }

        let resp = client
            .get(&url)
            .send()
            .await
            .expect("GET /instances/:name request failed");

        if resp.status().as_u16() == 404 {
            return;
        }

        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

// ---------------------------------------------------------------------------
// Logs helpers
// ---------------------------------------------------------------------------

/// Fetches logs from GET /instances/:name/logs and returns the lines.
pub async fn get_logs(client: &Client, name: &str) -> Vec<String> {
    let url = format!("{}/instances/{}/logs?tail=100", api_url(), name);
    let resp = client
        .get(&url)
        .send()
        .await
        .expect("GET /instances/:name/logs request failed");

    if !resp.status().is_success() {
        return vec![];
    }

    let logs: LogsResponse = resp.json().await.expect("failed to parse logs response");
    logs.lines
}

/// Polls logs until they contain the expected substring, or times out.
/// Returns the full log output on success.
pub async fn wait_for_log_containing(
    client: &Client,
    name: &str,
    expected: &str,
    timeout: Duration,
) -> String {
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        if tokio::time::Instant::now() > deadline {
            let lines = get_logs(client, name).await;
            panic!(
                "Timed out waiting for logs of '{}' to contain '{}' (waited {:?}). Last logs: {:?}",
                name, expected, timeout, lines
            );
        }

        let lines = get_logs(client, name).await;
        let combined = lines.join("\n");
        if combined.contains(expected) {
            return combined;
        }

        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

// ---------------------------------------------------------------------------
// Composite setup helpers
// ---------------------------------------------------------------------------

/// Creates an agent with cleanup guard. Runs cleanup first, then creates.
/// Returns (client, name, _guard) — keep _guard alive for RAII cleanup.
pub async fn setup_agent(prefix: &str) -> (Client, String, AgentGuard) {
    let client = http_client();
    let name = unique_name(prefix);
    cleanup_agent(&client, &name).await;
    let guard = AgentGuard::new(&client, &name);
    create_agent(&client, &name).await;
    (client, name, guard)
}

/// Creates an agent and waits for Running phase.
/// Returns (client, name, _guard).
pub async fn setup_running_agent(prefix: &str) -> (Client, String, AgentGuard) {
    let (client, name, guard) = setup_agent(prefix).await;
    wait_for_phase(&client, &name, "Running", TIMEOUT_RUNNING).await;
    (client, name, guard)
}

/// Creates an agent with a custom command and waits for Running phase.
/// Returns (client, name, _guard).
pub async fn setup_running_command_agent(
    prefix: &str,
    command: Vec<&str>,
) -> (Client, String, AgentGuard) {
    let client = http_client();
    let name = unique_name(prefix);
    cleanup_agent(&client, &name).await;
    let guard = AgentGuard::new(&client, &name);
    create_agent_with_command(&client, &name, command).await;
    wait_for_phase(&client, &name, "Running", TIMEOUT_RUNNING).await;
    (client, name, guard)
}

/// Creates two agents with custom commands and waits for both to reach Running.
/// Returns (client, name_a, name_b, _guard_a, _guard_b).
pub async fn setup_two_running_command_agents(
    prefix_a: &str,
    command_a: Vec<&str>,
    prefix_b: &str,
    command_b: Vec<&str>,
) -> (Client, String, String, AgentGuard, AgentGuard) {
    let client = http_client();
    let name_a = unique_name(prefix_a);
    let name_b = unique_name(prefix_b);
    cleanup_agent(&client, &name_a).await;
    cleanup_agent(&client, &name_b).await;
    let guard_a = AgentGuard::new(&client, &name_a);
    let guard_b = AgentGuard::new(&client, &name_b);
    create_agent_with_command(&client, &name_a, command_a).await;
    create_agent_with_command(&client, &name_b, command_b).await;
    wait_for_phase(&client, &name_a, "Running", TIMEOUT_RUNNING).await;
    wait_for_phase(&client, &name_b, "Running", TIMEOUT_RUNNING).await;
    (client, name_a, name_b, guard_a, guard_b)
}

/// Creates two agents and waits for both to reach Running.
/// Returns (client, name_a, name_b, _guard_a, _guard_b).
pub async fn setup_two_running_agents(
    prefix_a: &str,
    prefix_b: &str,
) -> (Client, String, String, AgentGuard, AgentGuard) {
    let client = http_client();
    let name_a = unique_name(prefix_a);
    let name_b = unique_name(prefix_b);
    cleanup_agent(&client, &name_a).await;
    cleanup_agent(&client, &name_b).await;
    let guard_a = AgentGuard::new(&client, &name_a);
    let guard_b = AgentGuard::new(&client, &name_b);
    create_agent(&client, &name_a).await;
    create_agent(&client, &name_b).await;
    wait_for_phase(&client, &name_a, "Running", TIMEOUT_RUNNING).await;
    wait_for_phase(&client, &name_b, "Running", TIMEOUT_RUNNING).await;
    (client, name_a, name_b, guard_a, guard_b)
}

// ---------------------------------------------------------------------------
// RAII cleanup guard
// ---------------------------------------------------------------------------

/// Ensures an agent is deleted when the test finishes, even on panic.
/// Uses a background thread with its own tokio runtime to handle the async
/// deletion in Drop (which is synchronous).
pub struct AgentGuard {
    client: Client,
    name: String,
}

impl AgentGuard {
    pub fn new(client: &Client, name: &str) -> Self {
        Self {
            client: client.clone(),
            name: name.to_string(),
        }
    }
}

impl Drop for AgentGuard {
    fn drop(&mut self) {
        let client = self.client.clone();
        let name = self.name.clone();
        let _ = std::thread::spawn(move || {
            if let Ok(rt) = tokio::runtime::Runtime::new() {
                rt.block_on(async {
                    cleanup_agent(&client, &name).await;
                });
            }
        })
        .join();
    }
}

/// Polls GET /instances/:name until restart_count exceeds min_count or timeout.
/// Returns the InstanceResponse with the increased restart count on success.
/// Panics on timeout.
pub async fn wait_for_restart_count_increase(
    client: &Client,
    name: &str,
    min_count: i32,
    timeout: Duration,
) -> InstanceResponse {
    let url = format!("{}/instances/{}", api_url(), name);
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        if tokio::time::Instant::now() > deadline {
            panic!(
                "Timed out waiting for agent '{}' restart_count to exceed {} (waited {:?})",
                name, min_count, timeout
            );
        }

        let resp = client
            .get(&url)
            .send()
            .await
            .expect("GET /instances/:name request failed");

        if resp.status().is_success() {
            let instance: InstanceResponse = resp
                .json()
                .await
                .expect("failed to parse instance response");

            let current_count = instance.restart_count.unwrap_or(0);
            if current_count > min_count {
                return instance;
            }
        }

        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Creates an agent with Docker enabled using a Docker-in-Docker image.
pub async fn create_agent_with_docker(client: &Client, name: &str) -> InstanceResponse {
    let url = format!("{}/instances", api_url());
    let body = serde_json::json!({
        "name": name,
        "image": "docker:dind",  // Use official Docker-in-Docker image
        "cpu": "500m",  // DinD needs more resources
        "memory": "512Mi",
        "disk": "2Gi",
        "enable_docker": true,
        "security_profile": "trusted",  // DinD needs writable rootfs
    });

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .expect("POST /instances request failed");

    assert_eq!(
        resp.status().as_u16(),
        201,
        "create_agent_with_docker must return 201 Created for name '{}'",
        name
    );

    resp.json::<InstanceResponse>()
        .await
        .expect("failed to parse response")
}
