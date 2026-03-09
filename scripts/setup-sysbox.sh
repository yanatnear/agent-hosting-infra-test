#!/usr/bin/env bash
# Install Sysbox on a Linux node and configure containerd to use it.
# Builds from source to get the latest containerd integration fix.
# This script must be run as root.
set -euo pipefail

# ---------- helpers ----------
info()  { echo "[INFO]  $*"; }
warn()  { echo "[WARN]  $*"; }
error() { echo "[ERROR] $*" >&2; exit 1; }

# ---------- pre-flight ----------
if [[ $EUID -ne 0 ]]; then
  error "This script must be run as root (or with sudo)."
fi

if [[ "$(uname -s)" != "Linux" ]]; then
  error "Sysbox is only supported on Linux."
fi

ARCH="$(uname -m)"
case "${ARCH}" in
  x86_64)  ARCH="amd64" ;;
  aarch64) ARCH="arm64" ;;
  *)       error "Unsupported architecture: ${ARCH}" ;;
esac

CONTAINERD_CONFIG="/var/lib/rancher/k3s/agent/etc/containerd/config.toml.tmpl"

# ---------- idempotency check ----------
if command -v sysbox-runc &>/dev/null; then
  CURRENT_VERSION="$(sysbox-runc --version 2>&1 | head -1 || true)"
  info "Sysbox is already installed: ${CURRENT_VERSION}"
  info "Verifying containerd configuration..."
else
  # ---------- install sysbox from source ----------
  info "Building Sysbox from source for ${ARCH}..."
  info "This provides the latest containerd integration fix (PR #106)"

  # Install dependencies
  if [[ -f /etc/os-release ]]; then
    . /etc/os-release
    DISTRO="${ID}"
  else
    error "Cannot detect Linux distribution."
  fi

  case "${DISTRO}" in
    ubuntu|debian)
      info "Installing build dependencies..."
      apt-get update -qq
      apt-get install -y -qq git make fuse3 rsync

      # Install Docker if not present (needed for build)
      if ! command -v docker &>/dev/null; then
        info "Installing Docker..."
        curl -fsSL https://get.docker.com | sh
      fi
      ;;
    *)
      error "Unsupported distribution: ${DISTRO}. Install manually: https://github.com/nestybox/sysbox"
      ;;
  esac

  # Clone and build Sysbox
  BUILD_DIR="/tmp/sysbox-build"
  rm -rf "${BUILD_DIR}"
  
  info "Cloning Sysbox repository..."
  git clone --recursive https://github.com/nestybox/sysbox.git "${BUILD_DIR}"
  cd "${BUILD_DIR}"

  # Update sysbox-runc to latest main (contains containerd fix)
  info "Updating sysbox-runc to latest main branch..."
  cd sysbox-runc
  git pull origin main
  cd ..

  # Build
  info "Building Sysbox (this takes ~5 minutes)..."
  make IMAGE_BASE_DISTRO=ubuntu IMAGE_BASE_RELEASE=jammy sysbox-static

  # Install binaries and systemd services
  info "Installing Sysbox binaries and services..."
  make install

  # Cleanup
  cd /
  rm -rf "${BUILD_DIR}"

  info "Installed sysbox-runc:"
  sysbox-runc --version 2>&1 | head -1 || true
fi

# ---------- ensure sysbox services are running ----------
info "Ensuring Sysbox services are running..."
systemctl enable sysbox --now 2>/dev/null || true
systemctl enable sysbox-mgr --now 2>/dev/null || true
systemctl enable sysbox-fs --now 2>/dev/null || true

# Give services a moment to start
sleep 2

# Verify services are active
if ! systemctl is-active --quiet sysbox-mgr; then
  error "sysbox-mgr failed to start. Check: systemctl status sysbox-mgr"
fi
if ! systemctl is-active --quiet sysbox-fs; then
  error "sysbox-fs failed to start. Check: systemctl status sysbox-fs"
fi

info "Sysbox services are running."

# ---------- configure containerd ----------
info "Configuring containerd to use sysbox-runc runtime..."

CONTAINERD_DIR="$(dirname "${CONTAINERD_CONFIG}")"
mkdir -p "${CONTAINERD_DIR}"

# K3s uses Go templates - we need to extend the base template
if [[ ! -f "${CONTAINERD_CONFIG}" ]]; then
  info "Creating containerd config template..."
  cat > "${CONTAINERD_CONFIG}" << 'TMPL'
{{ template "base" . }}

[plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.sysbox-runc]
  runtime_type = "io.containerd.runc.v2"
  [plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.sysbox-runc.options]
    SystemdCgroup = false
    BinaryName = "/usr/bin/sysbox-runc"
TMPL
  info "Created containerd config template with sysbox-runc runtime."
elif grep -q "runtimes.sysbox-runc" "${CONTAINERD_CONFIG}"; then
  info "sysbox-runc runtime already configured in containerd."
else
  # Append sysbox config to existing template
  cat >> "${CONTAINERD_CONFIG}" << 'TMPL'

[plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.sysbox-runc]
  runtime_type = "io.containerd.runc.v2"
  [plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.sysbox-runc.options]
    SystemdCgroup = false
    BinaryName = "/usr/bin/sysbox-runc"
TMPL
  info "Appended sysbox-runc runtime to containerd config template."
fi

# ---------- create RuntimeClass ----------
info "Creating Kubernetes RuntimeClass..."
if kubectl get runtimeclass sysbox-runc &>/dev/null; then
  info "RuntimeClass 'sysbox-runc' already exists."
else
  cat <<YAML | kubectl apply -f -
apiVersion: node.k8s.io/v1
kind: RuntimeClass
metadata:
  name: sysbox-runc
handler: sysbox-runc
YAML
  info "Created RuntimeClass 'sysbox-runc'."
fi

# ---------- restart K3s ----------
info "Restarting K3s to pick up containerd changes..."
if systemctl is-active --quiet k3s; then
  systemctl restart k3s
  info "K3s server restarted."
elif systemctl is-active --quiet k3s-agent; then
  systemctl restart k3s-agent
  info "K3s agent restarted."
else
  warn "Neither k3s nor k3s-agent is running. You may need to restart manually."
fi

# Wait for node to be ready
info "Waiting for node to become Ready..."
for i in $(seq 1 30); do
  if kubectl get nodes 2>/dev/null | grep -q " Ready "; then
    info "Node is Ready."
    break
  fi
  sleep 2
done

info "============================================"
info "Sysbox setup complete!"
info "The 'sysbox-runc' runtime is available."
info ""
info "To use it, set runtimeClassName: sysbox-runc"
info "and hostUsers: false in your pod specs."
info "============================================"
