# Production Deployment

This guide describes minimum production deployment patterns for Openportio.

## Runtime Prerequisites

- Rust binary built from `openportio-server`
- Port binding for your target address (default `127.0.0.1:3000`)
- Reverse proxy / ingress for TLS termination

## Required Environment Variables (Secure Baseline)

- `OPENPORTIO_AUTH_ENABLED=true`
- choose one auth mode:
  - `OPENPORTIO_AUTH_JWT_SECRET=<strong-random-secret>` (shared-secret mode)
  - `OPENPORTIO_AUTH_JWKS_URL=<https://issuer/.well-known/jwks.json>` (JWKS mode)
- optional JWKS tuning:
  - `OPENPORTIO_AUTH_JWKS_REFRESH_SECS=300`
  - `OPENPORTIO_AUTH_JWKS_ALGORITHMS=RS256,ES256`
- `OPENPORTIO_AUTH_ISSUER=<issuer-url>` (recommended)
- `OPENPORTIO_AUTH_AUDIENCE=<audience>` (recommended)
- `OPENPORTIO_CORS_ALLOW_ORIGINS=<comma-separated allowlist>`

Recommended hardening:
- `OPENPORTIO_TIMEOUT_SECONDS=15` (or lower based on SLO)
- `OPENPORTIO_REQUEST_BODY_LIMIT_BYTES=1048576` (or stricter)
- `OPENPORTIO_MAX_IN_FLIGHT_REQUESTS=1024` (size to capacity)
- `OPENPORTIO_METRICS_PATH=/metrics` (or custom path for scraping policy)

Optional observability export:
- `OPENPORTIO_SERVICE_NAME=openportio-api`
- `OPENPORTIO_OTEL_EXPORTER_OTLP_ENDPOINT=http://otel-collector:4317`
- `OPENPORTIO_OTEL_TRACE_SAMPLE_RATIO=0.2`
- `OPENPORTIO_OTEL_EXPORTER_TIMEOUT_SECONDS=5`

## Local Production-Like Run

```bash
OPENPORTIO_SERVER_ADDR=127.0.0.1:3000 \
OPENPORTIO_AUTH_ENABLED=true \
OPENPORTIO_AUTH_JWT_SECRET=replace-me \
OPENPORTIO_AUTH_ISSUER=https://issuer.local \
OPENPORTIO_AUTH_AUDIENCE=openportio-api \
OPENPORTIO_CORS_ALLOW_ORIGINS=https://app.example.com \
cargo run -p openportio-server
```

## Single-Port vs Dual-Port Deployment

Single-port (default) runs REST + gRPC on one listener:
- simpler ingress and fewer exposed ports
- good default for most teams

Dual-port mode splits REST and gRPC listeners:
- useful when your platform/load-balancer expects separate protocol ports
- enables independent traffic policy and SLO management per protocol

Programmatic dual-port example:

```rust
use openportio_server::OpenportioServer;

OpenportioServer::new()
    .with_rest_addr(([0, 0, 0, 0], 3000).into())
    .with_grpc_addr(([0, 0, 0, 0], 50051).into())
    .run()
    .await?;
```

Migration guidance:
- start from your existing single-port config
- introduce a dedicated gRPC port and route gRPC traffic there first
- keep REST on current port, validate probes/alerts, then remove single-port assumptions from infrastructure

## Preflight Gate (Recommended Before Rollout)

```bash
OPENPORTIO_PREFLIGHT_SECURE=true \
OPENPORTIO_PREFLIGHT_BOOT_SERVER=true \
OPENPORTIO_AUTH_ENABLED=true \
OPENPORTIO_AUTH_JWT_SECRET=replace-me \
OPENPORTIO_AUTH_ISSUER=https://issuer.local \
OPENPORTIO_AUTH_AUDIENCE=openportio-api \
OPENPORTIO_CORS_ALLOW_ORIGINS=https://app.example.com \
./scripts/prod_preflight.sh
```

## Systemd Example

```ini
[Unit]
Description=Openportio Server
After=network.target

[Service]
WorkingDirectory=/opt/openportio
ExecStart=/opt/openportio/openportio-server
Restart=always
RestartSec=5
Environment=OPENPORTIO_SERVER_ADDR=0.0.0.0:3000
Environment=OPENPORTIO_AUTH_ENABLED=true
Environment=OPENPORTIO_AUTH_JWT_SECRET=replace-me
Environment=OPENPORTIO_CORS_ALLOW_ORIGINS=https://app.example.com

[Install]
WantedBy=multi-user.target
```

## Kubernetes Notes

- Use readiness probe on `/health`
- Scrape metrics from `/metrics` (or configured metrics path)
- Put secrets in `Secret`, non-sensitive config in `ConfigMap`
- Terminate TLS at ingress and forward to Openportio over private network
