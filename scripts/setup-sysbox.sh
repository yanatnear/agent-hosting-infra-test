#!/usr/bin/env bash
# Install Sysbox on a Linux node and configure containerd to use it.
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
  # ---------- install sysbox ----------
  info "Installing Sysbox for ${ARCH}..."

  # Detect distro for package installation
  if [[ -f /etc/os-release ]]; then
    . /etc/os-release
    DISTRO="${ID}"
    VERSION="${VERSION_ID}"
  else
    error "Cannot detect Linux distribution."
  fi

  case "${DISTRO}" in
    ubuntu|debian)
      # Install via .deb package from GitHub releases
      SYSBOX_VERSION="${SYSBOX_VERSION:-0.6.6}"
      DEB_FILE="sysbox-ce_${SYSBOX_VERSION}-0.linux_${ARCH}.deb"
      DOWNLOAD_URL="https://github.com/nestybox/sysbox/releases/download/v${SYSBOX_VERSION}/${DEB_FILE}"

      info "Downloading Sysbox ${SYSBOX_VERSION}..."
      curl -fsSL "${DOWNLOAD_URL}" -o "/tmp/${DEB_FILE}"

      # Install dependencies
      apt-get update -qq
      apt-get install -y -qq jq fuse

      info "Installing Sysbox package..."
      dpkg -i "/tmp/${DEB_FILE}" || apt-get install -f -y -qq
      rm -f "/tmp/${DEB_FILE}"
      ;;
    *)
      error "Unsupported distribution: ${DISTRO}. Install Sysbox manually: https://github.com/nestybox/sysbox"
      ;;
  esac

  info "Installed sysbox-runc:"
  sysbox-runc --version 2>&1 | head -1 || true
fi

# ---------- ensure sysbox services are running ----------
info "Ensuring Sysbox services are running..."
systemctl enable sysbox --now 2>/dev/null || true
systemctl enable sysbox-mgr --now 2>/dev/null || true
systemctl enable sysbox-fs --now 2>/dev/null || true

# ---------- configure containerd ----------
info "Configuring containerd to use sysbox-runc runtime..."

CONTAINERD_DIR="$(dirname "${CONTAINERD_CONFIG}")"
CONTAINERD_GENERATED="${CONTAINERD_DIR}/config.toml"
mkdir -p "${CONTAINERD_DIR}"

# The sysbox-runc snippet to append. Works with both containerd v2 and v3
# plugin paths — we detect the format from the existing config.
SYSBOX_V3='
[plugins."io.containerd.cri.v1.runtime".containerd.runtimes.sysbox-runc]
  runtime_type = "io.containerd.runc.v2"

[plugins."io.containerd.cri.v1.runtime".containerd.runtimes.sysbox-runc.options]
  BinaryName = "/usr/bin/sysbox-runc"
'

SYSBOX_V2='
[plugins."io.containerd.grpc.v1.cri".containerd.runtimes.sysbox-runc]
  runtime_type = "io.containerd.runc.v2"

[plugins."io.containerd.grpc.v1.cri".containerd.runtimes.sysbox-runc.options]
  BinaryName = "/usr/bin/sysbox-runc"
'

if [[ ! -f "${CONTAINERD_CONFIG}" ]]; then
  # No template exists. Copy K3s's generated config as the base, then append.
  if [[ -f "${CONTAINERD_GENERATED}" ]]; then
    cp "${CONTAINERD_GENERATED}" "${CONTAINERD_CONFIG}"
    info "Copied K3s generated config as template base."
  else
    warn "No existing containerd config found. Restart K3s first to generate one."
    warn "Then re-run this script."
    exit 1
  fi
fi

if grep -q "runtimes.sysbox-runc" "${CONTAINERD_CONFIG}"; then
  info "sysbox-runc runtime already configured in containerd."
else
  # Detect config version and append the matching snippet
  if grep -q "io.containerd.cri.v1" "${CONTAINERD_CONFIG}"; then
    echo "${SYSBOX_V3}" >> "${CONTAINERD_CONFIG}"
  else
    echo "${SYSBOX_V2}" >> "${CONTAINERD_CONFIG}"
  fi
  info "Appended sysbox-runc runtime to containerd config template."
fi

# ---------- restart containerd / K3s ----------
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

info "Sysbox setup complete. The 'sysbox-runc' runtime is available for containerd."
