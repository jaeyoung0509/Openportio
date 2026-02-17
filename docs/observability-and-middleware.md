# Shared Middleware And Observability

This project applies one centralized middleware stack for both REST and gRPC traffic.

Source:
- `crates/openportio-server/src/middleware.rs`

Included layers:
- `TraceLayer` for structured request tracing
- `SetRequestIdLayer` to generate `x-request-id` when missing
- `PropagateRequestIdLayer` to echo request ID in responses
- request metrics capture (total/in-flight/status class/duration sum)
- `CorsLayer` with permissive origin policy (for REST/browser integration)
- `TimeoutLayer` for request timeout boundaries
- `ConcurrencyLimitLayer` for in-flight request control

Environment variables:
- `OPENPORTIO_TIMEOUT_SECONDS` (default: `15`)
- `OPENPORTIO_MAX_IN_FLIGHT_REQUESTS` (default: `1024`)
- `OPENPORTIO_REQUEST_BODY_LIMIT_BYTES` (default: `1048576`)
- `OPENPORTIO_METRICS_PATH` (default: `/metrics`)
- `OPENPORTIO_SERVICE_NAME` (default: `openportio-server`)
- `OPENPORTIO_OTEL_EXPORTER_OTLP_ENDPOINT` (optional, disabled by default)
- `OPENPORTIO_OTEL_TRACE_SAMPLE_RATIO` (default: `1.0`)
- `OPENPORTIO_OTEL_EXPORTER_TIMEOUT_SECONDS` (default: `3`)

Notes:
- Middleware is applied in `crates/openportio-server/src/main.rs`.
- Because the app is multiplexed (REST + gRPC on one listener), these layers are shared by both protocol paths.
- Built-in metrics output is Prometheus text format on `/metrics` (or custom path from env).
