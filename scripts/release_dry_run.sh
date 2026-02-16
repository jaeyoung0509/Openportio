#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

OUTPUT_DIR="${ROOT_DIR}/target/release"
LOG_DIR="${OUTPUT_DIR}/dry-run-logs"
SUMMARY_FILE="${OPENPORTIO_RELEASE_DRY_RUN_SUMMARY:-${OUTPUT_DIR}/dry-run-summary.md}"
METADATA_SUMMARY_FILE="${OPENPORTIO_RELEASE_METADATA_SUMMARY:-${OUTPUT_DIR}/metadata-summary.md}"

mkdir -p "$LOG_DIR"

./scripts/check_release_metadata.sh

RPC_PATCH_ARGS=(
  --config "patch.crates-io.openportio-core.path=\"crates/openportio-core\""
)

SERVER_PATCH_ARGS=(
  --config "patch.crates-io.openportio-core.path=\"crates/openportio-core\""
  --config "patch.crates-io.openportio-macros.path=\"crates/openportio-macros\""
  --config "patch.crates-io.openportio-rpc.path=\"crates/openportio-rpc\""
)

workspace_version="$(
  awk '
    $0 ~ /^\[workspace\.package\]/ { in_workspace_package = 1; next }
    /^\[/ && in_workspace_package { in_workspace_package = 0 }
    in_workspace_package && $1 == "version" {
      gsub(/"/, "", $3)
      print $3
      exit
    }
  ' Cargo.toml
)"

release_tag="${OPENPORTIO_RELEASE_TAG:-}"
dry_run_rows=()
dry_run_failed=0

run_dry_run() {
  local step="$1"
  local crate="$2"
  shift 2
  local log_file="${LOG_DIR}/${step}-${crate}.log"

  echo "[${step}/4] cargo publish --dry-run -p ${crate} --allow-dirty"
  if cargo publish --dry-run -p "${crate}" --allow-dirty "$@" 2>&1 | tee "${log_file}"; then
    dry_run_rows+=("| ${crate} | pass | ${log_file#${ROOT_DIR}/} |")
  else
    dry_run_rows+=("| ${crate} | fail | ${log_file#${ROOT_DIR}/} |")
    dry_run_failed=1
  fi
}

run_dry_run 1 openportio-core

run_dry_run 2 openportio-macros

run_dry_run 3 openportio-rpc "${RPC_PATCH_ARGS[@]}"

run_dry_run 4 openportio-server "${SERVER_PATCH_ARGS[@]}"

{
  echo "# Release Dry-Run Summary"
  echo
  echo "- Workspace version: \`${workspace_version:-unresolved}\`"
  if [[ -n "$release_tag" ]]; then
    echo "- Release tag input: \`${release_tag}\`"
  else
    echo "- Release tag input: _(not provided)_"
  fi
  echo "- Metadata summary: \`${METADATA_SUMMARY_FILE#${ROOT_DIR}/}\`"
  echo
  echo "| crate | result | log |"
  echo "|---|---|---|"
  for row in "${dry_run_rows[@]}"; do
    echo "$row"
  done
  echo
  if [[ "$dry_run_failed" -eq 0 ]]; then
    echo "[PASS] release dry-run checks succeeded."
    echo
    echo "Note: local crates.io patch overrides are used to validate publish order before first registry index propagation."
  else
    echo "[FAIL] release dry-run checks failed. Check logs above."
  fi
} > "$SUMMARY_FILE"

echo "Release dry-run summary: ${SUMMARY_FILE}"

if [[ "$dry_run_failed" -ne 0 ]]; then
  exit 1
fi
