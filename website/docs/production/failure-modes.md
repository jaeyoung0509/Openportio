# Production Failure-Mode Drills

Use this page to run realistic operational drills on `examples/production-api`.

## Why This Matters

Framework trust is not only about happy-path benchmarks.
You need reproducible failure handling for:

- auth rejection
- dependency outage
- timeout behavior
- readiness degradation with liveness continuity

## Prerequisites

- start Postgres with `examples/production-api/.env.local`
- run `cargo run -p production-api`
- create a test token:

```bash
TOKEN=$(python3 scripts/generate_dev_jwt.py \
  --secret "${OPENPORTIO_AUTH_JWT_SECRET}" \
  --issuer "${OPENPORTIO_AUTH_ISSUER}" \
  --audience "${OPENPORTIO_AUTH_AUDIENCE}")
```

## Drill 1: Invalid Token

```bash
curl -i http://127.0.0.1:4100/v1/notes \
  -H 'authorization: Bearer invalid-token'
```

Expected: `401 Unauthorized`.

## Drill 2: Dependency Outage

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

Recover DB:

```bash
docker compose --env-file examples/production-api/.env.local \
  -f examples/production-api/docker-compose.yml start postgres
```

## Drill 3: Timeout Budget

Enable local drill routes and force small timeout:

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

Expected: `408 Request Timeout` and body `request timed out`.

## References

- [examples/production-api/README.md](https://github.com/jaeyoung0509/Openportio/blob/develop/examples/production-api/README.md)
- [docs/production/production-api-runbook.md](https://github.com/jaeyoung0509/Openportio/blob/develop/docs/production/production-api-runbook.md)
