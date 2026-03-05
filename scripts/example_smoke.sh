#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

SIMPLE_ADDR="${OPENPORTIO_EXAMPLE_SIMPLE_ADDR:-127.0.0.1:4000}"
PRODUCTION_ADDR="${OPENPORTIO_EXAMPLE_PRODUCTION_ADDR:-127.0.0.1:4100}"
WAIT_SECONDS="${OPENPORTIO_EXAMPLE_WAIT_SECONDS:-150}"
PROTO_IMPORT_PATH="crates/openportio-rpc/proto"
PROTO_FILE="service.proto"
COMPOSE_FILE="examples/production-api/docker-compose.yml"
COMPOSE_PROJECT="openportio-example-smoke-$(date +%s)-$$"

SIMPLE_PID=""
PRODUCTION_PID=""
SIMPLE_LOG=""
PRODUCTION_LOG=""
SMOKE_ENV_FILE=""
POSTGRES_STARTED=false

SIMPLE_SECRET="simple-smoke-secret"
PRODUCTION_SECRET="production-smoke-secret"
AUTH_ISSUER="https://issuer.local"
AUTH_AUDIENCE="openportio-api"

info() {
  printf '[INFO] %s\n' "$1"
}

ok() {
  printf '[OK] %s\n' "$1"
}

fail() {
  printf '[FAIL] %s\n' "$1" >&2
  exit 1
}

validate_loopback_addr() {
  local value="$1"
  local label="$2"
  local port=""

  if [[ "$value" =~ ^127\.0\.0\.1:([0-9]{1,5})$ ]]; then
    port="${BASH_REMATCH[1]}"
  elif [[ "$value" =~ ^localhost:([0-9]{1,5})$ ]]; then
    port="${BASH_REMATCH[1]}"
  else
    fail "${label} must use loopback host (127.0.0.1 or localhost) with numeric port; got '${value}'"
  fi

  if ((port < 1 || port > 65535)); then
    fail "${label} has invalid port in '${value}'"
  fi
}

require_command() {
  local command="$1"
  if ! command -v "$command" >/dev/null 2>&1; then
    fail "missing required command: $command"
  fi
}

cleanup() {
  if [[ -n "$PRODUCTION_PID" ]]; then
    kill "$PRODUCTION_PID" >/dev/null 2>&1 || true
    wait "$PRODUCTION_PID" >/dev/null 2>&1 || true
  fi

  if [[ -n "$SIMPLE_PID" ]]; then
    kill "$SIMPLE_PID" >/dev/null 2>&1 || true
    wait "$SIMPLE_PID" >/dev/null 2>&1 || true
  fi

  if [[ "$POSTGRES_STARTED" == "true" && -n "$SMOKE_ENV_FILE" ]]; then
    docker compose \
      -p "$COMPOSE_PROJECT" \
      --env-file "$SMOKE_ENV_FILE" \
      -f "$COMPOSE_FILE" \
      down -v >/dev/null 2>&1 || true
  fi

  if [[ -n "$SIMPLE_LOG" && -f "$SIMPLE_LOG" ]]; then
    rm -f "$SIMPLE_LOG"
  fi
  if [[ -n "$PRODUCTION_LOG" && -f "$PRODUCTION_LOG" ]]; then
    rm -f "$PRODUCTION_LOG"
  fi
  if [[ -n "$SMOKE_ENV_FILE" && -f "$SMOKE_ENV_FILE" ]]; then
    rm -f "$SMOKE_ENV_FILE"
  fi
}

wait_for_http_status() {
  local url="$1"
  local expected_code="$2"
  local wait_seconds="$3"
  local process_pid="$4"
  local service_label="$5"
  local attempts=$((wait_seconds * 4))
  local code=""

  for _ in $(seq 1 "$attempts"); do
    code="$(curl -s -o /dev/null -w '%{http_code}' "$url" || true)"
    if [[ "$code" == "$expected_code" ]]; then
      return 0
    fi

    if [[ -n "$process_pid" ]] && ! kill -0 "$process_pid" >/dev/null 2>&1; then
      return 1
    fi
    sleep 0.25
  done

  info "$service_label endpoint $url returned $code while waiting for $expected_code"
  return 1
}

http_expect_status() {
  local method="$1"
  local url="$2"
  local expected_code="$3"
  local body_file="$4"
  shift 4

  local code
  set +e
  code="$(curl -sS -X "$method" -o "$body_file" -w '%{http_code}' "$url" "$@")"
  local rc=$?
  set -e

  if [[ "$rc" -ne 0 ]]; then
    fail "HTTP $method $url failed to execute"
  fi

  if [[ "$code" != "$expected_code" ]]; then
    if [[ -s "$body_file" ]]; then
      fail "HTTP $method $url returned $code (expected $expected_code): $(cat "$body_file")"
    fi
    fail "HTTP $method $url returned $code (expected $expected_code)"
  fi
}

json_assert_file() {
  local file_path="$1"
  local expression="$2"
  local description="$3"

  if ! python3 - "$file_path" "$expression" <<'PY'
import json
import sys

path = sys.argv[1]
expr = sys.argv[2]

with open(path, "r", encoding="utf-8") as handle:
    payload = json.load(handle)

context = {
    "payload": payload,
    "len": len,
    "isinstance": isinstance,
    "list": list,
    "dict": dict,
    "int": int,
    "str": str,
}

if not eval(expr, {"__builtins__": {}}, context):
    raise SystemExit(1)
PY
  then
    fail "JSON assertion failed: $description"
  fi
}

generate_dev_token() {
  local secret="$1"
  python3 scripts/generate_dev_jwt.py \
    --secret "$secret" \
    --issuer "$AUTH_ISSUER" \
    --audience "$AUTH_AUDIENCE"
}

grpc_say_hello() {
  local addr="$1"
  local name="$2"
  local token="${3:-}"
  local -a cmd=(
    grpcurl
    -plaintext
    -import-path "$PROTO_IMPORT_PATH"
    -proto "$PROTO_FILE"
  )

  if [[ -n "$token" ]]; then
    cmd+=(-H "authorization: Bearer ${token}")
  fi

  cmd+=(-d "{\"name\":\"${name}\"}" "$addr" openportio.v1.Greeter/SayHello)
  "${cmd[@]}"
}

grpc_expect_unauthenticated() {
  local addr="$1"
  local output

  set +e
  output="$(grpc_say_hello "$addr" "Rust" 2>&1)"
  local status=$?
  set -e

  if [[ "$status" -eq 0 ]]; then
    fail "gRPC unauthenticated check unexpectedly succeeded on $addr"
  fi

  if [[ "$output" != *"Unauthenticated"* && "$output" != *"UNAUTHENTICATED"* ]]; then
    fail "gRPC unauthenticated check did not report UNAUTHENTICATED on $addr: $output"
  fi
}

grpc_expect_message_prefix() {
  local addr="$1"
  local token="$2"
  local expected_prefix="$3"
  local output
  local stderr_file
  stderr_file="$(mktemp)"

  set +e
  output="$(grpc_say_hello "$addr" "Rust" "$token" 2>"$stderr_file")"
  local status=$?
  set -e

  if [[ "$status" -ne 0 ]]; then
    local stderr_output
    stderr_output="$(cat "$stderr_file" || true)"
    rm -f "$stderr_file"
    fail "gRPC success call failed on $addr: ${stderr_output}${output}"
  fi

  if ! python3 - "$expected_prefix" "$output" <<'PY'
import json
import sys

expected = sys.argv[1]
raw = sys.argv[2]
payload = json.loads(raw)
message = payload.get("message", "")
if not message.startswith(expected):
    raise SystemExit(1)
PY
  then
    rm -f "$stderr_file"
    fail "gRPC response message does not start with expected prefix '$expected_prefix': $output"
  fi

  rm -f "$stderr_file"
}

start_simple_server() {
  SIMPLE_LOG="$(mktemp)"
  OPENPORTIO_AUTH_ENABLED=true \
  OPENPORTIO_AUTH_JWT_SECRET="$SIMPLE_SECRET" \
  OPENPORTIO_AUTH_ISSUER="$AUTH_ISSUER" \
  OPENPORTIO_AUTH_AUDIENCE="$AUTH_AUDIENCE" \
  cargo run -p simple-server >"$SIMPLE_LOG" 2>&1 &
  SIMPLE_PID="$!"

  if ! wait_for_http_status "http://${SIMPLE_ADDR}/health" "200" "$WAIT_SECONDS" "$SIMPLE_PID" "simple-server"; then
    info "simple-server log tail:"
    tail -n 60 "$SIMPLE_LOG" || true
    fail "simple-server failed to start on $SIMPLE_ADDR"
  fi
}

run_simple_server_checks() {
  info "Running simple-server smoke checks on $SIMPLE_ADDR"
  local token
  token="$(generate_dev_token "$SIMPLE_SECRET")"
  local body
  body="$(mktemp)"

  http_expect_status "GET" "http://${SIMPLE_ADDR}/health" "200" "$body"
  ok "simple-server /health returned 200"

  http_expect_status "GET" "http://${SIMPLE_ADDR}/notes?limit=3" "200" "$body"
  json_assert_file "$body" "payload.get('limit') == 3" "simple-server /notes limit should be 3"
  ok "simple-server /notes limit validation success path"

  http_expect_status "GET" "http://${SIMPLE_ADDR}/notes?limit=0" "400" "$body"
  json_assert_file "$body" "payload.get('code') == 'validation_error'" "simple-server invalid query should produce validation_error"
  ok "simple-server invalid query rejected with validation_error"

  http_expect_status "GET" "http://${SIMPLE_ADDR}/protected/greet/Rust" "401" "$body"
  ok "simple-server protected REST route rejects missing token"

  http_expect_status \
    "GET" \
    "http://${SIMPLE_ADDR}/protected/greet/Rust" \
    "200" \
    "$body" \
    -H "authorization: Bearer ${token}"
  json_assert_file \
    "$body" \
    "payload.get('subject') == 'dev-user' and payload.get('message', '').startswith('[simple-server:rest:dev-user]')" \
    "simple-server protected REST route should return shared use-case message"
  ok "simple-server protected REST route accepts valid token"

  grpc_expect_unauthenticated "$SIMPLE_ADDR"
  ok "simple-server gRPC route rejects missing token"

  grpc_expect_message_prefix "$SIMPLE_ADDR" "$token" "[simple-server:grpc:dev-user]"
  ok "simple-server gRPC route accepts valid token"

  rm -f "$body"
}

start_production_postgres() {
  require_command docker
  if ! docker info >/dev/null 2>&1; then
    fail "docker daemon is not reachable; start Docker/OrbStack (or equivalent) and retry scripts/example_smoke.sh"
  fi
  if ! docker compose version >/dev/null 2>&1; then
    fail "docker compose plugin is required for production-api smoke checks; install it and retry"
  fi

  local db_password
  db_password="$(python3 - <<'PY'
import secrets
print(f"smoke-db-{secrets.token_hex(8)}")
PY
)"
  SMOKE_ENV_FILE="$(mktemp)"
  cat >"$SMOKE_ENV_FILE" <<EOF
PROD_API_DB_USER=postgres
PROD_API_DB_PASSWORD=${db_password}
PROD_API_DB_NAME=openportio
OPENPORTIO_AUTH_ENABLED=true
OPENPORTIO_AUTH_JWT_SECRET=${PRODUCTION_SECRET}
OPENPORTIO_AUTH_ISSUER=${AUTH_ISSUER}
OPENPORTIO_AUTH_AUDIENCE=${AUTH_AUDIENCE}
EOF

  docker compose \
    -p "$COMPOSE_PROJECT" \
    --env-file "$SMOKE_ENV_FILE" \
    -f "$COMPOSE_FILE" \
    up -d
  POSTGRES_STARTED=true
}

start_production_api() {
  PRODUCTION_LOG="$(mktemp)"
  set -a
  # shellcheck disable=SC1090
  source "$SMOKE_ENV_FILE"
  set +a

  PROD_API_DATABASE_URL="postgres://${PROD_API_DB_USER}:${PROD_API_DB_PASSWORD}@127.0.0.1:55432/${PROD_API_DB_NAME}" \
  PROD_API_ADDR="$PRODUCTION_ADDR" \
  PROD_API_SERVICE_NAME="production-api" \
  PROD_API_RUN_MIGRATIONS=true \
  PROD_API_ENABLE_DRILL_ROUTES=false \
  OPENPORTIO_AUTH_ENABLED=true \
  OPENPORTIO_AUTH_JWT_SECRET="$PRODUCTION_SECRET" \
  OPENPORTIO_AUTH_ISSUER="$AUTH_ISSUER" \
  OPENPORTIO_AUTH_AUDIENCE="$AUTH_AUDIENCE" \
  cargo run -p production-api >"$PRODUCTION_LOG" 2>&1 &
  PRODUCTION_PID="$!"

  if ! wait_for_http_status "http://${PRODUCTION_ADDR}/readyz" "200" "$WAIT_SECONDS" "$PRODUCTION_PID" "production-api"; then
    info "production-api log tail:"
    tail -n 80 "$PRODUCTION_LOG" || true
    fail "production-api failed to reach ready state on $PRODUCTION_ADDR"
  fi
}

run_production_api_checks() {
  info "Running production-api smoke checks on $PRODUCTION_ADDR"
  local token
  token="$(generate_dev_token "$PRODUCTION_SECRET")"

  local body
  body="$(mktemp)"

  http_expect_status "GET" "http://${PRODUCTION_ADDR}/livez" "200" "$body"
  ok "production-api /livez returned 200"

  http_expect_status "GET" "http://${PRODUCTION_ADDR}/readyz" "200" "$body"
  ok "production-api /readyz returned 200"

  http_expect_status "GET" "http://${PRODUCTION_ADDR}/v1/notes" "401" "$body"
  ok "production-api /v1/notes rejects missing token"

  http_expect_status \
    "POST" \
    "http://${PRODUCTION_ADDR}/v1/notes" \
    "201" \
    "$body" \
    -H "authorization: Bearer ${token}" \
    -H "content-type: application/json" \
    -d '{"title":"Smoke Note","body":"smoke body"}'
  json_assert_file "$body" "isinstance(payload.get('id'), int) and payload.get('title') == 'Smoke Note'" "production-api note creation response"
  local created_note_id
  created_note_id="$(python3 - "$body" <<'PY'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    payload = json.load(handle)
print(payload["id"])
PY
)"
  ok "production-api note creation succeeded (id=${created_note_id})"

  http_expect_status \
    "GET" \
    "http://${PRODUCTION_ADDR}/v1/notes?limit=5&q=Smoke" \
    "200" \
    "$body" \
    -H "authorization: Bearer ${token}"
  json_assert_file \
    "$body" \
    "isinstance(payload.get('notes'), list) and len(payload['notes']) >= 1 and payload['notes'][0].get('id') >= 1 and payload.get('page', {}).get('limit') == 5" \
    "production-api list notes response shape"
  ok "production-api list notes endpoint returned paginated result"

  http_expect_status \
    "GET" \
    "http://${PRODUCTION_ADDR}/protected/notes/${created_note_id}" \
    "200" \
    "$body" \
    -H "authorization: Bearer ${token}"
  json_assert_file "$body" "payload.get('subject') == 'dev-user'" "production-api protected note response"
  ok "production-api protected note endpoint returns owner-scoped payload"

  http_expect_status \
    "GET" \
    "http://${PRODUCTION_ADDR}/v1/greetings/Rust" \
    "200" \
    "$body" \
    -H "authorization: Bearer ${token}"
  json_assert_file \
    "$body" \
    "payload.get('message', '').startswith('[production-api][rest] hello, Rust (actor=dev-user)')" \
    "production-api shared REST greeting use-case message"
  ok "production-api REST greeting endpoint uses shared application use-case"

  grpc_expect_unauthenticated "$PRODUCTION_ADDR"
  ok "production-api gRPC route rejects missing token"

  grpc_expect_message_prefix "$PRODUCTION_ADDR" "$token" "[production-api][grpc] hello, Rust (actor=dev-user)"
  ok "production-api gRPC route accepts valid token"

  rm -f "$body"
}

main() {
  trap cleanup EXIT

  require_command cargo
  require_command curl
  require_command grpcurl
  require_command python3
  validate_loopback_addr "$SIMPLE_ADDR" "OPENPORTIO_EXAMPLE_SIMPLE_ADDR"
  validate_loopback_addr "$PRODUCTION_ADDR" "OPENPORTIO_EXAMPLE_PRODUCTION_ADDR"

  info "Starting example smoke verification (simple-server + production-api)"
  info "Runtime assumptions: simple-server=${SIMPLE_ADDR}, production-api=${PRODUCTION_ADDR}"

  start_simple_server
  run_simple_server_checks

  start_production_postgres
  start_production_api
  run_production_api_checks

  ok "Example smoke verification completed successfully"
}

main "$@"
