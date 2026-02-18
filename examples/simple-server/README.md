# simple-server

Runnable sample for FastAPI-like DTO validation, DI, SSE, WebSocket, and single-port REST+gRPC with context-based gRPC handlers (`with_grpc_say_hello_with_context`).

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
```

## gRPC Auth Flow

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

Call without token (expected `UNAUTHENTICATED`):

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

Call with token (expected success):

```bash
grpcurl -plaintext \
  -H "authorization: Bearer ${TOKEN}" \
  -import-path crates/openportio-rpc/proto \
  -proto service.proto \
  -d '{"name":"Rust"}' \
  127.0.0.1:4000 \
  openportio.v1.Greeter/SayHello
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
