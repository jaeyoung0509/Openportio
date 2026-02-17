# production-api Runbook

This runbook covers local boot, migration behavior, smoke tests, and dependency failure recovery for `examples/production-api`.

## Environment Variables

Required:

- `PROD_API_DATABASE_URL` (build from local DB env vars)

Optional (defaults shown):

- `PROD_API_ADDR=127.0.0.1:4100`
- `PROD_API_SERVICE_NAME=production-api`
- `PROD_API_DB_MAX_CONNECTIONS=10`
- `PROD_API_RUN_MIGRATIONS=true`
- `PROD_API_MIGRATION_RETRY_SECONDS=5`
- `PROD_API_ENABLE_DRILL_ROUTES=false`

Auth-related (recommended for realistic production flow):

- `OPENPORTIO_AUTH_ENABLED=true`
- `OPENPORTIO_AUTH_JWT_SECRET=<local-only-secret>`
- `OPENPORTIO_AUTH_ISSUER=<issuer>`
- `OPENPORTIO_AUTH_AUDIENCE=<audience>`

## Boot Procedure

1. Start PostgreSQL:

```bash
cp examples/production-api/.env.example examples/production-api/.env.local
# edit examples/production-api/.env.local and set local-only values
docker compose --env-file examples/production-api/.env.local \
  -f examples/production-api/docker-compose.yml up -d
```

2. Export env vars and start API:

```bash
set -a
source examples/production-api/.env.local
set +a

export PROD_API_DATABASE_URL="postgres://${PROD_API_DB_USER}:${PROD_API_DB_PASSWORD}@127.0.0.1:55432/${PROD_API_DB_NAME}"
export OPENPORTIO_AUTH_ENABLED='true'
export PROD_API_ENABLE_DRILL_ROUTES='false'

cargo run -p production-api
```

3. Verify probes:

```bash
curl -s http://127.0.0.1:4100/livez
curl -s http://127.0.0.1:4100/health
curl -i http://127.0.0.1:4100/readyz
```

## Migration Behavior

- Migrations are loaded from `examples/production-api/migrations`.
- On startup, migration execution runs in a retry loop when `PROD_API_RUN_MIGRATIONS=true`.
- If DB is temporarily unavailable, the service remains up and retries migrations every `PROD_API_MIGRATION_RETRY_SECONDS` seconds.

## Smoke Tests

REST:

```bash
TOKEN=$(python3 scripts/generate_dev_jwt.py \
  --secret "${OPENPORTIO_AUTH_JWT_SECRET}" \
  --issuer "${OPENPORTIO_AUTH_ISSUER}" \
  --audience "${OPENPORTIO_AUTH_AUDIENCE}")

curl -i http://127.0.0.1:4100/v1/notes

curl -s -X POST http://127.0.0.1:4100/v1/notes \
  -H "authorization: Bearer ${TOKEN}" \
  -H 'content-type: application/json' \
  -d '{"title":"hello","body":"world"}'

curl -s 'http://127.0.0.1:4100/v1/notes?limit=5' \
  -H "authorization: Bearer ${TOKEN}"

curl -s http://127.0.0.1:4100/protected/notes/1 \
  -H "authorization: Bearer ${TOKEN}"
```

gRPC:

```bash
grpcurl -plaintext \
  -H "authorization: Bearer ${TOKEN}" \
  -import-path crates/openportio-rpc/proto \
  -proto service.proto \
  -d '{"name":"Rust"}' \
  127.0.0.1:4100 \
  openportio.v1.Greeter/SayHello
```

## Failure Recovery

Scenario: PostgreSQL outage.

1. Simulate outage:

```bash
docker compose --env-file examples/production-api/.env.local \
  -f examples/production-api/docker-compose.yml stop postgres
```

2. Observe readiness failure (`503`):

```bash
curl -i http://127.0.0.1:4100/readyz
```

3. Recover DB:

```bash
docker compose --env-file examples/production-api/.env.local \
  -f examples/production-api/docker-compose.yml start postgres
```

4. Confirm readiness returns `200`.

If readiness does not recover:
- verify DB container health (`docker ps` / `docker logs`)
- verify `PROD_API_DATABASE_URL`
- verify migrations path and logs for migration retry errors

## Failure-Mode Drill Matrix

Use these drills for incident rehearsal and operational readiness validation.

### 1) Auth failure (invalid token)

```bash
curl -i http://127.0.0.1:4100/v1/notes \
  -H 'authorization: Bearer invalid-token'
```

Expected: `401 Unauthorized`.

### 2) Dependency outage

```bash
docker compose --env-file examples/production-api/.env.local \
  -f examples/production-api/docker-compose.yml stop postgres

curl -i http://127.0.0.1:4100/readyz
curl -i http://127.0.0.1:4100/livez
curl -s -i 'http://127.0.0.1:4100/v1/notes?limit=5' \
  -H "authorization: Bearer ${TOKEN}"
```

Expected:
- `/readyz` -> `503`
- `/livez` -> `200`
- `/v1/notes` -> `500`

### 3) Timeout behavior

Enable local drill route and lower timeout budget:

```bash
export PROD_API_ENABLE_DRILL_ROUTES='true'
export OPENPORTIO_TIMEOUT_SECONDS='1'
cargo run -p production-api
```

Then trigger:

```bash
curl -i http://127.0.0.1:4100/ops/drill/sleep/2 \
  -H "authorization: Bearer ${TOKEN}"
```

Expected: `408 Request Timeout` with body `request timed out`.

## Shutdown

```bash
docker compose --env-file examples/production-api/.env.local \
  -f examples/production-api/docker-compose.yml down -v
```
