#!/usr/bin/env bash
# K3s agent node join script for NEAR agent hosting platform.
# Joins a worker node to an existing K3s cluster and installs Sysbox.
set -euo pipefail

# ---------- helpers ----------
info()  { echo "[INFO]  $*"; }
error() { echo "[ERROR] $*" >&2; exit 1; }

# ---------- arguments ----------
if [[ $# -lt 2 ]]; then
  echo "Usage: $0 <SERVER_URL> <TOKEN>"
  echo ""
  echo "  SERVER_URL  K3s server URL, e.g. https://10.0.0.1:6443"
  echo "  TOKEN       Node join token from the server"
  exit 1
fi

SERVER_URL="$1"
TOKEN="$2"

# ---------- pre-flight ----------
if [[ $EUID -ne 0 ]]; then
  error "This script must be run as root (or with sudo)."
fi

# ---------- idempotency check ----------
if systemctl is-active --quiet k3s-agent 2>/dev/null; then
  info "K3s agent is already running on this node."
  exit 0
fi

# ---------- install K3s agent ----------
info "Installing K3s agent and joining cluster at ${SERVER_URL}..."
curl -sfL https://get.k3s.io | INSTALL_K3S_EXEC="agent" sh -s - \
  --server "${SERVER_URL}" \
  --token "${TOKEN}"

info "Waiting for K3s agent to start..."
until systemctl is-active --quiet k3s-agent; do
  sleep 2
done
info "K3s agent is running."

# ---------- install Sysbox runtime ----------
info "Installing Sysbox runtime on this node..."
bash "$(dirname "$0")/../../scripts/setup-sysbox.sh"

info "============================================"
info " Agent node joined successfully"
info " Server: ${SERVER_URL}"
info "============================================"
