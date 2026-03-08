#!/usr/bin/env bash
# Upload test results to TestOps in JUnit XML format.
#
# Usage:
#   ./scripts/testops-upload.sh                    # Run integration tests + upload
#   ./scripts/testops-upload.sh --unit             # Run unit tests + upload
#   ./scripts/testops-upload.sh --dry-run          # Generate XML without uploading
#   ./scripts/testops-upload.sh --from-file OUT    # Parse existing cargo output file
#
# Environment:
#   TESTOPS_URL        TestOps base URL (default: https://136.112.126.95)
#   TESTOPS_PROJECT_ID Project ID (default: 10)
#   RUN_NAME           Custom run name (default: auto-generated)

set -eo pipefail

TESTOPS_URL="${TESTOPS_URL:-https://136.112.126.95}"
TESTOPS_PROJECT_ID="${TESTOPS_PROJECT_ID:-10}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TESTS_SRC="$PROJECT_ROOT/tests/src"

DRY_RUN=false
UNIT_MODE=false
FROM_FILE=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --dry-run)  DRY_RUN=true; shift ;;
        --unit)     UNIT_MODE=true; shift ;;
        --from-file) FROM_FILE="$2"; shift 2 ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

# ─── Step 1: Extract @testops mapping from source files ─────────────────────
# Builds: fn_name -> "classname|testops_name"
declare -A TESTOPS_MAP

extract_mappings() {
    local current_testops=""
    local current_file="$1"

    while IFS= read -r line; do
        # Look for @testops annotation
        if [[ "$line" =~ @testops[[:space:]]+(.*) ]]; then
            current_testops="${BASH_REMATCH[1]}"
            continue
        fi

        # Look for fn test_ declaration following an @testops
        if [[ -n "$current_testops" && "$line" =~ fn[[:space:]]+(test_[a-zA-Z0-9_]+) ]]; then
            local fn_name="${BASH_REMATCH[1]}"
            local classname
            classname=$(get_classname "$current_testops")
            TESTOPS_MAP["$fn_name"]="$classname|$current_testops"
            current_testops=""
            continue
        fi

        # Reset if we hit a non-comment, non-attribute line without finding fn
        if [[ -n "$current_testops" && ! "$line" =~ ^[[:space:]]*(///|#\[) ]]; then
            current_testops=""
        fi
    done < "$current_file"
}

# Map TestOps case number to Feature.Story classname
get_classname() {
    local testops_name="$1"
    local num="${testops_name%% *}"  # e.g. "1.1", "11.31"
    local feature_num="${num%%.*}"   # e.g. "1", "11"
    local case_num="${num#*.}"       # e.g. "1", "31"

    case "$feature_num" in
        1)
            case "$case_num" in
                1|2|3|4|5) echo "Agent Deployment.Smoke" ;;
                *)         echo "Agent Deployment.Essential" ;;
            esac
            ;;
        2)
            case "$case_num" in
                1|2)  echo "Agent Deletion.Smoke" ;;
                *)    echo "Agent Deletion.Essential" ;;
            esac
            ;;
        8)
            case "$case_num" in
                1|2|3) echo "Monitoring & Alerting.Smoke" ;;
                *)     echo "Monitoring & Alerting.Essential" ;;
            esac
            ;;
        9)
            case "$case_num" in
                1|2|3) echo "Security & Isolation.Smoke" ;;
                *)     echo "Security & Isolation.Essential" ;;
            esac
            ;;
        11)
            case "$case_num" in
                1|2|3|4)     echo "API Contract.Smoke" ;;
                5|6|7|8|9)   echo "API Contract.Essential — Lifecycle Endpoints" ;;
                29|30|31)    echo "API Contract.Essential — API General" ;;
                32|33|34|35|36|37|38|39) echo "API Contract.Essential — Edge Cases" ;;
                *)           echo "API Contract.Stress" ;;
            esac
            ;;
        12)
            echo "Connectivity.Smoke"
            ;;
        *)
            echo "Unknown.Unknown"
            ;;
    esac
}

# Extract from all test source files
for src_file in "$TESTS_SRC"/*.rs; do
    [[ "$(basename "$src_file")" == "lib.rs" || "$(basename "$src_file")" == "helpers.rs" ]] && continue
    extract_mappings "$src_file"
done

echo "Extracted ${#TESTOPS_MAP[@]} test→TestOps mappings"

# ─── Step 2: Run tests or read from file ─────────────────────────────────────
TMPFILE=$(mktemp /tmp/testops-output.XXXXXX)
trap 'rm -f "$TMPFILE"' EXIT

if [[ -n "$FROM_FILE" ]]; then
    cp "$FROM_FILE" "$TMPFILE"
    echo "Using test output from: $FROM_FILE"
elif $UNIT_MODE; then
    echo "Running unit tests..."
    cargo test --workspace --lib 2>&1 | tee "$TMPFILE" || true
else
    echo "Running integration tests..."
    cargo test -p agent-tests 2>&1 | tee "$TMPFILE" || true
fi

# ─── Step 3: Parse test results ─────────────────────────────────────────────
declare -A RESULTS  # fn_name -> "passed|failed|skipped"

while IFS= read -r line; do
    # Match: "test some::path::test_name ... ok"
    if [[ "$line" =~ test[[:space:]]+(([a-zA-Z0-9_]+::)*(test_[a-zA-Z0-9_]+))[[:space:]]+\.\.\.[[:space:]]+(ok|FAILED|ignored) ]]; then
        local_fn="${BASH_REMATCH[3]}"
        result="${BASH_REMATCH[4]}"
        case "$result" in
            ok)      RESULTS["$local_fn"]="passed" ;;
            FAILED)  RESULTS["$local_fn"]="failed" ;;
            ignored) RESULTS["$local_fn"]="skipped" ;;
        esac
    fi
done < "$TMPFILE"

echo "Parsed ${#RESULTS[@]} test results"

# ─── Step 4: Generate JUnit XML ─────────────────────────────────────────────
generate_xml() {
    local total=0 failures=0 skipped=0
    local testcases=""

    for fn_name in "${!TESTOPS_MAP[@]}"; do
        local mapping="${TESTOPS_MAP[$fn_name]}"
        local classname="${mapping%%|*}"
        local testops_name="${mapping#*|}"
        local status="${RESULTS[$fn_name]:-skipped}"

        total=$((total + 1))

        local tc="    <testcase classname=\"$classname\" name=\"$testops_name\" time=\"0.001\">"
        case "$status" in
            failed)
                failures=$((failures + 1))
                tc="$tc
      <failure message=\"Test $fn_name failed\">See cargo test output for details</failure>"
                ;;
            skipped)
                skipped=$((skipped + 1))
                tc="$tc
      <skipped/>"
                ;;
        esac
        tc="$tc
    </testcase>"
        testcases="$testcases
$tc"
    done

    cat <<XMLEOF
<?xml version="1.0" encoding="UTF-8"?>
<testsuites>
  <testsuite name="agent-tests" tests="$total" failures="$failures" skipped="$skipped">
$testcases
  </testsuite>
</testsuites>
XMLEOF
}

XML=$(generate_xml)

# Count from the mapping
mapped_count=${#TESTOPS_MAP[@]}
echo ""
echo "Generated JUnit XML with $mapped_count mapped test cases"

if $DRY_RUN; then
    echo ""
    echo "--- DRY RUN: Generated XML ---"
    echo "$XML"
    exit 0
fi

# ─── Step 5: Upload to TestOps ───────────────────────────────────────────────
if [[ -z "${RUN_NAME:-}" ]]; then
    if $UNIT_MODE; then
        RUN_NAME="Unit Tests — cargo test ($(date +%Y-%m-%d))"
    else
        RUN_NAME="Integration Tests — cargo test ($(date +%Y-%m-%d))"
    fi
fi

echo "Uploading to $TESTOPS_URL as '$RUN_NAME'..."

JSON_XML=$(echo "$XML" | python3 -c "import sys,json; print(json.dumps(sys.stdin.read()))")
UPLOAD_BODY=$(cat <<JSONEOF
{"project_id": $TESTOPS_PROJECT_ID, "name": "$RUN_NAME", "xml": $JSON_XML}
JSONEOF
)

RESPONSE=$(curl -sk -X POST "$TESTOPS_URL/api/runs/upload" \
    -H 'Content-Type: application/json' \
    -d "$UPLOAD_BODY" \
    -w "\n%{http_code}" 2>&1)

HTTP_CODE=$(echo "$RESPONSE" | tail -1)
BODY=$(echo "$RESPONSE" | head -n -1)

if [[ "$HTTP_CODE" == "200" || "$HTTP_CODE" == "201" ]]; then
    echo "Upload successful!"
    echo "$BODY" | python3 -m json.tool 2>/dev/null || echo "$BODY"
else
    echo "Upload failed (HTTP $HTTP_CODE):"
    echo "$BODY"
    exit 1
fi
