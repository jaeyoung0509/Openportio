# FastAPI-Like Builder API

Openportio exposes a fluent server builder for a compact startup flow.

## Quick Start

```rust
use std::net::SocketAddr;

use openportio_server::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    OpenportioServer::new()
        .with_addr(SocketAddr::from(([127, 0, 0, 1], 3000)))
        .run()
        .await?;
    Ok(())
}
```

`openportio_server::prelude::*` includes:
- `OpenportioServer`
- `route` and `dto` macros
- composable derive aliases:
  - `OpenPortIOValidate` / `OpenPortIOSchema`
  - `MeldValidate` / `MeldSchema` (compatibility aliases)
- common validation extractors (`ValidatedJson`, `ValidatedQuery`, `ValidatedPath`, `ValidatedParts`)
- trait-first validation contract: `RequestValidation`
- `Depends` DI extractor

## Common Customization Points

- `with_state(...)`: inject shared app state
- `with_rest_router(...)`: replace default REST router
- `merge_raw_router(...)`: merge a plain Axum router escape hatch
- `with_grpc_service(...)`: add typed gRPC service
- `configure_tonic(...)` / `configure_tonic_routes(...)`: transform tonic `Routes` before final merge
- `without_grpc()`: run REST-only mode
- `with_middleware_config(...)`: configure shared middleware
- `with_middleware(...)`: add custom router-level middleware
- `on_startup(...)` / `on_shutdown(...)`: attach lifecycle hooks

## Raw Escape Hatches

```rust
use axum::{routing::get, Router};
use tonic::service::Routes;
use openportio_server::OpenportioServer;

let app = OpenportioServer::new()
    .merge_raw_router(
        Router::new().route("/internal/metrics", get(|| async { "metrics-ok" })),
    )
    .configure_tonic(|routes| {
        let grpc_router = routes
            .into_axum_router()
            .route("/grpc-hook", get(|| async { "grpc-hook-ok" }));
        Routes::from(grpc_router)
    })
    .build_app();
```

Ordering guarantees:
- base REST router (`with_rest_router(...)` or default)
- raw router merges in call order
- gRPC routes
- shared middleware + dependency overrides
- final custom middleware chain (`with_middleware(...)`)

Supported / unsupported interactions:
- Supported: adding plain Axum routes (`/internal/*`) through `merge_raw_router(...)`.
- Supported: route-level gRPC router transformation through `configure_tonic(...)`.
- Not supported: full `tonic::transport::Server` tuning via this hook (for example transport-level HTTP/2 socket options).
- `configure_tonic(...)` is ignored when `without_grpc()` is set.
- Built-in shared metrics uses `/metrics` (or `OPENPORTIO_METRICS_PATH`), so avoid route collisions with that path unless you intentionally reconfigure it.

## DTO And Dependency Injection Pattern

Openportio supports three DTO styles:
- all-in-one `#[openportio_server::dto]`
- composable derives (`OpenPortIOValidate` / `OpenPortIOSchema`)
- trait-first validation via `RequestValidation`

Requirements:
- `utoipa` must be available in crate dependencies (for schema derive expansion)
- `validator` field attributes (such as `#[validate(...)]`) are supported directly for macro/derive-based validation

```rust
use std::sync::Arc;

use openportio_core::AppState;
use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::Serialize;

#[openportio_server::dto]
struct NotePath {
    id: String,
}

#[openportio_server::dto]
struct NoteQuery {
    q: Option<String>,
    limit: Option<u32>,
}

#[openportio_server::dto]
struct CreateNoteBody {
    title: String,
}

#[derive(
    openportio_server::serde::Deserialize,
    openportio_server::OpenPortIOValidate,
    openportio_server::OpenPortIOSchema
)]
struct ComposableCreateNoteBody {
    #[validate(length(min = 2, max = 120))]
    title: String,
}

#[derive(Serialize)]
struct NoteResponse {
    id: String,
    title: String,
}

async fn get_note(
    State(state): State<Arc<AppState>>,
    Path(path): Path<NotePath>,
) -> Json<NoteResponse> {
    let title = state.greet(&path.id).unwrap_or_else(|_| "fallback".to_string());
    Json(NoteResponse { id: path.id, title })
}
```

Trait-first validation example:

```rust
use axum::{Json, http::StatusCode};
use openportio_server::api::{
    ApiError, ApiErrorResponse, ApiValidationIssue, RequestValidation,
};

#[derive(openportio_server::serde::Deserialize)]
struct TraitFirstBody {
    title: String,
}

impl RequestValidation for TraitFirstBody {
    fn validate_request(&self, source: &'static str) -> Result<(), ApiError> {
        if self.title.starts_with("admin") {
            let issue = ApiValidationIssue {
                loc: vec![source.to_string(), "title".to_string()],
                msg: "admin-prefixed titles are reserved".to_string(),
                issue_type: "custom_validation".to_string(),
            };
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResponse::validation(
                    "request validation failed",
                    Some(vec![issue]),
                    None,
                )),
            ));
        }
        Ok(())
    }
}
```

Migration guidance:
- keep `#[dto]` when you want the shortest path
- switch to composable derives when you need explicit per-derive control
- use trait-first `RequestValidation` when validator derive cannot express your logic

See `/examples/simple-server/src/main.rs` for runnable end-to-end handler patterns.

## Validation And Error DTO Pattern

Openportio now includes reusable REST validation helpers:

- `openportio_server::api::ValidatedJson<T>`
- `openportio_server::api::ValidatedQuery<T>`
- `openportio_server::api::ApiErrorResponse`

Typical usage:

```rust
use openportio_server::api::{ApiError, ValidatedJson};
#[openportio_server::dto]
struct CreateNoteBody {
    #[validate(length(min = 2, max = 120))]
    title: String,
}

async fn create_note(
    ValidatedJson(body): ValidatedJson<CreateNoteBody>,
) -> Result<axum::Json<String>, ApiError> {
    Ok(axum::Json(body.title))
}
```

Validation failures return a structured `400` error JSON with:
- `code`
- `message`
- `detail` (FastAPI-like issue list with `loc`, `msg`, `type`)
- `details` (legacy field-level map kept for compatibility)

OpenAPI wiring:
- shared error schema uses `ApiErrorResponse`
- REST path annotations can reference the same response body for `400/401/500`

## Auto-Validate Route Macro (FastAPI-Like DX)

For a more FastAPI-like handler style, use `#[openportio_server::route(..., auto_validate)]`.

With `auto_validate`, handler arguments are rewritten at compile time:
- `Json<T>` -> `ValidatedJson<T>`
- `Query<T>` -> `ValidatedQuery<T>`
- `Path<T>` -> `ValidatedPath<T>`

Example:

```rust
use openportio_server::api::ApiError;
use axum::Json;

#[openportio_server::route(post, "/notes", auto_validate)]
async fn create_note(Json(body): Json<CreateNoteBody>) -> Result<Json<String>, ApiError> {
    Ok(Json(body.title))
}
```

If you omit `auto_validate`, behavior stays unchanged.

For header/cookie wrapper patterns, use `ValidatedParts<T>` with your custom parts extractor
that implements `Validate`.

Macro portability:
- `#[route(...)]` expansion is dependency-rename safe.
- Example compile coverage exists under `examples/openportio-app`.

## SSE Endpoint Pattern

Openportio supports Server-Sent Events (SSE) for lightweight one-way real-time updates.

```rust
use std::{convert::Infallible, time::Duration};

use axum::response::sse::{Event, KeepAlive, Sse};
use tokio_stream::{once, wrappers::IntervalStream, Stream, StreamExt};

async fn events() -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let initial = once(Ok(Event::default().event("heartbeat").data("ready")));
    let mut sequence = 0u64;
    let ticks = IntervalStream::new(tokio::time::interval(Duration::from_secs(2)))
        .map(move |_| {
            sequence += 1;
            Ok(Event::default()
                .event("message")
                .data(format!("tick-{sequence}")))
        });

    Sse::new(initial.chain(ticks)).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("heartbeat"),
    )
}
```

Client reconnect guidance:
- Use automatic reconnect with exponential backoff (for example 1s, 2s, 4s ... capped at 30s).
- Add small jitter to avoid synchronized reconnect spikes.
- Resume with `Last-Event-ID` when your client stack supports it.

## WebSocket Endpoint Pattern

Openportio supports WebSocket upgrade handlers for bidirectional realtime flows.

```rust
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use futures_util::SinkExt;

async fn ws_echo(ws: WebSocketUpgrade) -> impl axum::response::IntoResponse {
    ws.max_message_size(4096).on_upgrade(handle_ws)
}

async fn handle_ws(mut socket: WebSocket) {
    while let Some(Ok(Message::Text(text))) = socket.recv().await {
        let _ = socket.send(Message::Text(format!("echo: {text}"))).await;
    }
}
```

Server defaults in `openportio-server`:
- max text frame bytes: `OPENPORTIO_WS_MAX_TEXT_BYTES` (default `4096`)
- idle timeout seconds: `OPENPORTIO_WS_IDLE_TIMEOUT_SECS` (default `45`)

## OAuth2/OIDC JWT Auth Pattern

Openportio includes shared JWT claim validation used by both REST and gRPC layers.

Environment configuration:
- `OPENPORTIO_AUTH_ENABLED=true`
- choose one validation mode:
  - shared-secret mode: `OPENPORTIO_AUTH_JWT_SECRET=<hmac-secret>`
  - JWKS mode: `OPENPORTIO_AUTH_JWKS_URL=<https://issuer/.well-known/jwks.json>`
- optional JWKS tuning:
  - `OPENPORTIO_AUTH_JWKS_REFRESH_SECS=300`
  - `OPENPORTIO_AUTH_JWKS_ALGORITHMS=RS256,ES256`
- optional: `OPENPORTIO_AUTH_ISSUER=<issuer>`
- optional: `OPENPORTIO_AUTH_AUDIENCE=<audience>`
- if both secret and JWKS are set, runtime prefers JWKS mode.

When enabled:
- REST protected route example: `GET /protected/whoami` (Bearer token required)
- gRPC interceptor validates `authorization: Bearer <token>` metadata

When disabled:
- existing baseline routes continue without auth enforcement.

## Depends-Like Extractor Pattern

For FastAPI `Depends(...)` style injection, use `openportio_server::di::Depends<T>`.
`Depends<T>` resolves dependencies from state (`FromRef`) with request-scoped caching.

```rust
use std::sync::Arc;

use openportio_core::AppState;
use openportio_server::di::Depends;
use axum::extract::FromRef;

#[derive(Clone)]
struct ServiceInfo {
    service_name: String,
}

impl FromRef<Arc<AppState>> for ServiceInfo {
    fn from_ref(state: &Arc<AppState>) -> Self {
        Self {
            service_name: state.config.service_name.clone(),
        }
    }
}

async fn handler(Depends(info): Depends<ServiceInfo>) -> String {
    info.service_name
}
```

Test override helper:
- `openportio_server::di::with_dependency_override(router, value)`
- `openportio_server::di::with_dependency(router, value)`
- `openportio_server::di::with_dependency_overrides(router, overrides)`
- `OpenportioServer::with_dependency(value)`

## Notes

- Default `OpenportioServer::new()` enables both REST and gRPC on a single listener.
- Default address uses `OPENPORTIO_SERVER_ADDR` if set, otherwise `127.0.0.1:3000`.

## Single-Port vs Dual-Port

Default mode is single-port multiplexing:

```rust
OpenportioServer::new()
    .with_addr(([0, 0, 0, 0], 3000).into())
    .run()
    .await?;
```

Explicit dual-port mode:

```rust
OpenportioServer::new()
    .with_rest_addr(([0, 0, 0, 0], 3000).into())
    .with_grpc_addr(([0, 0, 0, 0], 50051).into())
    .run()
    .await?;
```

Operational guidance:
- Keep single-port for local development and simple edge deployments.
- Use dual-port when platform networking prefers explicit protocol separation (for example dedicated gRPC service ports, strict L4/L7 rules, or separate SLO tracking).
- Dual-port config is all-or-nothing: both `with_rest_addr(...)` and `with_grpc_addr(...)` must be provided.

## gRPC Quickstart (No-Auth + Auth)

Prerequisites:
- `grpcurl` for local gRPC calls
- `python3` for dev-token generation script

Start server (default no-auth):

```bash
cargo run -p openportio-server
```

No-auth smoke tests:

```bash
grpcurl -plaintext 127.0.0.1:3000 list
grpcurl -plaintext \
  -import-path crates/openportio-rpc/proto \
  -proto service.proto \
  -d '{"name":"Rust"}' \
  127.0.0.1:3000 \
  openportio.v1.Greeter/SayHello
```

Enable auth (restart server):

```bash
OPENPORTIO_AUTH_ENABLED=true \
OPENPORTIO_AUTH_JWT_SECRET=dev-secret \
OPENPORTIO_AUTH_ISSUER=https://issuer.local \
OPENPORTIO_AUTH_AUDIENCE=openportio-api \
cargo run -p openportio-server
```

Expected failure without token:

```bash
grpcurl -plaintext \
  -import-path crates/openportio-rpc/proto \
  -proto service.proto \
  -d '{"name":"Rust"}' \
  127.0.0.1:3000 \
  openportio.v1.Greeter/SayHello
```

Expected outcome:
- gRPC status code: `UNAUTHENTICATED`
- Typical message: `missing bearer token`

Generate a development token (dev-only helper):

```bash
TOKEN=$(python3 scripts/generate_dev_jwt.py \
  --secret dev-secret \
  --issuer https://issuer.local \
  --audience openportio-api)
```

Expected success with token:

```bash
grpcurl -plaintext \
  -H "authorization: Bearer ${TOKEN}" \
  -import-path crates/openportio-rpc/proto \
  -proto service.proto \
  -d '{"name":"Rust"}' \
  127.0.0.1:3000 \
  openportio.v1.Greeter/SayHello
```

Common auth mismatch outcomes:
- wrong `--secret` / `OPENPORTIO_AUTH_JWT_SECRET`: `UNAUTHENTICATED`
- wrong `--issuer` / `OPENPORTIO_AUTH_ISSUER`: `UNAUTHENTICATED`
- wrong `--audience` / `OPENPORTIO_AUTH_AUDIENCE`: `UNAUTHENTICATED`
- unknown `kid` (JWKS mode): `UNAUTHENTICATED`
- unreachable `OPENPORTIO_AUTH_JWKS_URL`: `INTERNAL`
- malformed JWKS payload: `INTERNAL`
