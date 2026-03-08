# nearai-infra — Agent Hosting Platform

Multi-tenant agent hosting platform where AI agents run on shared K3s clusters. Agents are managed as Kubernetes custom resources with full lifecycle support.

## Architecture

```
Internet → API Server (REST) / sshpiper (SSH)
                    ↓
              K3s Control Plane
                    ↓
            K3s Node (GCE VM)
              ↓         ↓
        Agent Pod    Agent Pod
        (PVC)        (PVC)
```

Each agent = 1 K8s Pod + 1 PVC + 1 NetworkPolicy + 1 Service, managed by a Rust operator watching Agent CRDs.

## Components

| Component | Path | Description |
|-----------|------|-------------|
| API Server | `api/` | Rust/Axum REST API (synchronous) |
| Operator | `operator/` | Rust/kube-rs controller reconciling Agent CRDs |
| K3s Scripts | `deploy/k3s/` | Cluster bootstrap and node join |
| K8s Manifests | `deploy/manifests/` | CRD, RBAC, deployments |
| Terraform | `deploy/terraform/` | GCE VM provisioning |

## Quick Start

### 1. Bootstrap K3s

On a Linux server (GCE VM or bare metal):

```bash
sudo deploy/k3s/install.sh
```

### 2. Build Images

```bash
make build
# Then build Docker images and import into K3s containerd:
docker build -f Dockerfile.api -t ghcr.io/nearai/agent-api:latest .
docker build -f Dockerfile.operator -t ghcr.io/nearai/agent-operator:latest .
docker save ghcr.io/nearai/agent-api:latest | sudo k3s ctr images import -
docker save ghcr.io/nearai/agent-operator:latest | sudo k3s ctr images import -
```

### 3. Deploy

```bash
export KUBECONFIG=/etc/rancher/k3s/k3s.yaml
scripts/deploy.sh
```

### 4. Verify

```bash
kubectl -n agents get pods
kubectl -n agents get agents
```

## API Reference

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/instances` | Create a new agent instance |
| `GET` | `/instances` | List all instances |
| `GET` | `/instances/{name}` | Get instance details and status |
| `DELETE` | `/instances/{name}` | Delete an instance |
| `POST` | `/instances/{name}/stop` | Stop a running instance |
| `POST` | `/instances/{name}/start` | Start a stopped instance |
| `POST` | `/instances/{name}/restart` | Restart instance (or upgrade image) |
| `GET` | `/instances/{name}/logs?tail=100` | Tail logs from agent pod |
| `GET` | `/health` | Health check |

All responses are synchronous JSON. Error format: `{"error": {"code": "...", "message": "...", "request_id": "..."}}`

### Examples

```bash
# Create
curl -X POST http://<host>:30080/instances \
  -H 'Content-Type: application/json' \
  -d '{"name": "my-agent", "image": "nearaidev/ironclaw-nearai-worker:latest"}'

# List
curl http://<host>:30080/instances

# Get status
curl http://<host>:30080/instances/my-agent

# Stop / Start / Restart
curl -X POST http://<host>:30080/instances/my-agent/stop
curl -X POST http://<host>:30080/instances/my-agent/start
curl -X POST http://<host>:30080/instances/my-agent/restart

# Tail logs
curl http://<host>:30080/instances/my-agent/logs?tail=50

# Delete
curl -X DELETE http://<host>:30080/instances/my-agent
```

### SSH Access

```bash
ssh my-agent@<host> -p 30022
```

## Agent Defaults

| Resource | Default |
|----------|---------|
| CPU | 1 vCPU |
| Memory | 4Gi |
| Disk (PVC) | 10Gi |
| Storage class | local-path |

| Volume mount | /home/agent |

Override per-agent: `{"cpu": "2", "memory": "8Gi", "disk": "20Gi", "volume_mount": "/home/myuser"}`

## What's Included (MVP)

- Agent lifecycle: create, start, stop, restart, delete
- SSH access to each agent instance
- Persistent local storage per agent (survives restarts)
- Auto-restart on agent crash (K8s `restartPolicy: Always`)
- Per-agent log tail via API
- Network isolation between agents (NetworkPolicy)
- Container hardening (non-root, read-only rootfs, dropped capabilities)

## Not Yet Included

- SSE streaming on lifecycle endpoints
- Backup/restore
- Per-subdomain HTTPS routing
- Webhooks
- Monitoring dashboards
- SLA guarantees

## CLI

A command-line client for the API is included in `cli/`.

```bash
cargo build --release -p agent-cli

# Set the API URL (or pass --api-url each time)
export AGENT_API_URL=http://136.119.211.246:30080

agent-cli health
agent-cli create my-agent ubuntu:24.04 --cpu 2 --memory 8Gi
agent-cli list
agent-cli get my-agent
agent-cli logs my-agent --tail 50
agent-cli stop my-agent
agent-cli start my-agent
agent-cli restart my-agent
agent-cli delete my-agent
```

### Deploying Ironclaw

Use the `nearaidev/ironclaw-nearai-worker` image. The entrypoint configures libSQL, SSH, and the gateway automatically. Mount the PVC at `/home/agent` (the image's home directory):

```bash
agent-cli create ironclaw nearaidev/ironclaw-nearai-worker:latest \
  --volume-mount /home/agent \
  --security-profile trusted \
  --port webhook:8080 --port webui:18789 --port ssh:22 \
  --env NEARAI_API_KEY=<your-key>
```

The first `--port` is used for health probes, so put a port that starts quickly (webhook:8080) first. Ports are exposed as NodePorts — use `agent-cli get ironclaw` to see the assigned external ports.

Optional env vars:
- `SSH_PUBKEY` — public key for SSH access into the agent
- `OPENCLAW_GATEWAY_TOKEN` — gateway auth token (default: `changeme`)
- `NEARAI_API_URL` — NEAR AI API endpoint (default: `https://cloud-api.near.ai/v1`)

The persistent volume at `/home/agent` preserves the database, config, and workspace across restarts.

## Development

```bash
make build        # Build both crates
make test         # Run tests
make fmt          # Format code
make clippy       # Lint
```

```bash
KUBECONFIG=~/.kube/config AGENT_NAMESPACE=agents cargo run -p agent-api
```
