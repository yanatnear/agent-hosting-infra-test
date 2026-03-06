#!/usr/bin/env bash
# Full deployment script for the NEAR agent hosting platform.
# Applies all manifests and installs Helm charts in the correct order.
set -euo pipefail

# ---------- helpers ----------
info()  { echo "[INFO]  $*"; }
warn()  { echo "[WARN]  $*"; }
error() { echo "[ERROR] $*" >&2; exit 1; }

# ---------- configuration ----------
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
MANIFESTS="${REPO_ROOT}/deploy/manifests"
MONITORING="${MANIFESTS}/monitoring"

KUBECONFIG="${KUBECONFIG:-/etc/rancher/k3s/k3s.yaml}"
export KUBECONFIG

# ---------- pre-flight ----------
if ! command -v kubectl &>/dev/null; then
  error "kubectl is not installed or not in PATH."
fi

if ! kubectl cluster-info &>/dev/null; then
  error "Cannot connect to the Kubernetes cluster. Check KUBECONFIG."
fi

if ! command -v helm &>/dev/null; then
  warn "helm is not installed. Monitoring stack will be skipped."
  HELM_AVAILABLE=false
else
  HELM_AVAILABLE=true
fi

# ---------- 1. Namespace ----------
info "Applying namespace..."
kubectl apply -f "${MANIFESTS}/namespace.yaml"

# ---------- 2. CRD ----------
info "Applying Agent CRD..."
kubectl apply -f "${MANIFESTS}/agent-crd.yaml"

# Wait for CRD to be established
info "Waiting for Agent CRD to be established..."
kubectl wait --for=condition=established --timeout=30s crd/agents.agents.near.ai

# ---------- 3. RuntimeClass ----------
info "Applying Sysbox RuntimeClass..."
kubectl apply -f "${MANIFESTS}/sysbox-runtimeclass.yaml"

# ---------- 4. MinIO ----------
info "Applying MinIO..."
kubectl apply -f "${MANIFESTS}/minio.yaml"

# ---------- 5. Operator ----------
info "Applying agent operator..."
kubectl apply -f "${MANIFESTS}/operator.yaml"

# ---------- 6. API server ----------
info "Applying API server..."
kubectl apply -f "${MANIFESTS}/api-server.yaml"

# ---------- 7. SSH proxy ----------
info "Applying SSH proxy..."
kubectl apply -f "${MANIFESTS}/ssh-proxy.yaml"

# ---------- 8. Ingress (Traefik) ----------
info "Applying Traefik ingress..."
kubectl apply -f "${MANIFESTS}/ingress.yaml"

# ---------- 9. Monitoring (Helm) ----------
if [[ "${HELM_AVAILABLE}" == "true" ]]; then
  info "Adding Helm repositories..."
  helm repo add prometheus-community https://prometheus-community.github.io/helm-charts 2>/dev/null || true
  helm repo add grafana https://grafana.github.io/helm-charts 2>/dev/null || true
  helm repo update

  info "Installing kube-prometheus-stack..."
  helm upgrade --install monitoring prometheus-community/kube-prometheus-stack \
    --namespace monitoring --create-namespace \
    --values "${MONITORING}/kube-prometheus-values.yaml" \
    --wait --timeout 5m

  info "Installing Loki stack..."
  helm upgrade --install loki grafana/loki-stack \
    --namespace monitoring --create-namespace \
    --values "${MONITORING}/loki-values.yaml" \
    --wait --timeout 5m
else
  warn "Skipping monitoring installation (helm not available)."
fi

# ---------- 10. Wait for rollouts ----------
info "Waiting for rollouts to complete..."

kubectl -n agents rollout status statefulset/minio --timeout=120s || \
  warn "MinIO rollout not ready yet."

kubectl -n agents rollout status deployment/agent-operator --timeout=60s || \
  warn "Operator rollout not ready yet."

kubectl -n agents rollout status deployment/agent-api --timeout=60s || \
  warn "API server rollout not ready yet."

kubectl -n agents rollout status deployment/ssh-proxy --timeout=60s || \
  warn "SSH proxy rollout not ready yet."

kubectl -n traefik rollout status deployment/traefik --timeout=60s || \
  warn "Traefik rollout not ready yet."

# ---------- summary ----------
info "============================================"
info " Deployment complete"
info "============================================"
info ""
info "Namespaces:"
kubectl get ns agents traefik monitoring 2>/dev/null || true
info ""
info "Agents namespace pods:"
kubectl -n agents get pods
info ""
info "Services:"
kubectl -n agents get svc
info "============================================"
