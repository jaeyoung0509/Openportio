# Operations Runbook

This runbook provides first-response steps for common production incidents.

## 1) Service Not Reachable

Checklist:
- Confirm process is running
- Confirm bind address (`OPENPORTIO_SERVER_ADDR`)
- Check `/health` through service endpoint
- Check ingress / load balancer routing

Commands:

```bash
curl -i http://127.0.0.1:3000/health
```

## 2) Auth Failures

Symptoms:
- REST `401 unauthorized`
- gRPC `UNAUTHENTICATED`

Checklist:
- `OPENPORTIO_AUTH_ENABLED=true`
- JWT secret matches token signer
- issuer/audience claims match runtime config

## 3) Contract Endpoint Failures

Symptoms:
- `/openapi.json` unavailable
- `/grpc/contracts` unavailable

Checklist:
- Validate startup logs for server initialization issues
- Run preflight locally in same environment

```bash
OPENPORTIO_PREFLIGHT_BOOT_SERVER=true ./scripts/prod_preflight.sh
```

## 4) Release Gate Before Rollout

Execute:

```bash
OPENPORTIO_PREFLIGHT_SECURE=true \
OPENPORTIO_PREFLIGHT_BOOT_SERVER=true \
OPENPORTIO_AUTH_ENABLED=true \
OPENPORTIO_AUTH_JWT_SECRET=replace-me \
OPENPORTIO_CORS_ALLOW_ORIGINS=https://app.example.com \
./scripts/prod_preflight.sh
```

## 5) Example Smoke Gate (REST + gRPC + Auth)

Run this before rollout when you want end-to-end confidence on reference apps:

```bash
./scripts/example_smoke.sh
```

It validates:
- `examples/simple-server` REST + gRPC
- `examples/production-api` REST + gRPC
- auth failure/success behavior for both protocols

Prerequisites:
- `grpcurl`
- `python3`
- Docker + Docker Compose

If the gate fails:
- read the emitted service log tail in CI/local output
- verify Docker daemon availability and Postgres container health
- verify JWT settings (`OPENPORTIO_AUTH_JWT_SECRET`, `OPENPORTIO_AUTH_ISSUER`, `OPENPORTIO_AUTH_AUDIENCE`)

Rollback guideline:
- If preflight has critical failures, stop rollout.
- Revert to last known-good release and re-run preflight after configuration fixes.

For detailed local failure-mode rehearsal on the production reference service, see:
- `docs/production/production-api-runbook.md`
