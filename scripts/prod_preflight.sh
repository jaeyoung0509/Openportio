#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

OPENPORTIO_PREFLIGHT_BASE_URL="${OPENPORTIO_PREFLIGHT_BASE_URL:-http://127.0.0.1:3000}"
OPENPORTIO_PREFLIGHT_BOOT_SERVER="${OPENPORTIO_PREFLIGHT_BOOT_SERVER:-false}"
OPENPORTIO_PREFLIGHT_SECURE="${OPENPORTIO_PREFLIGHT_SECURE:-false}"
OPENPORTIO_PREFLIGHT_WAIT_SECONDS="${OPENPORTIO_PREFLIGHT_WAIT_SECONDS:-120}"

FAIL_COUNT=0
WARN_COUNT=0
SERVER_PID=""
SERVER_LOG=""

normalize_bool() {
  printf '%s' "$1" | tr '[:upper:]' '[:lower:]'
}

is_truthy() {
  case "$(normalize_bool "$1")" in
    1|true|yes|on) return 0 ;;
    *) return 1 ;;
  esac
}

ok() {
  printf '[OK] %s\n' "$1"
}

warn() {
  WARN_COUNT=$((WARN_COUNT + 1))
  printf '[WARN] %s\n' "$1"
}

fail() {
  FAIL_COUNT=$((FAIL_COUNT + 1))
  printf '[FAIL] %s\n' "$1"
}

cleanup() {
  if [[ -n "$SERVER_PID" ]]; then
    kill "$SERVER_PID" >/dev/null 2>&1 || true
    wait "$SERVER_PID" >/dev/null 2>&1 || true
  fi
  if [[ -n "$SERVER_LOG" && -f "$SERVER_LOG" ]]; then
    rm -f "$SERVER_LOG"
  fi
}

wait_for_server() {
  local base_url="$1"
  local wait_seconds="$2"
  local attempts=$((wait_seconds * 4))
  local i
  for i in $(seq 1 "$attempts"); do
    if curl -fsS "$base_url/health" >/dev/null 2>&1; then
      return 0
    fi
    if [[ -n "$SERVER_PID" ]] && ! kill -0 "$SERVER_PID" >/dev/null 2>&1; then
      return 1
    fi
    sleep 0.25
  done
  return 1
}

check_endpoint_status() {
  local path="$1"
  local expected="$2"
  local url="${OPENPORTIO_PREFLIGHT_BASE_URL}${path}"
  local body
  body="$(mktemp)"
  local code
  code="$(curl -sS -o "$body" -w '%{http_code}' "$url" || true)"
  if [[ "$code" == "$expected" ]]; then
    ok "endpoint $path returned $expected"
  else
    fail "endpoint $path returned $code (expected $expected)"
    if [[ -s "$body" ]]; then
      printf '[INFO] response body: %s\n' "$(cat "$body")"
    fi
  fi
  rm -f "$body"
}

check_endpoint_content_type() {
  local path="$1"
  local expected_prefix="$2"
  local url="${OPENPORTIO_PREFLIGHT_BASE_URL}${path}"
  local headers
  headers="$(mktemp)"
  curl -sS -D "$headers" -o /dev/null "$url" || true
  local content_type
  content_type="$(awk -F': ' 'tolower($1)=="content-type" {print $2}' "$headers" | tr -d '\r' | tail -n1)"
  if [[ "$content_type" == "$expected_prefix"* ]]; then
    ok "endpoint $path content-type is $content_type"
  else
    fail "endpoint $path content-type is '$content_type' (expected prefix '$expected_prefix')"
  fi
  rm -f "$headers"
}

check_runtime_config() {
  if is_truthy "$OPENPORTIO_PREFLIGHT_SECURE"; then
    if is_truthy "${OPENPORTIO_AUTH_ENABLED:-false}"; then
      ok "secure mode: OPENPORTIO_AUTH_ENABLED=true"
    else
      fail "secure mode requires OPENPORTIO_AUTH_ENABLED=true"
    fi

    if [[ -n "${OPENPORTIO_AUTH_JWT_SECRET:-}" ]]; then
      ok "secure mode: OPENPORTIO_AUTH_JWT_SECRET is set"
    else
      fail "secure mode requires OPENPORTIO_AUTH_JWT_SECRET"
    fi

    if [[ -n "${OPENPORTIO_AUTH_ISSUER:-}" ]]; then
      ok "secure mode: OPENPORTIO_AUTH_ISSUER is set"
    else
      warn "secure mode: OPENPORTIO_AUTH_ISSUER is not set (issuer validation disabled)"
    fi

    if [[ -n "${OPENPORTIO_AUTH_AUDIENCE:-}" ]]; then
      ok "secure mode: OPENPORTIO_AUTH_AUDIENCE is set"
    else
      warn "secure mode: OPENPORTIO_AUTH_AUDIENCE is not set (audience validation disabled)"
    fi
  else
    warn "secure mode is disabled (OPENPORTIO_PREFLIGHT_SECURE=false)"
  fi

  local cors="${OPENPORTIO_CORS_ALLOW_ORIGINS:-}"
  if [[ -z "$cors" ]]; then
    warn "OPENPORTIO_CORS_ALLOW_ORIGINS is not set (cross-origin requests disabled by default)"
  elif [[ "$cors" == "*" ]]; then
    warn "OPENPORTIO_CORS_ALLOW_ORIGINS='*' is unsafe for production"
  else
    ok "CORS allow list configured"
  fi

  if [[ -z "${OPENPORTIO_TIMEOUT_SECONDS:-}" ]]; then
    warn "OPENPORTIO_TIMEOUT_SECONDS not set (default 15s applies)"
  else
    ok "OPENPORTIO_TIMEOUT_SECONDS is set"
  fi

  if [[ -z "${OPENPORTIO_REQUEST_BODY_LIMIT_BYTES:-}" ]]; then
    warn "OPENPORTIO_REQUEST_BODY_LIMIT_BYTES not set (default 1048576 applies)"
  else
    ok "OPENPORTIO_REQUEST_BODY_LIMIT_BYTES is set"
  fi

  if [[ -z "${OPENPORTIO_OTEL_EXPORTER_OTLP_ENDPOINT:-}" ]]; then
    warn "OPENPORTIO_OTEL_EXPORTER_OTLP_ENDPOINT not set (OTel export disabled)"
  else
    ok "OPENPORTIO_OTEL_EXPORTER_OTLP_ENDPOINT is set"
  fi
}

start_server_if_needed() {
  if ! is_truthy "$OPENPORTIO_PREFLIGHT_BOOT_SERVER"; then
    return 0
  fi

  SERVER_LOG="$(mktemp)"
  cargo run -p openportio-server --bin openportio-server >"$SERVER_LOG" 2>&1 &
  SERVER_PID="$!"

  if wait_for_server "$OPENPORTIO_PREFLIGHT_BASE_URL" "$OPENPORTIO_PREFLIGHT_WAIT_SECONDS"; then
    ok "booted openportio-server for endpoint checks ($OPENPORTIO_PREFLIGHT_BASE_URL)"
  else
    fail "failed to boot openportio-server within ${OPENPORTIO_PREFLIGHT_WAIT_SECONDS}s"
    if [[ -f "$SERVER_LOG" ]]; then
      printf '[INFO] server log tail:\n'
      tail -n 40 "$SERVER_LOG" || true
    fi
  fi
}

main() {
  trap cleanup EXIT

  check_runtime_config
  start_server_if_needed

  check_endpoint_status "/health" "200"
  check_endpoint_status "/openapi.json" "200"
  check_endpoint_content_type "/openapi.json" "application/json"
  check_endpoint_status "/metrics" "200"
  check_endpoint_content_type "/metrics" "text/plain"
  check_endpoint_status "/grpc/contracts" "200"
  check_endpoint_content_type "/grpc/contracts" "text/html"

  printf '\nSummary: fails=%d warnings=%d\n' "$FAIL_COUNT" "$WARN_COUNT"
  if [[ "$FAIL_COUNT" -gt 0 ]]; then
    exit 1
  fi
}

main "$@"
