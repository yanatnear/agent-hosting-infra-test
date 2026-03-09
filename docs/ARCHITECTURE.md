# Agent Hosting Platform - Architecture Design

## Context

NEAR needs a **multi-tenant agent hosting platform** where many AI agents (IronClaw/OpenClaw/other) run on shared hosts. The test case document defines 290 tests across 12 categories. Architecture must support GCloud (VMs only, no GCP-specific services) initially, then migrate to bare metal with minimal changes. Code lives in `nearai-infra/`.

**Key decisions:**
- **Orchestration:** K3s (lightweight Kubernetes)
- **Isolation:** Sysbox runtime for all agent pods (enables Docker-in-Docker for sub-agents)
- **Agent image:** Product-defined (IronClaw, OpenClaw, or other)
- **Infrastructure:** GCE VMs now, bare metal later
- **Storage:** Local PVCs (local-path or TopoLVM) for live filesystems, MinIO (S3-compatible) for backups
- **TEE:** Not required

---

## Architecture Overview

```
                         Internet
                            |
                     +------+------+
                     |   GCP LB    |
                     +------+------+
                            |
               +------------+------------+
               |            |            |
         HTTPS Ingress   SSH Proxy    API Server
         (Traefik/Caddy) (sshpiper)  (Rust/Axum, Deployment)
               |            |            |
               +------------+------------+
                            |
                     K3s Control Plane
                     (K3s server node(s) + SQLite/PostgreSQL)
                            |
              +-------------+-------------+
              |             |             |
         K3s Agent Node  K3s Agent Node  K3s Agent Node
         (GCE VM)        (GCE VM)        (GCE VM)
              |
    +---------+---------+
    |         |         |
  Agent Pod  Agent Pod  Agent Pod
  (PVC)      (PVC)      (PVC)
    |
  [main container: Sysbox runtime]
  [sub-agents: Docker-in-Docker via Sysbox]
```

**How K3s maps to the agent model:**
- Each **agent** = 1 K8s Pod (Sysbox runtime, with Docker available inside for sub-agents)
- Each **agent's persistent filesystem** = 1 PersistentVolumeClaim (local-path or TopoLVM)
- Each **sub-agent** = Docker container spawned inside the agent pod (Docker-in-Docker via Sysbox `runtimeClassName: sysbox`)
- **Agent-to-agent isolation** = NetworkPolicy (deny all inter-pod traffic) + separate PID/mount/network namespaces (default in K8s)
- **Scheduling** = K8s scheduler with resource requests/limits (1 vCPU, 4Gi RAM)
- **Auto-restart** = `restartPolicy: Always` + K8s CrashLoopBackOff detection
- **Health checks** = liveness/readiness probes per container

---

## Priority Tiers

### P0 - MVP (~172 tests)

**Phase 1: K3s Cluster + Agent Lifecycle**
1. K3s cluster on GCE VMs (1 server + N agent nodes) -- VMs are the only GCP dependency
2. K8s Operator (Rust, using kube-rs) that manages Agent CRD
3. REST API (Rust/Axum) with SSE streaming, translates to K8s API calls
4. Agent CRD -> Pod + PVC + NetworkPolicy + Service
5. Sysbox RuntimeClass for agent pods (enables Docker-in-Docker for sub-agents)
6. Resource limits: 1 vCPU, 4Gi RAM, 10Gi PVC per agent

**Phase 2: Connectivity**
7. SSH proxy (sshpiper) routing `{name}@ssh.agents.near.ai` to agent pods
8. HTTPS ingress with wildcard cert (`{name}.agents.near.ai`)
9. Webhook delivery via same HTTPS path

**Phase 3: Reliability**
10. Graceful shutdown via `terminationGracePeriodSeconds` + preStop hooks
11. Backup controller: snapshot PVCs -> compress -> encrypt -> MinIO
12. Restore: download from MinIO -> create PVC -> create pod
13. Prometheus + Grafana (kube-prometheus-stack helm chart)
14. Structured logging via Loki + Promtail
15. Alert rules (Alertmanager)

### P1 - Scale & Hardening (~40 tests)

**Phase 4: Capacity**
16. Manual node addition (add K3s agent nodes as needed)
17. Capacity dashboard (Grafana + kube-state-metrics)
18. Deploy queue (API server queues when no capacity, watches for space)

**Phase 5: HA**
19. K3s HA: 3 server nodes with embedded etcd
20. API server: 2+ replicas as K8s Deployment
21. Node failure: K8s marks pods as evicted -> operator restores from backup
22. Data plane independence: pods run even if API server is down (test 10.9)

**Phase 6: Stress**
23. Density testing (MAX_AGENTS_PER_HOST)
24. Concurrent deploy/delete under load
25. I/O + network saturation

### P2 - Add-on (~78 tests)
26. Hibernation (VolumeSnapshot + delete pod + wake on request)
27. Live migration (`kubectl drain` + PV reattach)
28. Scale-down & consolidation
29. Tier system (labels + priority classes)
30. Auto-deletion (TTL controller for idle agents)
31. Billing endpoints

---

## Component Details

### 1. Agent CRD + Operator

**Custom Resource Definition:**
```yaml
apiVersion: agents.near.ai/v1
kind: Agent
metadata:
  name: my-agent
spec:
  image: ghcr.io/nearai/ironclaw:latest
  resources:
    cpu: "1"
    memory: "4Gi"
    disk: "10Gi"
  state: running  # desired: running | stopped
status:
  phase: Running  # actual: Creating | Running | Stopped | Failed | CrashLoopBackOff
  hostNode: gce-node-03
  podIP: 10.42.1.15
  sshPort: 30122
  restartCount: 0
  lastBackup: "2026-03-05T12:00:00Z"
  conditions: [...]
```

**Operator reconciliation (Rust, kube-rs):**
1. Watch Agent CRD changes
2. For each Agent, ensure:
   - Pod exists with correct image, resources, `runtimeClassName: sysbox`, capability restrictions
   - PVC exists (local-path, 10Gi, ReadWriteOnce)
   - NetworkPolicy exists (deny ingress from other agent pods, allow from ingress controller + SSH proxy)
   - Service exists (ClusterIP for internal routing, or NodePort for SSH)
3. Update Agent status from Pod status
4. Sub-agents are managed by the agent process itself via Docker-in-Docker (enabled by Sysbox runtime)
5. Log lifecycle events to PostgreSQL (or K8s Events + external sink)

### 2. Isolation Model

**Sysbox-based isolation:**
- All agent pods run under the **Sysbox runtime** (`runtimeClassName: sysbox`), which provides enhanced container isolation and enables Docker-in-Docker without privileged mode
- **Sub-agents** are spawned as Docker containers *inside* the agent pod (DinD), managed by the agent process itself
- Container hardening (securityContext) is applied on top of Sysbox

**Agent pod spec (Sysbox + hardening):**
```yaml
spec:
  runtimeClassName: sysbox        # Sysbox runtime for DinD support
  containers:
    - name: agent
      securityContext:
        capabilities:
          drop: ["ALL"]           # test 9.9, 9.11
          add: ["NET_BIND_SERVICE"]
      resources:
        requests:
          cpu: "1"
          memory: "4Gi"
        limits:
          cpu: "1"
          memory: "4Gi"
```

**Sub-agent spawning (Docker-in-Docker):**
```bash
# Inside the agent container, the agent process can run:
docker run --rm untrusted-tool:latest <command>
```

Sysbox intercepts Docker's syscalls and runs the inner container in its own user namespace — no `--privileged` flag needed. The inner container is isolated from the host kernel via Sysbox's syscall interception layer.

**NetworkPolicy (agent-to-agent isolation):**
```yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: agent-{name}-isolation
spec:
  podSelector:
    matchLabels:
      agents.near.ai/name: {name}
  policyTypes: [Ingress, Egress]
  ingress:
    - from:
      - podSelector:
          matchLabels:
            app: ingress-controller  # HTTPS gateway
      - podSelector:
          matchLabels:
            app: ssh-proxy           # SSH access
  egress:
    - to: []  # Allow all egress (agent needs internet for LLM APIs)
      ports:
        - port: 443
        - port: 80
```

This ensures:
- Agent A cannot see/reach Agent B (test 9.4, 9.5)
- Agent cannot access host management services (test 9.10)
- Only ingress controller and SSH proxy can reach agents

### 3. API Server

**Rust/Axum service deployed as K8s Deployment (2+ replicas):**

```
Lifecycle:
  POST   /instances                      SSE stream, creates Agent CRD
  GET    /instances                      List Agent CRDs (filter by status)
  GET    /instances/{name}               Get Agent CRD status
  POST   /instances/{name}/start         Patch Agent spec.state=running
  POST   /instances/{name}/stop          Patch Agent spec.state=stopped
  POST   /instances/{name}/restart       Delete pod (operator recreates)
  DELETE /instances/{name}               Delete Agent CRD (cascade)

Backup:
  POST   /instances/{name}/backup        Trigger VolumeSnapshot, SSE stream
  GET    /instances/{name}/backups       List VolumeSnapshots for agent
  GET    /instances/{name}/backups/{id}  Presigned MinIO download URL

Health:
  GET    /health                         API liveness
```

**SSE implementation:**
- API server creates/patches the Agent CRD, then watches the Agent status via K8s watch API
- Status changes (Creating -> Running, error conditions) are streamed as SSE events
- Client disconnect does NOT cancel the operation (K8s operations are declarative, test 11.23)
- Final SSE event: `{"event": "complete", "data": {"status": "running"}}` or `{"event": "error", ...}`

**Behavior:**
- Duplicate name -> 409 Conflict
- Stop on stopped -> 200 no-op
- Start on running -> 200 no-op
- Auth: API key or JWT (K8s ServiceAccount token for internal, external auth middleware for user-facing)
- Error format: `{ "error": { "code": "...", "message": "...", "request_id": "..." } }`

### 4. Networking & Connectivity

**SSH Access:**
```
User -> GCP LB (TCP :22) -> sshpiper (K8s Deployment)
     -> {agent-name}@ssh.agents.near.ai
     -> Forward to agent pod IP:22 (or NodePort Service)
```

- Each agent pod runs sshd
- sshpiper watches Agent CRDs for routing updates (pod IP changes on restart/migration)
- Auth: per-agent SSH key pair. Generated by operator at creation, stored in K8s Secret, public key returned via API.
- SSH session lands inside the agent container namespace (test 12.5)
- Multiple concurrent sessions supported (test 12.10)

**HTTPS Gateway:**
```
User -> GCP LB (HTTPS :443) -> Ingress Controller (Traefik, bundled with K3s)
     -> Route by Host header ({name}.agents.near.ai)
     -> Forward to agent pod ClusterIP Service
```

- Wildcard TLS: cert-manager + Let's Encrypt DNS-01 challenge for `*.agents.near.ai`
- Each agent gets a K8s Service + Ingress resource (created by operator)
- WebSocket upgrade supported (Traefik supports this natively)
- Nonexistent agent -> 404; stopped agent -> 503
- After migration/restore: Service selector auto-updates when new pod matches labels

**Webhooks:** Same HTTPS path. External services POST to `{name}.agents.near.ai/...`. Stopped agent -> 503.

### 5. Backup & Restore

**On-demand backup (product-triggered):**
1. `POST /instances/{name}/backup`
2. Operator pauses agent (optional: exec `fsfreeze` for app-consistent snapshot, test 4.7)
3. Backup job: tar + compress (zstd) the PVC data from the node
4. Encrypt (AES-256-GCM, key managed by operator secret or HashiCorp Vault)
5. Upload to MinIO (S3-compatible, self-hosted): `s3://agent-backups/{agent-id}/{timestamp}.tar.zst.enc`
6. Register backup metadata in Agent CRD status or dedicated ConfigMap
7. SSE stream reports progress

**Scheduled backup (infra safety net):**
- CronJob or backup controller runs on schedule (every BACKUP_RPO_HOURS)
- Same flow as on-demand
- Freshness alert if any agent exceeds RPO without successful backup

**Restore:**
1. Download encrypted backup from MinIO
2. Verify SHA-256 checksum (mismatch -> try next-oldest, test 4.11)
3. Decrypt + extract to new PVC
4. Create Agent CRD pointing to restored PVC
5. K8s scheduler places pod on node with capacity
6. Ingress + SSH routing auto-updates via label selectors

**Retention:** Keep last 10 backups per agent (MVP). MinIO lifecycle policy for cleanup.

### 6. Monitoring & Alerting

**Stack: kube-prometheus-stack (Helm chart) -- gives everything out of the box:**
- Prometheus (scrapes kubelet/cAdvisor for per-pod metrics)
- Grafana (pre-built K8s dashboards)
- Alertmanager (routes to PagerDuty/Slack)
- kube-state-metrics (pod state, deployment health)
- node-exporter (host-level CPU/memory/disk/network)

**Per-agent metrics (auto-discovered via pod labels):**
- CPU/memory/disk/network per pod (cAdvisor, built into kubelet)
- Container restart count (kube-state-metrics)
- Pod phase transitions
- Custom metrics via operator (backup freshness, sub-agent count)

**Dashboards:**
1. Fleet overview: agents by state, total capacity, available slots
2. Per-node: agent count, resource utilization
3. Per-agent: real-time resource usage, lifecycle events, logs

**Alert rules:**

| Alert | Condition | Severity |
|-------|-----------|----------|
| HostCPUHigh | Node CPU > 85% for 5min | warning |
| HostMemHigh | Node memory > 90% | critical |
| HostDiskHigh | Node disk > 85% | critical |
| AgentCrashLoop | Pod in CrashLoopBackOff | critical |
| AgentOOMKill | OOMKilled reason on container | warning |
| BackupStale | No VolumeSnapshot > RPO hours | warning |
| AllNodesAtCapacity | 0 allocatable slots | critical |
| NodeUnreachable | Node NotReady > 30s | critical |
| AgentHeartbeatMissing | Liveness probe failing > 60s | warning |

**Logging:**
- Promtail DaemonSet -> Loki (self-hosted)
- Pod logs auto-labeled with agent name, node, namespace
- Queryable by agent name across node changes (pod labels are stable)

### 7. HA & Disaster Recovery

**K3s HA (P1):**
- 3 K3s server nodes with embedded etcd
- API server replicas: 2+ behind K8s Service
- Key principle: **data plane independent of control plane** -- kubelet keeps pods running even if K3s server is unreachable (test 10.9)

**Node failure recovery:**
1. K8s marks node as NotReady (within 40s default, tunable)
2. After `pod-eviction-timeout` (5min default, tunable to 30s): pods evicted
3. Operator detects evicted Agent pods -> restores from backup on healthy node
4. Since PVCs are local storage (not network-attached), disk data is lost with the node. Operator restores from latest MinIO backup onto a new node.
5. Ingress + SSH routing auto-updates

**API server recovery:**
- Stateless Deployment with replicas. K8s restarts crashed pods.
- On restart, reads Agent CRDs (source of truth in etcd). No separate reconciliation needed.

---

## GCloud -> Bare Metal Migration Path

**No GCP-specific services are used** (only GCE VM instances as hosts). All components are portable:

| Component | GCloud (GCE VMs) | Bare Metal |
|-----------|-------------------|------------|
| K3s nodes | GCE instances | Physical servers |
| PV driver | local-path-provisioner or TopoLVM | Same (local storage) |
| Backup storage | MinIO (self-hosted on a VM) | MinIO (self-hosted) |
| K3s HA backend | Embedded etcd | Embedded etcd |
| Load balancer | MetalLB or external LB VM | MetalLB + HAProxy |
| DNS | Cloudflare (external) | Cloudflare (external) |
| TLS certs | cert-manager + Let's Encrypt | Same |
| Node provisioning | Terraform + K3s agent join | Ansible + K3s agent join |

**What changes on migration:** Only Terraform -> Ansible for node provisioning. Everything else (K3s, operator, API server, CRDs, Ingress, NetworkPolicy, Prometheus, Loki, sshpiper, MinIO, backup controller) is identical.

---

## Open Questions for Product Team

1. **Heartbeat mechanism** (test 1.11, 8.9): How does IronClaw/OpenClaw signal liveness? HTTP endpoint? K8s liveness probe needs a target.
2. **Credential storage** (test 1.16): Local filesystem in PVC? K8s Secret mounted as volume? External vault?
3. **Sub-agent management** (test 1.10, 1.24): Agents spawn sub-agents via Docker-in-Docker (Sysbox). Remaining question: how does the platform track/limit sub-agent resource usage?
4. **Traffic routing** (test 1.23): Does agent poll for messages, or does gateway push inbound requests?
5. **In-flight message handling on stop** (test 2.12): Complete current request? Drop? Return error?
6. **Agent image versioning** (test 11.10): How are images tagged and upgraded? Rolling update or hard restart?
7. **SSH auth model** (test 12.8): Per-agent keys provisioned by infra? Or user brings own key?

---

## Test Coverage Summary

| Category | P0 MVP | P1 Hardening | P2 Add-on |
|----------|--------|--------------|-----------|
| 1. Agent Deployment (38) | 25 | 10 | 3 |
| 2. Agent Deletion (21) | 12 | 2 | 7 |
| 3. Suspend/Resume (13) | 0 | 0 | 13 |
| 4. Backup & Restore (19) | 13 | 2 | 4 |
| 5. Scale-Up (16) | 9 | 3 | 4 |
| 6. Scale-Down (6) | 0 | 0 | 6 |
| 7. Migration (16) | 0 | 0 | 16 |
| 8. Monitoring (34) | 21 | 3 | 10 |
| 9. Security (29) | 18 | 9 | 2 |
| 10. HA & DR (27) | 19 | 4 | 4 |
| 11. API Contract (45) | 35 | 4 | 6 |
| 12. Connectivity (26) | 20 | 3 | 3 |
| **Total** | **172** | **40** | **78** |

MVP covers 172/290 tests (59%). P0+P1 covers 212 (73%). Remaining 78 are P2 add-on features.

---

## Technology Summary

| Component | Technology | Why |
|-----------|-----------|-----|
| Orchestration | K3s | Lightweight K8s; scheduling, health checks, restart, PVCs, NetworkPolicy for free; portable to bare metal |
| Operator | Rust + kube-rs | Type-safe, performant K8s operator; manages Agent CRD lifecycle |
| API Server | Rust + Axum | High-performance async HTTP; SSE via K8s watch API |
| Agent isolation | runc + seccomp + capabilities + NetworkPolicy | Standard container hardening; sufficient for trusted agent images |
| Agent runtime + sub-agent isolation | Sysbox (RuntimeClass) | Enables Docker-in-Docker without privileged mode; agents spawn sub-agents as inner Docker containers |
| Persistent storage | local-path-provisioner or TopoLVM | Local disk PVCs; same on GCE VMs and bare metal |
| Backup storage | MinIO (self-hosted) | S3-compatible; presigned URLs; encryption at rest; portable |
| HTTPS gateway | Traefik (K3s bundled) or Caddy | Wildcard TLS; subdomain routing; WebSocket support |
| SSH proxy | sshpiper | Routes by username; lightweight; watches CRDs for updates |
| Monitoring | kube-prometheus-stack | Prometheus + Grafana + Alertmanager + kube-state-metrics + node-exporter; one Helm install |
| Logging | Loki + Promtail | Lightweight; label-based queries; auto-discovers pod logs |
| TLS certs | cert-manager + Let's Encrypt | Automated wildcard cert via DNS-01 |
