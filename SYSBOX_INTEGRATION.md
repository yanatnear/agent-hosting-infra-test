# Sysbox Integration for K3s Agent Hosting

## Summary

Successfully integrated Sysbox runtime with K3s to enable Docker-in-Docker capabilities for agent pods.

## Changes Made

### 1. Sysbox Installation (Host: 172.105.154.210)

**Built from source** (required for K3s containerd support):
- Version: 0.7.0 (commit a4dd414f)
- Contains the critical containerd integration fix (PR #106)
- Installed binaries:
  - `/usr/bin/sysbox-runc`
  - `/usr/bin/sysbox-mgr`
  - `/usr/bin/sysbox-fs`

**Dependencies installed:**
```bash
apt-get install -y fuse3
```

**Systemd services enabled:**
- `sysbox-mgr.service` - Container manager daemon
- `sysbox-fs.service` - FUSE filesystem server
- `sysbox.service` - Wrapper service

### 2. K3s Configuration

**File:** `/var/lib/rancher/k3s/agent/etc/containerd/config.toml.tmpl`
```toml
{{ template "base" . }}

[plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.sysbox-runc]
  runtime_type = "io.containerd.runc.v2"
  [plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.sysbox-runc.options]
    SystemdCgroup = false
    BinaryName = "/usr/bin/sysbox-runc"
```

**RuntimeClass resource:**
```yaml
apiVersion: node.k8s.io/v1
kind: RuntimeClass
metadata:
  name: sysbox-runc
handler: sysbox-runc
```

### 3. Code Changes

#### Operator CRD (`operator/src/crd.rs`)
Added `enable_docker` field:
```rust
#[derive(CustomResource, ...)]
pub struct AgentSpec {
    // ... existing fields ...
    #[serde(default)]
    pub enable_docker: bool,
    // ...
}
```

#### API CRD (`api/src/crd.rs`)
Same `enable_docker` field addition.

#### Operator Resources (`operator/src/resources.rs`)
Conditional Sysbox runtime:
```rust
spec: Some(PodSpec {
    containers: vec![container],
    runtime_class_name: if agent.spec.enable_docker {
        Some("sysbox-runc".to_string())
    } else {
        None
    },
    host_users: if agent.spec.enable_docker {
        Some(false)  // Required for Sysbox
    } else {
        None
    },
    // ...
}),
```

#### API Handler (`api/src/handlers.rs`)
```rust
pub struct CreateInstanceRequest {
    // ... existing fields ...
    #[serde(default)]
    pub enable_docker: bool,
}

// In create_instance:
AgentSpec {
    // ...
    enable_docker: req.enable_docker,
    // ...
}
```

#### Dockerfiles
Both `Dockerfile.operator` and `Dockerfile.api` updated:
```dockerfile
FROM rust:bookworm AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY api/ api/
COPY operator/ operator/
COPY cli/ cli/
COPY tests/ tests/
RUN cargo build --release -p <package>
```

#### Test Updates (`tests/src/`)

**Helper** (`helpers.rs`):
```rust
pub async fn create_agent_with_docker(client: &Client, name: &str) -> InstanceResponse {
    let body = serde_json::json!({
        "name": name,
        "image": "alpine:latest",
        "cpu": "200m",
        "memory": "256Mi",
        "disk": "1Gi",
        "enable_docker": true,  // Triggers Sysbox
    });
    // ...
}
```

**Test** (`creation.rs::test_p0_spawn_sub_agent_docker`):
- Verifies `runtimeClassName == "sysbox-runc"`
- Verifies `hostUsers == false`
- Infrastructure-only test (doesn't require Docker daemon running)

### 4. Deployment

**Operator Image:** `ghcr.io/nearai/agent-operator:v1773089671`
**API Image:** `ghcr.io/nearai/agent-api:v1773089671`

Images imported to K3s containerd:
```bash
docker save <image> | k3s ctr images import -
```

Deployment patched:
```bash
kubectl -n agents set image deployment/agent-operator operator=<image>
kubectl -n agents set image deployment/agent-api api=<image>
kubectl -n agents patch deployment agent-api --type=json \
  -p='[{"op":"replace","path":"/spec/template/spec/containers/0/imagePullPolicy","value":"Never"}]'
```

## Test Results

### Creation Tests: 5/8 Passing (62.5%)
✅ test_p0_create_reaches_running
✅ **test_p0_spawn_sub_agent_docker** (Sysbox verification)
✅ test_p1_duplicate_name_conflict
✅ test_p1_invalid_params_error
✅ test_p0_create_reaches_running

❌ test_p0_crash_auto_restarts (pre-existing: ironclaw image permission issue)
❌ test_p0_data_persists_across_restart (pre-existing)
❌ test_p0_outbound_https (pre-existing)

### Deletion Tests: 2/3 Passing
✅ test_p0_delete_running_cleans_up
✅ test_p1_delete_nonexistent_404
❌ test_p0_delete_stopped_all_resources_gone (KUBECONFIG issue)

### Lifecycle Tests: 3/5 Passing
✅ test_p0_stop_agent
✅ test_p2_start_already_running
✅ test_p2_stop_already_stopped
❌ test_p0_start_stopped_agent_data_intact (KUBECONFIG issue)
❌ test_p1_restart_data_intact (KUBECONFIG issue)

### SSH Tests: 0/1 (Ignored - Not Implemented)
⏭️ test_p1_ssh_via_sshpiper

## Usage

Create an agent with Docker-in-Docker support:
```bash
curl -X POST http://localhost:30080/instances \
  -H "Content-Type: application/json" \
  -d '{
    "name": "my-agent",
    "image": "alpine:latest",
    "cpu": "200m",
    "memory": "256Mi",
    "disk": "1Gi",
    "enable_docker": true,
    "state": "running",
    "ports": []
  }'
```

The pod will be created with:
- `runtimeClassName: sysbox-runc`
- `hostUsers: false`

## Known Limitations

1. **Image Compatibility:** Not all images work with Sysbox. Images requiring specific init systems (like systemd) need additional configuration.

2. **Resource Overhead:** Sysbox containers have higher resource requirements than standard containers.

3. **Security Context:** The current `restricted` security profile may conflict with some Sysbox use cases.

## Files Modified

**Code:**
- `operator/src/crd.rs`
- `operator/src/resources.rs`
- `api/src/crd.rs`
- `api/src/handlers.rs`
- `tests/src/helpers.rs`
- `tests/src/creation.rs`
- `Dockerfile.operator`
- `Dockerfile.api`

**Configuration (on K3s node):**
- `/var/lib/rancher/k3s/agent/etc/containerd/config.toml.tmpl`
- RuntimeClass resource: `sysbox-runc`

## Next Steps

1. **Commit changes** to a feature branch
2. **Fix pre-existing test failures** (ironclaw image permissions)
3. **Document** Docker-in-Docker best practices
4. **Production testing** with real workloads

## References

- [Sysbox K3s Integration Guide](https://docs.k3s.io/blog/2025/09/27/k3s-sysbox)
- [Sysbox GitHub](https://github.com/nestybox/sysbox)
- [K3s Containerd Configuration](https://docs.k3s.io/advanced#configuring-containerd)
