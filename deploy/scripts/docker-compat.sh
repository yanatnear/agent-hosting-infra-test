#!/bin/bash
# Docker CLI compatibility wrapper for K3s.
# Translates docker exec/inspect/ps to kubectl equivalents.
# Install: sudo cp deploy/scripts/docker-compat.sh /usr/local/bin/docker && sudo chmod +x /usr/local/bin/docker

NAMESPACE="${AGENT_NAMESPACE:-agents}"

# Resolve an instance name to a K8s pod name.
# Strips -worker suffix and prepends agent- (unless already prefixed).
_pod_name() {
  local name="${1%-worker}"
  if [[ "$name" == agent-* ]]; then
    echo "$name"
  else
    echo "agent-${name}"
  fi
}

case "$1" in
  exec)
    CONTAINER="$2"
    shift 2
    POD_NAME=$(_pod_name "$CONTAINER")
    kubectl exec -n "$NAMESPACE" "$POD_NAME" -c agent -- "$@"
    ;;
  inspect)
    shift
    FORMAT=""
    CONTAINER=""
    while [ $# -gt 0 ]; do
      case "$1" in
        --format) FORMAT="$2"; shift 2 ;;
        *) CONTAINER="$1"; shift ;;
      esac
    done
    POD_NAME=$(_pod_name "$CONTAINER")
    if echo "$FORMAT" | grep -q "State.Status"; then
      PHASE=$(kubectl get pod -n "$NAMESPACE" "$POD_NAME" -o jsonpath='{.status.phase}' 2>/dev/null)
      echo "${PHASE,,}"  # lowercase: Running -> running
    else
      kubectl get pod -n "$NAMESPACE" "$POD_NAME" -o json
    fi
    ;;
  ps)
    shift  # remove "ps"
    QUIET=false
    INSTANCE_NAME=""
    while [ $# -gt 0 ]; do
      case "$1" in
        -q) QUIET=true; shift ;;
        --filter)
          if [[ "$2" == *crabshack_instance=* ]]; then
            INSTANCE_NAME="${2##*crabshack_instance=}"
          fi
          shift 2 ;;
        *) shift ;;
      esac
    done
    if [ -n "$INSTANCE_NAME" ]; then
      POD_NAME="agent-${INSTANCE_NAME}"
      if [ "$QUIET" = true ]; then
        # -q: output just the pod name (like docker container ID)
        kubectl get pod -n "$NAMESPACE" "$POD_NAME" --no-headers -o custom-columns='NAME:.metadata.name' 2>/dev/null
      else
        kubectl get pod -n "$NAMESPACE" "$POD_NAME" --no-headers -o custom-columns='NAME:.metadata.name,STATUS:.status.phase' 2>/dev/null
      fi
    else
      if [ "$QUIET" = true ]; then
        kubectl get pods -n "$NAMESPACE" -l app=agent --no-headers -o custom-columns='NAME:.metadata.name' 2>/dev/null
      else
        kubectl get pods -n "$NAMESPACE" --no-headers -o custom-columns='NAME:.metadata.name,STATUS:.status.phase' 2>/dev/null
      fi
    fi
    ;;
  *)
    echo "Unsupported docker command: $1" >&2
    exit 1
    ;;
esac
