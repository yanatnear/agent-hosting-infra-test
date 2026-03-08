# Test Cases — Two-Week MVP

Priority levels:
- **P0**: System non-functional without this. Must pass.
- **P1**: Degraded experience. Should pass before handoff.
- **P2**: Quality improvement. Fix shortly after handoff.

## Agent Creation

| # | P | Test |
|---|---|------|
| 1 | P0 | Create agent via API — reaches running state |
| 2 | P0 | Created agent can make outbound HTTPS requests |
| 3 | P0 | Created agent has writable persistent filesystem |
| 4 | P0 | Created agent can spawn sub-agent Docker container (Sysbox DinD) |
| 5 | P1 | Create with invalid params — returns clear error |
| 6 | P1 | Create with duplicate name — returns clear error |
| 7 | P0 | Agent crashes — auto-restarts and recovers |
| 8 | P0 | Agent data persists across restart |

## Agent Stop / Start / Restart

| # | P | Test |
|---|---|------|
| 9 | P0 | Stop agent via API — process stops, resources held |
| 10 | P0 | Start stopped agent via API — resumes, data intact |
| 11 | P1 | Restart agent via API — comes back running, data intact |
| 12 | P2 | Stop already-stopped agent — defined behavior, no crash |
| 13 | P2 | Start already-running agent — defined behavior, no crash |

## Agent Deletion

| # | P | Test |
|---|---|------|
| 14 | P0 | Delete stopped agent — all resources cleaned up |
| 15 | P0 | Delete running agent — stops first, then cleans up |
| 16 | P1 | Delete nonexistent agent — returns 404 |

## Logs & Stats

| # | P | Test |
|---|---|------|
| 17 | P0 | GET /instances/{name}/logs — returns last N log lines |
| 18 | P1 | GET /instances/{name}/stats — returns CPU and memory |
| 19 | P2 | Logs/stats for nonexistent agent — returns clear error |

## API General

| # | P | Test |
|---|---|------|
| 20 | P0 | GET /instances — returns list with correct status |
| 21 | P0 | GET /instances/{name} — returns details |
| 22 | P2 | All error responses have consistent format |

## Basic Isolation (Sysbox)

| # | P | Test |
|---|---|------|
| 23 | P0 | Agent A cannot see agent B's processes |
| 24 | P0 | Agent A cannot access agent B's filesystem |
| 25 | P1 | Agent cannot access host filesystem outside its mount |
| 26 | P1 | Agent cannot access Docker daemon or host management services |

## Host Health

| # | P | Test |
|---|---|------|
| 27 | P1 | node_exporter running — host metrics in Prometheus |
| 28 | P0 | Docker log rotation configured — disk won't fill |

## SSH Access

| # | P | Test |
|---|---|------|
| 29 | P1 | SSH into agent via sshpiper works |

## Summary

| Priority | Count |
|----------|-------|
| P0 | 16 |
| P1 | 9 |
| P2 | 4 |
| Total | 29 |