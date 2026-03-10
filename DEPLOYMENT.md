# Deployment Guide

Complete step-by-step instructions for deploying the agent hosting platform.

## Prerequisites

- Ubuntu 24.04 LTS
- 4+ CPU cores, 8GB+ RAM
- Docker installed
- Git with SSH key configured for GitHub

## 1. Clone Repository

```bash
git clone git@github.com:yanatnear/agent-hosting-infra-test.git
cd agent-hosting-infra-test
```

## 2. Install K3s

```bash
curl -sfL https://get.k3s.io | sh -
```

Verify:
```bash
sudo k3s kubectl get nodes
export KUBECONFIG=/etc/rancher/k3s/k3s.yaml
kubectl get nodes
```

## 3. Build Docker Images

```bash
# Build agent-api (assumes Dockerfile in api/ dir)
docker build -t agent-api:latest . -f api/Dockerfile

# Build agent-operator
docker build -t agent-operator:latest . -f operator/Dockerfile

# Or if Dockerfiles are not in repo, build from source:
cd api && cargo build --release && cd ..
cd operator && cargo build --release && cd ..
```

## 4. Load Images into K3s

```bash
# Import into containerd (K3s default runtime)
docker save agent-api:latest | ctr -n k8s.io images import /dev/stdin
docker save agent-operator:latest | ctr -n k8s.io images import /dev/stdin

# Verify
ctr -n k8s.io images ls | grep agent
```

## 5. Apply RBAC Manifests

**Critical:** RBAC must be applied before deploying API and Operator.

```bash
# Create agents namespace
kubectl create namespace agents

# Apply RBAC for agent-api
kubectl apply -f kubernetes/rbac/agent-api-rbac.yaml

# Apply RBAC for agent-operator
kubectl apply -f kubernetes/rbac/agent-operator-rbac.yaml

# Verify service accounts
kubectl get sa -n agents
kubectl get clusterrole | grep agent-
kubectl get clusterrolebinding | grep agent-
```

## 6. Deploy Services

Option A: Use deploy script (recommended)
```bash
bash scripts/deploy.sh
```

Option B: Manual deployment
```bash
# Apply Agent CRD
kubectl apply -f kubernetes/agent-crd.yaml

# Deploy MinIO (storage)
kubectl apply -f kubernetes/minio-deployment.yaml

# Deploy agent-operator
kubectl apply -f kubernetes/agent-operator-deployment.yaml

# Deploy agent-api
kubectl apply -f kubernetes/agent-api-deployment.yaml

# Deploy SSH proxy (optional)
kubectl apply -f kubernetes/ssh-proxy-deployment.yaml
```

## 7. Verify Deployment

```bash
# Check all pods are running
kubectl get pods -n agents

# API should respond
curl http://localhost:30080/health

# Check operator logs
kubectl logs -n agents deployment/agent-operator -f
```

## 8. Create Test Agent

```bash
# Create agent with trusted security profile (required for file operations)
kubectl apply -f - << 'AGENT'
apiVersion: agents.near.ai/v1
kind: Agent
metadata:
  name: test-agent
  namespace: agents
spec:
  image: nearaidev/ironclaw-nearai-worker:latest
  cpu: "100m"
  memory: "256Mi"
  disk: "1Gi"
  security_profile: "trusted"  # Important: allows chown capabilities
  enable_docker: false
AGENT
```

## 9. Verify Agent Pod

```bash
# Wait for pod to reach Running state
kubectl get pods -n agents -w

# Check pod logs
kubectl logs -n agents pod/agent-test-agent --tail=50
```

## Key Configuration

### Security Profiles

Two security profiles are available:

- **`restricted`** (default): Non-root, read-only rootfs, no privilege escalation
  - Suitable for stateless workloads
  - More secure but limited

- **`trusted`**: Allows root-capable operations, writable filesystem
  - Required for agents that need file ownership, Docker, etc.
  - Use explicitly via `security_profile: "trusted"` in Agent spec

### Resource Defaults

- CPU: 100m (request), 500m (limit)
- Memory: 128Mi (request), 512Mi (limit)
- Disk: 1Gi (PVC)

Override in Agent spec:
```yaml
spec:
  cpu: "500m"
  memory: "1Gi"
  disk: "10Gi"
```

### Environment Variables

Pass environment variables to agent pods:
```yaml
spec:
  env:
  - name: DEBUG
    value: "1"
  - name: LOG_LEVEL
    value: "debug"
```

## Troubleshooting

### Agent pods stay Pending

**Cause:** Not enough cluster resources allocated  
**Fix:** Check node resources and reduce pod resource requests

```bash
kubectl describe nodes localhost
kubectl top nodes  # If metrics-server is installed
```

### Permission denied / chown errors

**Cause:** Agent image needs writable filesystem but security profile is restrictive  
**Fix:** Use `security_profile: "trusted"` in Agent spec

```bash
kubectl get agent {name} -n agents -o yaml | grep security_profile
# Should show: security_profile: trusted
```

### API returns 403 Forbidden on Agent CRD access

**Cause:** RBAC not applied or service account not patched  
**Fix:** Verify RBAC is in place and API pod uses correct service account

```bash
kubectl get clusterrole agent-api-role
kubectl get clusterrolebinding agent-api-rolebinding
kubectl get deployment agent-api -n agents -o yaml | grep serviceAccountName
# Should show: serviceAccountName: agent-api
```

### Operator not reconciling agents

**Cause:** Operator RBAC missing or operator not running  
**Fix:** Check operator logs and RBAC

```bash
kubectl logs -n agents deployment/agent-operator --tail=100
kubectl get clusterrole agent-operator-role
kubectl get clusterrolebinding agent-operator-rolebinding
```

## Architecture Notes

- **API Server**: REST service on port 30080 (NodePort)
- **SSH Proxy**: SSH service on port 30022 (NodePort) for agent shell access
- **Operator**: Watches Agent CRDs and creates/updates pods
- **Storage**: MinIO for agent data (optional, can use host storage)
- **Networking**: NetworkPolicy isolates agent traffic by default

## Integration Testing

```bash
# On separate machine (test client)
export AGENT_API_URL="http://{cluster-ip}:30080"
export PROMETHEUS_URL="http://{cluster-ip}:9090"  # Optional

cd tests
cargo test --lib api_general -- --nocapture --test-threads=1
```

Expected: All api_general tests pass (3/3)

## Production Considerations

- Use persistent storage backend (not ephemeral)
- Configure proper resource quotas and limits
- Enable network policies for isolation
- Use proper TLS certificates (not localhost)
- Set up monitoring/alerting (Prometheus + Grafana)
- Implement backup/restore for agent data
- Use proper authentication (not just API access)
