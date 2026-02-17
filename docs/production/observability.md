# Observability Guide

Openportio includes basic observability primitives through middleware and tracing.

## Built-in Signals

- Structured logs via `tracing` / `tracing-subscriber`
- Request ID propagation (`x-request-id`)
- Prometheus-style metrics endpoint (`/metrics` by default)
- Health endpoint: `GET /health`
- API contract endpoints:
  - `GET /openapi.json`
  - `GET /grpc/contracts`

## Logging Baseline

Set log level using standard Rust tracing environment:

```bash
RUST_LOG=info cargo run -p openportio-server
```

For deeper diagnostics:

```bash
RUST_LOG=debug,openportio_server=debug cargo run -p openportio-server
```

## OTel (Opt-In) Setup

Openportio keeps OTel disabled unless an exporter endpoint is configured.

## Configuration Matrix

| Variable | Default | Purpose |
| --- | --- | --- |
| `RUST_LOG` | `info,openportio_server=info,tower_http=info` | log filter baseline |
| `OPENPORTIO_METRICS_PATH` | `/metrics` | metrics endpoint path |
| `OPENPORTIO_SERVICE_NAME` | `openportio-server` | service identity in trace resource |
| `OPENPORTIO_OTEL_EXPORTER_OTLP_ENDPOINT` | unset | enable OTLP exporter when set |
| `OPENPORTIO_OTEL_TRACE_SAMPLE_RATIO` | `1.0` | trace sampling ratio (`0.0` to `1.0`) |
| `OPENPORTIO_OTEL_EXPORTER_TIMEOUT_SECONDS` | `3` | OTLP export timeout |

### Local/Dev Collector

```bash
OPENPORTIO_OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:4317 \
OPENPORTIO_SERVICE_NAME=openportio-local \
OPENPORTIO_OTEL_TRACE_SAMPLE_RATIO=1.0 \
cargo run -p openportio-server
```

### Production Collector Example

```bash
OPENPORTIO_OTEL_EXPORTER_OTLP_ENDPOINT=http://otel-collector.observability.svc.cluster.local:4317 \
OPENPORTIO_SERVICE_NAME=openportio-api \
OPENPORTIO_OTEL_TRACE_SAMPLE_RATIO=0.2 \
OPENPORTIO_OTEL_EXPORTER_TIMEOUT_SECONDS=5 \
RUST_LOG=info,openportio_server=info,tower_http=warn \
cargo run -p openportio-server
```

Recommended production baseline:
- start with sample ratio `0.1` to `0.2`
- route traces/logs/metrics to the same environment-level collector
- alert on collector-unreachable startup failures in deployment pipeline

## Runtime Verification

Use preflight endpoint checks:

```bash
OPENPORTIO_PREFLIGHT_BOOT_SERVER=true ./scripts/prod_preflight.sh
```

This validates:
- `/health`
- `/openapi.json`
- `/grpc/contracts`
- `/metrics`

## Recommended External Integrations

- Centralized log sink (ELK, Loki, Cloud Logging)
- Metrics pipeline from Openportio `/metrics` scrape + infra/service metrics
- Alerting on:
  - health endpoint failures
  - auth failure spikes (401 / UNAUTHENTICATED trends)
  - latency and saturation indicators

## Troubleshooting

- `failed to initialize OTel tracing pipeline`: check `OPENPORTIO_OTEL_EXPORTER_OTLP_ENDPOINT` reachability and protocol (`grpc` on `4317`).
- `/metrics` returns 404: ensure you are hitting the configured `OPENPORTIO_METRICS_PATH`.
- missing `x-request-id`: confirm shared middleware is enabled (default `OpenportioServer::new()` path) and no custom layer removes the header.
