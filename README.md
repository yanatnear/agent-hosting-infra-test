# nearai-infra — Agent Hosting Platform

Multi-tenant agent hosting platform where AI agents run on shared K3s clusters. Agents are managed as Kubernetes custom resources with full lifecycle support (create, start, stop, restart, backup, delete).

## Architecture

```
Internet → GCP LB → Traefik (HTTPS) / sshpiper (SSH) / API Server (REST)
                          ↓
                    K3s Control Plane
                          ↓
              K3s Agent Nodes (GCE VMs)
                    ↓         ↓
              Agent Pod    Agent Pod
              (runc)       (runc)
                ↓
           Sub-agent sidecar (Sysbox RuntimeClass)
```

Each agent = 1 K8s Pod + 1 PVC + 1 NetworkPolicy + 1 Service, managed by a Rust operator watching Agent CRDs.

## Components

| Component | Path | Description |
|-----------|------|-------------|
| API Server | `api/` | Rust/Axum REST API with SSE streaming |
| Operator | `operator/` | Rust/kube-rs controller reconciling Agent CRDs |
| K3s Scripts | `deploy/k3s/` | Cluster bootstrap and node join |
| K8s Manifests | `deploy/manifests/` | All YAML (CRD, RBAC, deployments, monitoring) |
| Terraform | `deploy/terraform/` | GCE VM provisioning |
| Deploy Script | `scripts/deploy.sh` | Full cluster deployment orchestration |

## Prerequisites

- Rust 1.85+ (for building API and operator)
- Docker (for container images)
- `kubectl` and `helm` (for deployment)
- Terraform (for GCE VM provisioning)
- GCP project with Compute Engine API enabled (or bare metal servers)

## Quick Start

### 1. Provision VMs (GCP)

```bash
cd deploy/terraform
cp terraform.tfvars.example terraform.tfvars  # edit with your values
terraform init
terraform apply
```

Or use existing servers — K3s runs on any Linux box.

### 2. Bootstrap K3s Cluster

On the first server node:

```bash
sudo deploy/k3s/install.sh
```

This installs K3s, Sysbox runtime, and applies the RuntimeClass. It prints a join token.

On each worker node:

```bash
sudo deploy/k3s/join.sh https://<server-ip>:6443 <join-token>
```

### 3. Build & Push Images

```bash
make docker push REGISTRY=ghcr.io/yourorg TAG=latest
```

Or build locally for development:

```bash
make build
```

### 4. Deploy Platform

```bash
export KUBECONFIG=/etc/rancher/k3s/k3s.yaml
scripts/deploy.sh
```

This applies (in order): namespace, CRD, RuntimeClass, MinIO, operator, API server, SSH proxy, Traefik ingress, and monitoring (Prometheus + Loki via Helm).

### 5. Verify

```bash
kubectl -n agents get pods
kubectl -n agents get agents   # should be empty initially
```

## Usage

### Create an Agent

```bash
curl -N -X POST http://<api-server>:8080/instances \
  -H 'Content-Type: application/json' \
  -d '{"name": "my-agent", "image": "ghcr.io/nearai/ironclaw:latest"}'
```

Returns an SSE stream of status updates until the agent is running.

### List Agents

```bash
curl http://<api-server>:8080/instances
```

### Stop / Start / Restart

```bash
curl -X POST http://<api-server>:8080/instances/my-agent/stop
curl -X POST http://<api-server>:8080/instances/my-agent/start
curl -X POST http://<api-server>:8080/instances/my-agent/restart
```

### SSH into an Agent

```bash
ssh my-agent@ssh.agents.near.ai
```

### HTTPS Access

Each agent is accessible at `https://<name>.agents.near.ai` (requires DNS + wildcard cert setup).

### Delete an Agent

```bash
curl -X DELETE http://<api-server>:8080/instances/my-agent
```

### Trigger Backup

```bash
curl -N -X POST http://<api-server>:8080/instances/my-agent/backup
```

## API Reference

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/health` | Health check |
| `POST` | `/instances` | Create agent (SSE stream) |
| `GET` | `/instances` | List all agents |
| `GET` | `/instances/{name}` | Get agent details |
| `POST` | `/instances/{name}/start` | Start agent |
| `POST` | `/instances/{name}/stop` | Stop agent (keeps PVC) |
| `POST` | `/instances/{name}/restart` | Restart agent pod |
| `DELETE` | `/instances/{name}` | Delete agent and all resources |
| `POST` | `/instances/{name}/backup` | Trigger backup (SSE stream) |
| `GET` | `/instances/{name}/backups` | List backups |

Error responses: `{"error": {"code": "...", "message": "...", "request_id": "..."}}`

## Agent Defaults

| Resource | Default |
|----------|---------|
| CPU | 1 vCPU |
| Memory | 4Gi |
| Disk (PVC) | 10Gi |
| Storage class | local-path |

Override per-agent via the create request: `{"cpu": "2", "memory": "8Gi", "disk": "20Gi"}`.

## Monitoring

After deployment, access:

- **Grafana**: `kubectl -n monitoring port-forward svc/monitoring-grafana 3000:80`
- **Prometheus**: `kubectl -n monitoring port-forward svc/monitoring-kube-prometheus-prometheus 9090:9090`

Alert rules are pre-configured for: host CPU/memory/disk, agent crash loops, OOM kills, stale backups, node unreachable, and capacity exhaustion.

## Development

```bash
make build        # Build both crates
make test         # Run tests
make fmt          # Format code
make clippy       # Lint
make crd-gen      # Regenerate CRD YAML from Rust types
```

To run the API server locally (requires kubeconfig):

```bash
KUBECONFIG=~/.kube/config AGENT_NAMESPACE=agents cargo run -p agent-api
```

## Bare Metal Migration

No GCP-specific services are used. To migrate:

1. Replace Terraform with Ansible for node provisioning
2. Use MetalLB + HAProxy instead of GCP load balancer
3. Everything else (K3s, operator, manifests, MinIO, monitoring) is identical
