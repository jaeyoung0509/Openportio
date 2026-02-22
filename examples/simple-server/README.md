# simple-server

Integrated onboarding reference for Openportio.

This example demonstrates, in one runnable app:
- REST DTO validation (`ValidatedJson`, `ValidatedQuery`, `ValidatedPath`)
- Depends-style DI in REST (`Depends<T>`)
- gRPC context-based handler with validation + DI + principal
- shared application use case reused by REST and gRPC handlers
- single-port REST + gRPC runtime with optional auth toggle

## Prerequisites

- `grpcurl` installed
- `python3` installed (for development token helper)

## Run

From repository root:

```bash
cargo run -p simple-server
```

`simple-server` binds to `127.0.0.1:4000`.

## REST Smoke Check

```bash
curl -s http://127.0.0.1:4000/health
curl -s http://127.0.0.1:4000/notes?limit=3
curl -i 'http://127.0.0.1:4000/notes?limit=0'   # expected 400 validation_error
```

## gRPC Quickstart (No Auth)

```bash
grpcurl -plaintext 127.0.0.1:4000 list
grpcurl -plaintext \
  -import-path crates/openportio-rpc/proto \
  -proto service.proto \
  -d '{"name":"Rust"}' \
  127.0.0.1:4000 \
  openportio.v1.Greeter/SayHello

grpcurl -plaintext \
  -import-path crates/openportio-rpc/proto \
  -proto service.proto \
  -d '{"name":""}' \
  127.0.0.1:4000 \
  openportio.v1.Greeter/SayHello   # expected INVALID_ARGUMENT
```

## Optional Auth Toggle (REST + gRPC)

Restart server with auth enabled:

```bash
OPENPORTIO_AUTH_ENABLED=true \
OPENPORTIO_AUTH_JWT_SECRET=dev-secret \
OPENPORTIO_AUTH_ISSUER=https://issuer.local \
OPENPORTIO_AUTH_AUDIENCE=openportio-api \
cargo run -p simple-server
```

Alternative auth mode:
- instead of `OPENPORTIO_AUTH_JWT_SECRET`, you can use
  `OPENPORTIO_AUTH_JWKS_URL=<issuer jwks endpoint>`
- optional: `OPENPORTIO_AUTH_JWKS_REFRESH_SECS`, `OPENPORTIO_AUTH_JWKS_ALGORITHMS`

Call protected REST without token (expected `401`):

```bash
curl -i http://127.0.0.1:4000/protected/greet/Rust
```

Call gRPC without token (expected `UNAUTHENTICATED`):

```bash
grpcurl -plaintext \
  -import-path crates/openportio-rpc/proto \
  -proto service.proto \
  -d '{"name":"Rust"}' \
  127.0.0.1:4000 \
  openportio.v1.Greeter/SayHello
```

Generate a development token (dev-only helper):

```bash
TOKEN=$(python3 scripts/generate_dev_jwt.py \
  --secret dev-secret \
  --issuer https://issuer.local \
  --audience openportio-api)
```

## Shared Use Case Across REST + gRPC

Both endpoints execute the same greeting use case:
- REST: `GET /protected/greet/:id`
- gRPC: `openportio.v1.Greeter/SayHello`

Call REST with token (expected `200`):

```bash
curl -s http://127.0.0.1:4000/protected/greet/Rust \
  -H "authorization: Bearer ${TOKEN}"
```

Call gRPC with token (expected success):

```bash
grpcurl -plaintext \
  -H "authorization: Bearer ${TOKEN}" \
  -import-path crates/openportio-rpc/proto \
  -proto service.proto \
  -d '{"name":"Rust"}' \
  127.0.0.1:4000 \
  openportio.v1.Greeter/SayHello
```

Expected message prefix contains service/adapter/actor metadata from DI + principal context:
- REST: `[simple-server:rest:<subject>] ...`
- gRPC: `[simple-server:grpc:<subject>] ...`

## DI Example Check

REST `GET /notes/:id` uses `Depends<ServiceInfo>` and request context extraction:

```bash
curl -s http://127.0.0.1:4000/notes/Rust \
  -H "x-request-id: onboarding-1"
```

## Troubleshooting

- `grpcurl: command not found`
  - Install grpcurl, then re-run commands.
- `python3: command not found`
  - Install Python 3 or generate token with another JWT tool.
- `Code: Unauthenticated`
  - Verify `OPENPORTIO_AUTH_JWT_SECRET`, `OPENPORTIO_AUTH_ISSUER`, `OPENPORTIO_AUTH_AUDIENCE` match token inputs.
- `Code: Internal` in JWKS mode
  - Verify `OPENPORTIO_AUTH_JWKS_URL` is reachable and returns a valid JWKS document with expected `kid`.
- `Failed to process proto source files` / import errors
  - Run commands from repository root so `crates/openportio-rpc/proto` and `service.proto` resolve correctly.
