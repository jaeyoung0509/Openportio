# production-api

Production-oriented reference example for Openportio.

This example demonstrates:
- explicit env configuration validation
- PostgreSQL-backed REST endpoints
- auth-protected REST routes (`/v1/notes`, `/protected/*`)
- liveness/health/readiness probes
- single-port REST + gRPC serving

## Prerequisites

- Docker + Docker Compose
- `grpcurl`
- `python3` (for local dev JWT helper)

## 1) Start PostgreSQL

From repository root:

```bash
cp examples/production-api/.env.example examples/production-api/.env.local
# edit examples/production-api/.env.local and set local-only values
docker compose --env-file examples/production-api/.env.local \
  -f examples/production-api/docker-compose.yml up -d
```

## 2) Run server

```bash
set -a
source examples/production-api/.env.local
set +a

export PROD_API_DATABASE_URL="postgres://${PROD_API_DB_USER}:${PROD_API_DB_PASSWORD}@127.0.0.1:55432/${PROD_API_DB_NAME}"
export PROD_API_ADDR='127.0.0.1:4100'
export PROD_API_SERVICE_NAME='production-api'
export PROD_API_RUN_MIGRATIONS='true'
export PROD_API_ENABLE_DRILL_ROUTES='false'
export OPENPORTIO_AUTH_ENABLED='true'

cargo run -p production-api
```

## 3) Probe endpoints

```bash
curl -s http://127.0.0.1:4100/livez
curl -s http://127.0.0.1:4100/health
curl -s -i http://127.0.0.1:4100/readyz
```

## 4) Auth-protected REST smoke test

Without token (expected `401`):

```bash
curl -i http://127.0.0.1:4100/v1/notes
```

Create dev token:

```bash
TOKEN=$(python3 scripts/generate_dev_jwt.py \
  --secret "${OPENPORTIO_AUTH_JWT_SECRET}" \
  --issuer "${OPENPORTIO_AUTH_ISSUER}" \
  --audience "${OPENPORTIO_AUTH_AUDIENCE}")
```

Create/list with token:

```bash
curl -s -X POST http://127.0.0.1:4100/v1/notes \
  -H "authorization: Bearer ${TOKEN}" \
  -H 'content-type: application/json' \
  -d '{"title":"Production note","body":"hello"}'

curl -s 'http://127.0.0.1:4100/v1/notes?limit=10&q=prod' \
  -H "authorization: Bearer ${TOKEN}"

# cursor-based next page (use next_cursor from previous response)
curl -s 'http://127.0.0.1:4100/v1/notes?limit=10&cursor=<NEXT_CURSOR>&q=prod' \
  -H "authorization: Bearer ${TOKEN}"
```

List response contract (example):

```json
{
  "notes": [
    { "id": 42, "title": "Production note", "body": "hello", "created_at": "..." }
  ],
  "page": {
    "limit": 10,
    "cursor": null,
    "next_cursor": 42,
    "has_more": true
  },
  "query": "prod"
}
```

## 5) Protected note lookup

Without token (expected `401`):

```bash
curl -i http://127.0.0.1:4100/protected/notes/1
```

Call with token (expected `200` when note exists):

```bash
curl -s http://127.0.0.1:4100/protected/notes/1 \
  -H "authorization: Bearer ${TOKEN}"
```

## 6) gRPC call path on same port

Without token (expected `UNAUTHENTICATED`):

```bash
grpcurl -plaintext \
  -import-path crates/openportio-rpc/proto \
  -proto service.proto \
  -d '{"name":"Rust"}' \
  127.0.0.1:4100 \
  openportio.v1.Greeter/SayHello
```

With token (expected success):

```bash
grpcurl -plaintext \
  -H "authorization: Bearer ${TOKEN}" \
  -import-path crates/openportio-rpc/proto \
  -proto service.proto \
  -d '{"name":"Rust"}' \
  127.0.0.1:4100 \
  openportio.v1.Greeter/SayHello
```

## Failure Recovery Basics

1. Stop PostgreSQL:

```bash
docker compose --env-file examples/production-api/.env.local \
  -f examples/production-api/docker-compose.yml stop postgres
```

2. Readiness should fail (`503`):

```bash
curl -i http://127.0.0.1:4100/readyz
```

3. Start PostgreSQL again:

```bash
docker compose --env-file examples/production-api/.env.local \
  -f examples/production-api/docker-compose.yml start postgres
```

4. Readiness should recover (`200`):

```bash
curl -i http://127.0.0.1:4100/readyz
```

## Failure-Mode Drills (Recommended)

The example now includes automated coverage for:
- auth failures (missing/invalid bearer token)
- validation failures (`400` before database access)
- dependency outage (database unavailable -> `500` on notes endpoints)
- timeout behavior (`408` when request exceeds middleware timeout budget)
- readiness degradation (`/readyz` can fail while `/livez` stays healthy)

### A) Invalid Token Drill

```bash
curl -i http://127.0.0.1:4100/v1/notes \
  -H 'authorization: Bearer invalid-token'
```

Expected: `401 Unauthorized`.

### B) Dependency Outage Drill

```bash
docker compose --env-file examples/production-api/.env.local \
  -f examples/production-api/docker-compose.yml stop postgres

curl -i http://127.0.0.1:4100/readyz
curl -s -i 'http://127.0.0.1:4100/v1/notes?limit=5' \
  -H "authorization: Bearer ${TOKEN}"
curl -i http://127.0.0.1:4100/livez
```

Expected:
- `/readyz` -> `503`
- `/v1/notes` -> `500`
- `/livez` -> `200`

### C) Validation Drill

```bash
curl -s -i 'http://127.0.0.1:4100/v1/notes?limit=101' \
  -H "authorization: Bearer ${TOKEN}"
```

Expected: `400 Bad Request` with `code=validation_error`.

### D) Timeout Drill

Enable drill route and lower timeout budget:

```bash
export PROD_API_ENABLE_DRILL_ROUTES='true'
export OPENPORTIO_TIMEOUT_SECONDS='1'
cargo run -p production-api
```

Then call:

```bash
curl -i http://127.0.0.1:4100/ops/drill/sleep/2 \
  -H "authorization: Bearer ${TOKEN}"
```

Expected: `408 Request Timeout` with body `request timed out`.

## Troubleshooting

- `missing required environment variable: PROD_API_DATABASE_URL`
  - Set `PROD_API_DATABASE_URL` before running.
- `401 unauthorized` on `/v1/notes` or `/protected/*`
  - Verify `OPENPORTIO_AUTH_ENABLED`, `OPENPORTIO_AUTH_JWT_SECRET`, `OPENPORTIO_AUTH_ISSUER`, `OPENPORTIO_AUTH_AUDIENCE`.
  - Recreate token with `scripts/generate_dev_jwt.py`.
- `400 validation_error` on `/v1/notes`
  - Verify query constraints: `limit` range `1..100`, `cursor >= 1`, `q` max length `80`.
- `UNAUTHENTICATED` in gRPC call
  - Confirm `authorization: Bearer <token>` metadata and issuer/audience match.
- `503` from `/readyz`
  - Check PostgreSQL container health and DB URL host/port/database credentials.
- `relation "notes" does not exist`
  - Ensure migrations are enabled (`PROD_API_RUN_MIGRATIONS=true`) and wait for migration retry logs to succeed.

## Shutdown / Cleanup

```bash
docker compose --env-file examples/production-api/.env.local \
  -f examples/production-api/docker-compose.yml down -v
```
