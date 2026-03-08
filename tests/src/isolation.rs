use crate::helpers::*;

/// **Test Case #23 — Cannot see other agent's processes**
///
/// @testops 9.1 Agent A cannot see agent B's processes (ps, /proc)
///
/// WHY THIS MATTERS:
/// Multi-tenant isolation requires that agents cannot see each other's
/// processes. PID namespace isolation prevents information leakage between
/// tenants.
///
/// WHAT THIS TEST DOES:
/// 1. Creates two agents (A and B), waits for both Running
/// 2. Execs `ps aux` inside agent A
/// 3. Asserts agent B's name does not appear in A's process list
///
/// IF THIS FAILS:
/// PID namespace isolation is broken. Agents can discover other tenants'
/// processes, which is a security violation.
#[tokio::test]
async fn test_p0_cannot_see_other_agent_processes() {
    let (_client, name_a, name_b, _guard_a, _guard_b) =
        setup_two_running_agents("iso-a", "iso-b").await;

    let (pods, pod_a) = pod_api(&name_a).await;

    let ps_output = exec_in_pod(&pods, &pod_a, vec!["ps", "aux"]).await;

    assert!(
        !ps_output.contains(&name_b),
        "agent A must not see agent B's processes in ps output"
    );
}

/// **Test Case #24 — Cannot access other agent's filesystem**
///
/// @testops 9.2 Agent A cannot read agent B's filesystem
///
/// WHY THIS MATTERS:
/// Each agent's filesystem must be isolated. One agent must not be able to
/// read or write another agent's PVC-backed storage.
///
/// WHAT THIS TEST DOES:
/// 1. Creates two agents (A and B), waits for both Running
/// 2. Writes a file in agent B's /home/agent
/// 3. Attempts to read that path from agent A
/// 4. Asserts the content is NOT the same (different PVC mounts)
///
/// IF THIS FAILS:
/// Agents share filesystem mounts. One tenant can read/modify another
/// tenant's data — a critical security breach.
#[tokio::test]
async fn test_p0_cannot_access_other_agent_filesystem() {
    let (_client, name_a, name_b, _guard_a, _guard_b) =
        setup_two_running_agents("fs-a", "fs-b").await;

    let (pods, _) = pod_api(&name_a).await;

    // Write secret in B
    let pod_b = format!("agent-{}", name_b);
    const SECRET: &str = "b-secret-data-77777";
    let write_cmd = format!("echo -n '{}' > /home/agent/secret", SECRET);
    exec_in_pod(&pods, &pod_b, vec!["sh", "-c", &write_cmd]).await;

    // Try to read from A's perspective — file won't exist
    let pod_a = format!("agent-{}", name_a);
    let result = exec_in_pod(
        &pods,
        &pod_a,
        vec!["cat", "/home/agent/secret"],
    )
    .await;

    assert_ne!(
        result.trim(),
        SECRET,
        "agent A must not be able to read agent B's files"
    );
}

/// **Test Case #25 — Cannot access host filesystem**
///
/// @testops 9.3 Agent cannot access host filesystem outside its allocation
///
/// WHY THIS MATTERS:
/// Agents must be confined to their own filesystem. Access to host paths
/// like /etc/rancher would expose cluster secrets and configuration.
///
/// WHAT THIS TEST DOES:
/// 1. Creates an agent and waits for Running
/// 2. Attempts to list /etc/rancher (K3s host config)
/// 3. Asserts the command fails (directory not found or permission denied)
///
/// IF THIS FAILS:
/// Host filesystem is mounted or accessible inside agent containers.
/// This is a critical container escape vector.
#[tokio::test]
async fn test_p1_cannot_access_host_filesystem() {
    let (_client, name, _guard) = setup_running_agent("hostfs").await;
    let (pods, pod_name) = pod_api(&name).await;

    let result =
        exec_in_pod(&pods, &pod_name, vec!["ls", "/etc/rancher"]).await;

    // The directory should not exist or should return an error
    assert!(
        result.contains("No such file") || result.contains("cannot access") || result.is_empty(),
        "host path /etc/rancher must not be accessible; got: {}",
        result
    );
}

/// **Test Case #26 — Cannot access Kubernetes API**
///
/// @testops 9.10 Agent cannot access host management services (Docker socket, Prometheus, APIs)
///
/// WHY THIS MATTERS:
/// Agents must not be able to reach the Kubernetes API server. Access would
/// allow container escape via service account token exploitation.
///
/// WHAT THIS TEST DOES:
/// 1. Creates an agent and waits for Running
/// 2. Attempts to curl the K8s API internal endpoint
/// 3. Asserts the connection is blocked or refused
///
/// IF THIS FAILS:
/// NetworkPolicy doesn't block access to the K8s API. Agents could list
/// pods, read secrets, or escalate privileges.
#[tokio::test]
async fn test_p1_cannot_access_host_management() {
    let (_client, name, _guard) = setup_running_agent("k8sapi").await;
    let (pods, pod_name) = pod_api(&name).await;

    // Attempt to reach the K8s API — should timeout or be refused
    let result = exec_in_pod(
        &pods,
        &pod_name,
        vec![
            "curl",
            "-sf",
            "--connect-timeout",
            "5",
            "https://kubernetes.default.svc/api",
        ],
    )
    .await;

    // The request should fail — empty output, connection refused, or timeout
    assert!(
        result.is_empty()
            || result.contains("refused")
            || result.contains("timed out")
            || result.contains("couldn't connect"),
        "K8s API must not be reachable from agent pods; got: {}",
        result
    );
}
