#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

OUTPUT_DIR="${ROOT_DIR}/target/release"
SUMMARY_FILE="${OPENPORTIO_RELEASE_METADATA_SUMMARY:-${OUTPUT_DIR}/metadata-summary.md}"
RELEASE_TAG="${OPENPORTIO_RELEASE_TAG:-}"

mkdir -p "$(dirname "$SUMMARY_FILE")"

failures=()

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

if [[ -z "$workspace_version" ]]; then
  failures+=("failed to resolve [workspace.package].version from Cargo.toml")
fi

tag_version=""
if [[ -n "$RELEASE_TAG" ]]; then
  if [[ "$RELEASE_TAG" =~ ^v([0-9]+)\.([0-9]+)\.([0-9]+)(-rc\.[0-9]+)?$ ]]; then
    tag_version="${RELEASE_TAG#v}"
    if [[ -n "$workspace_version" && "$tag_version" != "$workspace_version" ]]; then
      failures+=(
        "release tag version (${tag_version}) does not match workspace version (${workspace_version})"
      )
    fi
  else
    failures+=("release tag '${RELEASE_TAG}' is invalid (expected vX.Y.Z or vX.Y.Z-rc.N)")
  fi
fi

if ! grep -q '^## \[Unreleased\]' CHANGELOG.md; then
  failures+=("CHANGELOG.md is missing required '## [Unreleased]' section")
fi

if [[ -n "$tag_version" ]]; then
  escaped_tag_version="$(printf '%s\n' "$tag_version" | sed 's/\./\\./g')"
  if ! grep -Eq "^## \\[${escaped_tag_version}\\]" CHANGELOG.md; then
    failures+=("CHANGELOG.md is missing section for release version '${tag_version}'")
  fi
fi

release_crates=(
  "openportio-core"
  "openportio-macros"
  "openportio-rpc"
  "openportio-server"
)

crate_rows=()
for crate in "${release_crates[@]}"; do
  manifest="crates/${crate}/Cargo.toml"

  if [[ ! -f "$manifest" ]]; then
    failures+=("missing crate manifest: ${manifest}")
    crate_rows+=("| ${crate} | missing manifest | - |")
    continue
  fi

  if grep -Eq '^version\.workspace\s*=\s*true$' "$manifest"; then
    crate_rows+=("| ${crate} | workspace | ${workspace_version:-unknown} |")
  else
    crate_version="$(
      awk '
        $0 ~ /^\[package\]/ { in_package = 1; next }
        /^\[/ && in_package { in_package = 0 }
        in_package && $1 == "version" {
          gsub(/"/, "", $3)
          print $3
          exit
        }
      ' "$manifest"
    )"
    if [[ -z "$crate_version" ]]; then
      failures+=("${manifest} missing package version")
      crate_rows+=("| ${crate} | explicit | missing |")
    elif [[ -n "$workspace_version" && "$crate_version" != "$workspace_version" ]]; then
      failures+=(
        "${manifest} package version (${crate_version}) does not match workspace version (${workspace_version})"
      )
      crate_rows+=("| ${crate} | explicit | ${crate_version} (mismatch) |")
    else
      crate_rows+=("| ${crate} | explicit | ${crate_version} |")
    fi
  fi

  while IFS= read -r line; do
    dep_name="$(printf '%s\n' "$line" | sed -E 's/.*(openportio-[a-z]+)[[:space:]]*=[[:space:]]*\{.*/\1/')"
    dep_version="$(printf '%s\n' "$line" | sed -E 's/.*version[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/')"
    if [[ -n "$workspace_version" && "$dep_version" != "$workspace_version" ]]; then
      failures+=(
        "${manifest} has ${dep_name} dependency version ${dep_version} (expected ${workspace_version})"
      )
    fi
  done < <(
    grep -E \
      'openportio-(core|macros|rpc|server)[[:space:]]*=[[:space:]]*\{[^}]*version[[:space:]]*=[[:space:]]*"[^"]+"' \
      "$manifest" || true
  )
done

{
  echo "# Release Metadata Summary"
  echo
  echo "- Workspace version: \`${workspace_version:-unresolved}\`"
  if [[ -n "$RELEASE_TAG" ]]; then
    echo "- Release tag input: \`${RELEASE_TAG}\`"
  else
    echo "- Release tag input: _(not provided)_"
  fi
  echo
  echo "## Release Crate Versions"
  echo
  echo "| crate | version source | version |"
  echo "|---|---|---|"
  for row in "${crate_rows[@]}"; do
    echo "$row"
  done
  echo
  if [[ "${#failures[@]}" -eq 0 ]]; then
    echo "## Result"
    echo
    echo "[PASS] release metadata checks passed."
  else
    echo "## Result"
    echo
    echo "[FAIL] release metadata checks failed:"
    for failure in "${failures[@]}"; do
      echo "- ${failure}"
    done
  fi
} > "$SUMMARY_FILE"

echo "Release metadata summary: ${SUMMARY_FILE}"

if [[ "${#failures[@]}" -ne 0 ]]; then
  exit 1
fi
