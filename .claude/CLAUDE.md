# Agent Hosting Platform (nearai-infra)

## Project Overview
Rust-based agent hosting platform for NEAR AI. Manages AI agent instances on K3s clusters using Kubernetes CRDs.

## Architecture
- **API** (`api/`): Axum REST server on port 8080, package `agent-api`
- **Operator** (`operator/`): kube-rs controller, package `agent-operator`
- **CRD**: `agents.agents.near.ai` (group: `agents.near.ai`, version: `v1`, kind: `Agent`)
- **Workspace**: Cargo workspace root at `Cargo.toml`

## Key Files
- `api/src/main.rs` — Axum router setup, all routes
- `api/src/handlers.rs` — Request handlers (create, list, get, delete, start, stop, restart, logs)
- `api/src/crd.rs` — Agent CRD definition (shared type, must stay in sync with operator)
- `api/src/error.rs` — AppError enum → JSON responses
- `operator/src/main.rs` — Controller setup, CRD registration
- `operator/src/controller.rs` — Reconcile loop: ensures Pod, PVC, NetworkPolicy, Service
- `operator/src/resources.rs` — K8s resource builders (build_pod, build_pvc, etc.)
- `operator/src/crd.rs` — Same CRD definition (must match api/src/crd.rs)
- `deploy/manifests/` — K8s YAML manifests
- `deploy/terraform/main.tf` — GCE VM provisioning

## MVP API Endpoints
- `GET /health`
- `POST /instances` — Create agent instance
- `GET /instances` — List all instances
- `GET /instances/{name}` — Get instance details
- `DELETE /instances/{name}` — Delete instance
- `POST /instances/{name}/start`
- `POST /instances/{name}/stop`
- `POST /instances/{name}/restart`
- `GET /instances/{name}/logs?tail=N` — Tail pod logs

## Deployment
- **GCE VM**: `agent-hosting-test` in `us-central1-a`, IP `136.119.211.246`
- **Code on VM**: `/opt/nearai-infra/` (copied, not git cloned)
- **Cargo on VM**: `/home/yan/.cargo/bin/cargo`
- **K3s**: Running, namespace `agents`
- **Docker images**: Built locally on VM from pre-built binaries, imported via `docker save | k3s ctr images import`
- **API Service**: ClusterIP on `10.43.42.18:8080` — needs to be changed to NodePort 30080 for external access
- **SSH proxy**: NodePort 2222/30022

## Build & Deploy Flow on VM
```bash
# SSH to VM
gcloud compute ssh agent-hosting-test --zone=us-central1-a

# Build
cd /opt/nearai-infra
export PATH="/home/yan/.cargo/bin:$PATH"
cargo build --release

# Build Docker images (use pre-built binaries, not multi-stage)
# The Dockerfiles in repo have rustc version mismatch; use prebuilt approach:
cat > /tmp/Dockerfile.api.prebuilt << 'EOF'
FROM ubuntu:24.04
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY target/release/agent-api /usr/local/bin/agent-api
ENTRYPOINT ["/usr/local/bin/agent-api"]
EOF
sudo docker build -f /tmp/Dockerfile.api.prebuilt -t ghcr.io/nearai/agent-api:latest .
# Same pattern for operator

# Import to K3s
sudo docker save ghcr.io/nearai/agent-api:latest | sudo k3s ctr images import -

# Restart
sudo kubectl -n agents rollout restart deployment/agent-api
sudo kubectl -n agents rollout restart deployment/agent-operator
```

## Important Conventions
- CRD group is `agents.near.ai` (not `near.ai`) — name becomes `agents.agents.near.ai`
- `AgentState` enum: `Running`/`Stopped`, serialized lowercase via `#[serde(rename_all = "lowercase")]`
- `AgentStatus` fields are all `Option<T>`
- All K8s sub-resources use owner references for cascade delete
- Pod naming: `agent-{name}`
- Container base image: `ubuntu:24.04` (not Debian bookworm — GLIBC 2.39 needed)
- AGENT_NAMESPACE env var defaults to `"agents"`

## Git Remote
- Remote: `git@github-yanatnear:yanatnear/agent-hosting-infra-test.git`
- Uses SSH host alias `github-yanatnear` (configured in `~/.ssh/config`) for 1Password identity selection as `yanatnear`

## Architecture & Roadmap
- See `docs/ARCHITECTURE.md` for full architecture design, priority tiers, and test coverage mapping
- Test case doc: https://docs.google.com/document/d/1tiPQGtPxVh3uUMV1RfjLzlMCLNBkD62ZNZiUIWPSOMA
- Test management: `nearai-infra-testops/` (Next.js + CLI for organizing 290 test cases)

## Pending Work
- Expose API externally: change Service from ClusterIP to NodePort 30080
- Fix Dockerfiles to match VM rustc version or keep using prebuilt approach
- `runAsNonRoot` security context prevents nginx test images from running (expected; real agent images should use non-root)
- GCP firewall rule may be needed for NodePort 30080
- SSE streaming for lifecycle operations
- Backup/restore API + MinIO integration
- SSH proxy (sshpiper) setup
- HTTPS subdomain routing (Traefik wildcard ingress)
- Monitoring stack (kube-prometheus-stack)
- gVisor RuntimeClass for sub-agents
