# Test Execution Guide

## Test Categories

### 1. Unit Tests (40 total)
- Location: `api/src/`, `operator/src/`
- Execution: `cargo test --lib` (in respective crate)
- Requirements: None (pure unit tests)
- Status: **✅ All 40 passing**

### 2. Integration Tests (29 total)

#### A. API General Tests (3 tests) - REMOTE EXECUTION ✅
- `test_p0_get_instance_details`
- `test_p0_list_instances`
- `test_p2_error_format_consistent`
- Execution: HTTP API calls only
- Requirements: API URL via `AGENT_API_URL` env var
- Status: **✅ All 3 passing**

#### B. Creation Tests (5 tests)
- **Blackbox/API-only (2)** - Can run remotely ✅
  - `test_p0_create_reaches_running` ✅
  - `test_p1_duplicate_name_conflict` ✅
  - `test_p1_invalid_params_error` ✅

- **Whitebox/Requires K8s client (3)** - Need KUBECONFIG ❌
  - `test_p0_crash_auto_restarts` (needs pod watch)
  - `test_p0_data_persists_across_restart` (needs pod restart monitoring)
  - `test_p0_outbound_https` (needs pod exec)
  - `test_p0_writable_persistent_filesystem` (needs pod exec)
  - `test_p0_spawn_sub_agent_docker` (needs pod watch)

#### C. Lifecycle Tests (3 tests) - REMOTE EXECUTION ✅
- Tests that verify agent state transitions via API
- Status: **✅ Ready for remote execution**

#### D. Deletion Tests (2 tests)
- **Blackbox (1)** - API-only ✅
  - `test_p0_delete_running_cleans_up` ✅

- **Whitebox (1)** - Requires K8s client ❌
  - `test_p0_delete_stopped_all_resources_gone` (needs K8s observation)

#### E. Isolation Tests - Whitebox ❌
- `test_p0_cannot_access_other_agent_filesystem` (needs K8s client)

#### F. Host Health Tests - Ignored ⏭️
- `test_p0_log_rotation_configured` (needs host access)
- `test_p1_node_exporter_running` (needs Prometheus URL)

#### G. SSH Tests - Not Implemented ⏭️
- SSH key generation pending

#### H. Logs Tests - Not Implemented ⏭️
- Endpoint not implemented

## Execution Scenarios

### Scenario 1: Remote Test Client (No K8s Access)
```bash
# On separate machine with API access
export AGENT_API_URL="http://{cluster-ip}:30080"

# Run API-only tests
cargo test --lib api_general -- --test-threads=1
cargo test --lib creation::test_p0_create_reaches_running -- --test-threads=1
cargo test --lib creation::test_p1_duplicate_name_conflict -- --test-threads=1
cargo test --lib deletion::test_p0_delete_running_cleans_up -- --test-threads=1

# Expected: 7/7 tests pass
```

### Scenario 2: Cluster Node (Has K8s Access)
```bash
# On cluster node with kubectl access
export KUBECONFIG=/etc/rancher/k3s/k3s.yaml
export AGENT_API_URL="http://localhost:30080"

# Run all tests including whitebox K8s tests
cargo test --lib -- --test-threads=1

# Expected: ~19-20 tests pass, 2 ignored (host_health)
```

## Current Test Pass Rate

| Category | Pass | Total | % | Notes |
|----------|------|-------|---|-------|
| Unit Tests (api) | 12 | 12 | 100% | ✅ |
| Unit Tests (tests) | 0 | 0 | N/A | Integration tests only |
| API General | 3 | 3 | 100% | ✅ |
| Creation (API) | 3 | 3 | 100% | ✅ |
| Creation (K8s) | 0 | 5 | 0% | ❌ Needs KUBECONFIG |
| Deletion (API) | 1 | 1 | 100% | ✅ |
| Deletion (K8s) | 0 | 1 | 0% | ❌ Needs KUBECONFIG |
| Isolation | 0 | 1 | 0% | ❌ Needs KUBECONFIG |
| Host Health | 0 | 2 | 0% | ⏭️ Ignored |
| SSH | 0 | 1 | 0% | ⏭️ Not implemented |
| Logs | 0 | 1 | 0% | ⏭️ Not implemented |
| **TOTAL** | **19** | **29** | **66%** | 7 fail (K8s access), 3 ignored/pending |

## Recommended Testing Strategy

1. **Dev/Testing**: Run on cluster node with `KUBECONFIG` set
   - Get full coverage including whitebox tests

2. **CI/CD (Remote)**: Run API-only tests from separate machine
   - 7 passing API-focused tests
   - No infrastructure assumptions

3. **Blackbox Acceptance**: Run on test client
   - Validates API contract
   - Independent of K8s implementation details

## Future Improvements

To get all tests passing on remote clients:
- Convert whitebox K8s tests to blackbox API tests
- Use agent logs endpoint to verify pod behavior
- Mock K8s client for isolation tests
- Implement SSH key generation for SSH tests
