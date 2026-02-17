#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if [[ -n "${OPENPORTIO_CONTRACT_COMPAT_BASE_REF:-}" ]]; then
  BASE_REF="${OPENPORTIO_CONTRACT_COMPAT_BASE_REF}"
elif [[ -n "${GITHUB_BASE_REF:-}" ]]; then
  BASE_REF="origin/${GITHUB_BASE_REF}"
else
  BASE_REF="origin/develop"
fi

if ! git rev-parse --verify --quiet "$BASE_REF" >/dev/null; then
  if [[ "$BASE_REF" == origin/* ]]; then
    BRANCH="${BASE_REF#origin/}"
    git fetch --no-tags --depth=1 origin "$BRANCH"
  fi
fi

if ! git rev-parse --verify --quiet "$BASE_REF" >/dev/null; then
  echo "[FAIL] base ref not found for compatibility check: $BASE_REF" >&2
  echo "Set OPENPORTIO_CONTRACT_COMPAT_BASE_REF explicitly if needed." >&2
  exit 1
fi

./scripts/generate_contracts_bundle.sh

python3 scripts/check_contract_compatibility.py \
  --base-ref "$BASE_REF" \
  --head-bundle docs/generated/contracts-bundle.json \
  --base-bundle docs/generated/contracts-bundle.json \
  --exceptions contracts/compat_exceptions.toml

echo "Contract compatibility gate passed against ${BASE_REF}."
