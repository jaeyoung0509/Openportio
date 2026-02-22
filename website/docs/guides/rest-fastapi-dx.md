# REST FastAPI-like DX

## Goal

This guide focuses on the REST developer experience in Openportio:
- minimal handler boilerplate
- structured validation
- typed dependency injection
- predictable auth and error behavior

## Quick REST Runtime

```rust
use openportio_server::OpenportioServer;

OpenportioServer::new()
    .with_addr(([127, 0, 0, 1], 3000).into())
    .run()
    .await?;
```

Core REST endpoints from default router:
- `GET /health`
- `GET /hello/:name`
- `GET /protected/whoami`
- `GET /events` (SSE)
- `GET /ws` (WebSocket upgrade)

## Handler Pattern

```rust
use axum::Json;
use openportio_server::api::{ApiError, ValidatedJson};

#[openportio_server::dto]
struct CreateNoteBody {
    #[validate(length(min = 2, max = 120))]
    title: String,
}

#[openportio_server::route(post, "/notes", auto_validate, transparent)]
async fn create_note(
    ValidatedJson(body): ValidatedJson<CreateNoteBody>,
) -> Result<Json<String>, ApiError> {
    Ok(Json(body.title))
}
```

Recommended mode:
- use `auto_validate, transparent`
- keep extractor types explicit in handler signature
- avoid hidden macro rewrites

## DTO Modes

1. All-in-one macro:
- `#[openportio_server::dto]`

2. Composable derive mode:
- `Deserialize + OpenPortIOValidate + OpenPortIOSchema`

3. Trait-first mode:
- implement `RequestValidation` directly for custom rules

## Dependency Injection

REST DI uses `Depends<T>` with request-scoped caching.

```rust
use axum::{extract::FromRef, Json};
use openportio_server::di::Depends;

#[derive(Clone)]
struct ServiceLabel(String);

impl FromRef<std::sync::Arc<openportio_core::AppState>> for ServiceLabel {
    fn from_ref(state: &std::sync::Arc<openportio_core::AppState>) -> Self {
        Self(state.config.service_name.clone())
    }
}

async fn read_label(Depends(label): Depends<ServiceLabel>) -> Json<String> {
    Json(label.0)
}
```

## Auth Behavior

`/protected/whoami` is the REST auth probe endpoint.

- auth disabled (`OPENPORTIO_AUTH_ENABLED=false`):
  - returns `200` with anonymous principal
- auth enabled:
  - missing/invalid bearer token returns `401`
  - valid token returns current principal

## Error Model

REST validation and domain failures map into a stable JSON shape:

```json
{
  "code": "validation_error",
  "message": "request validation failed",
  "detail": [
    { "loc": ["body", "title"], "msg": "length", "type": "length" }
  ]
}
```

Status mapping summary:
- validation failure: `400`
- auth failure: `401`
- internal domain failure: `500` (sanitized message)

## Smoke Checks

```bash
curl -s http://127.0.0.1:3000/health
curl -s http://127.0.0.1:3000/hello/Rust
curl -s http://127.0.0.1:3000/protected/whoami
curl -s http://127.0.0.1:3000/openapi.json
```

## Deep References

- [`docs/fastapi-like-builder.md`](https://github.com/jaeyoung0509/Openportio/blob/develop/docs/fastapi-like-builder.md)
- [`examples/simple-server/README.md`](https://github.com/jaeyoung0509/Openportio/blob/develop/examples/simple-server/README.md)
- [`examples/simple-server/src/main.rs`](https://github.com/jaeyoung0509/Openportio/blob/develop/examples/simple-server/src/main.rs)
- [`crates/openportio-server/src/api.rs`](https://github.com/jaeyoung0509/Openportio/blob/develop/crates/openportio-server/src/api.rs)
