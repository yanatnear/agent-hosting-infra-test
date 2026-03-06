#!/usr/bin/env bash
# K3s cluster bootstrap script for NEAR agent hosting platform.
# Installs K3s server, enables Sysbox RuntimeClass, and prints the join token.
set -euo pipefail

INSTALL_DIR="/var/lib/rancher/k3s"
KUBECONFIG_PATH="/etc/rancher/k3s/k3s.yaml"

# ---------- helpers ----------
info()  { echo "[INFO]  $*"; }
warn()  { echo "[WARN]  $*"; }
error() { echo "[ERROR] $*" >&2; exit 1; }

# ---------- pre-flight ----------
if [[ $EUID -ne 0 ]]; then
  error "This script must be run as root (or with sudo)."
fi

# ---------- idempotency check ----------
if systemctl is-active --quiet k3s 2>/dev/null; then
  info "K3s server is already running."
  info "Join token: $(cat /var/lib/rancher/k3s/server/node-token)"
  info "Server URL: https://$(hostname -I | awk '{print $1}'):6443"
  exit 0
fi

# ---------- install K3s server ----------
info "Installing K3s server..."
curl -sfL https://get.k3s.io | INSTALL_K3S_EXEC="server" sh -s - \
  --disable=traefik \
  --write-kubeconfig-mode=644

info "Waiting for K3s to become ready..."
export KUBECONFIG="${KUBECONFIG_PATH}"
until kubectl get nodes &>/dev/null; do
  sleep 2
done
info "K3s server is up."

# ---------- install Sysbox ----------
info "Installing Sysbox runtime..."
bash "$(dirname "$0")/../../scripts/setup-sysbox.sh"

# ---------- apply Sysbox RuntimeClass ----------
info "Applying Sysbox RuntimeClass..."
kubectl apply -f "$(dirname "$0")/../manifests/sysbox-runtimeclass.yaml"

# ---------- local-path-provisioner ----------
# K3s ships with local-path-provisioner enabled by default.
info "Verifying local-path-provisioner..."
kubectl -n kube-system rollout status deploy/local-path-provisioner --timeout=60s || \
  warn "local-path-provisioner not ready yet; it may still be starting."

# ---------- output join information ----------
JOIN_TOKEN="$(cat /var/lib/rancher/k3s/server/node-token)"
SERVER_IP="$(hostname -I | awk '{print $1}')"

info "============================================"
info " K3s server installation complete"
info "============================================"
info " Server URL : https://${SERVER_IP}:6443"
info " Join token : ${JOIN_TOKEN}"
info ""
info " To join agent nodes, run on each node:"
info "   sudo ./join.sh https://${SERVER_IP}:6443 ${JOIN_TOKEN}"
info "============================================"
