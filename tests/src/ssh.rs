/// **Test Case #29 — SSH access via sshpiper**
///
/// @testops 12.1 SSH into a running agent instance — connection succeeds, shell functional
///
/// WHY THIS MATTERS:
/// SSH is the primary interactive access method for agents. Users connect
/// via sshpiper (port 30022), which routes to the correct agent pod based
/// on the username.
///
/// NOTE: This test is ignored because sshpiper integration is not yet
/// implemented in the operator.
#[tokio::test]
#[ignore = "sshpiper not yet integrated — SSH key generation missing from operator"]
async fn test_p1_ssh_via_sshpiper() {
    // This test would need to:
    // 1. Create an agent and wait for Running
    // 2. Retrieve the agent's SSH key from the API or K8s secret
    // 3. SSH to localhost:30022 with username=agent-name
    // 4. Execute `whoami` and verify the output
    //
    // Prerequisites:
    // - Operator generates SSH keypairs on agent creation
    // - sshpiper deployment is configured and running
    // - NodePort 30022 is accessible
    todo!("sshpiper integration not implemented");
}
