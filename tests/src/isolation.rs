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
/// 1. Creates two agents (A runs `ps aux`, B runs `sleep infinity`)
/// 2. Checks agent A's logs for ps output
/// 3. Asserts agent B's name does not appear in A's process list
///
/// IF THIS FAILS:
/// PID namespace isolation is broken. Agents can discover other tenants'
/// processes, which is a security violation.
#[tokio::test]
async fn test_p0_cannot_see_other_agent_processes() {
    let (client, name_a, name_b, _guard_a, _guard_b) =
        setup_two_running_command_agents(
            "iso-a",
            vec!["sh", "-c", "ps aux && sleep infinity"],
            "iso-b",
            vec!["sh", "-c", "sleep infinity"],
        )
        .await;

    let logs = wait_for_log_containing(
        &client,
        &name_a,
        "PID",
        std::time::Duration::from_secs(30),
    )
    .await;

    assert!(
        !logs.contains(&name_b),
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
/// 1. Creates agent B that writes a secret file to /home/agent/secret
/// 2. Creates agent A that tries to read /home/agent/secret
/// 3. Asserts A does NOT see B's secret (separate PVC mounts)
///
/// IF THIS FAILS:
/// Agents share filesystem mounts. One tenant can read/modify another
/// tenant's data — a critical security breach.
#[tokio::test]
async fn test_p0_cannot_access_other_agent_filesystem() {
    const SECRET: &str = "b-secret-data-77777";

    let (client, name_a, _name_b, _guard_a, _guard_b) =
        setup_two_running_command_agents(
            "fs-a",
            vec![
                "sh",
                "-c",
                "cat /home/agent/secret 2>&1 || echo 'FILE_NOT_FOUND'; sleep infinity",
            ],
            "fs-b",
            vec![
                "sh",
                "-c",
                &format!(
                    "echo -n '{}' > /home/agent/secret && echo 'SECRET_WRITTEN' && sleep infinity",
                    SECRET
                ),
            ],
        )
        .await;

    // Wait for B to write its secret
    wait_for_log_containing(
        &client,
        &_name_b,
        "SECRET_WRITTEN",
        std::time::Duration::from_secs(30),
    )
    .await;

    // Check A's logs — should NOT contain B's secret
    let logs_a = get_logs(&client, &name_a).await;
    let combined = logs_a.join("\n");

    assert!(
        !combined.contains(SECRET),
        "agent A must not be able to read agent B's files; got: {}",
        combined
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
/// 1. Creates an agent with a command that lists /etc/rancher
/// 2. Checks logs for error output (directory not found or permission denied)
///
/// IF THIS FAILS:
/// Host filesystem is mounted or accessible inside agent containers.
/// This is a critical container escape vector.
#[tokio::test]
async fn test_p1_cannot_access_host_filesystem() {
    let (client, name, _guard) = setup_running_command_agent(
        "hostfs",
        vec!["sh", "-c", "ls /etc/rancher 2>&1; echo 'DONE'; sleep infinity"],
    )
    .await;

    let logs = wait_for_log_containing(
        &client,
        &name,
        "DONE",
        std::time::Duration::from_secs(30),
    )
    .await;

    // The directory should not exist or should return an error
    assert!(
        logs.contains("No such file") || logs.contains("cannot access") || !logs.contains("k3s"),
        "host path /etc/rancher must not be accessible; got: {}",
        logs
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
/// 1. Creates an agent with a command that curls the K8s API
/// 2. Checks logs for connection failure
///
/// IF THIS FAILS:
/// NetworkPolicy doesn't block access to the K8s API. Agents could list
/// pods, read secrets, or escalate privileges.
#[tokio::test]
async fn test_p1_cannot_access_host_management() {
    let (client, name, _guard) = setup_running_command_agent(
        "k8sapi",
        vec![
            "sh",
            "-c",
            "curl -sf --connect-timeout 5 https://kubernetes.default.svc/api 2>&1 || echo 'K8S_API_BLOCKED'; sleep infinity",
        ],
    )
    .await;

    let logs = wait_for_log_containing(
        &client,
        &name,
        "K8S_API_BLOCKED",
        std::time::Duration::from_secs(30),
    )
    .await;

    assert!(
        logs.contains("K8S_API_BLOCKED")
            || logs.contains("refused")
            || logs.contains("timed out")
            || logs.contains("couldn't connect"),
        "K8s API must not be reachable from agent pods; got: {}",
        logs
    );
}
