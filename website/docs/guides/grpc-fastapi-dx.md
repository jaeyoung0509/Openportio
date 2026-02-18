# gRPC FastAPI-like DX

## Function-Style gRPC Registration

Use `with_grpc_say_hello(...)` when you want a FastAPI-like handler flow without tonic trait boilerplate.

```rust
use std::sync::Arc;

use openportio_core::{AppState, OpenportioError};
use openportio_server::{
    grpc::{GrpcHelloRequest, GrpcHelloResponse},
    OpenportioServer,
};

async fn say_hello(
    state: Arc<AppState>,
    request: GrpcHelloRequest,
) -> Result<GrpcHelloResponse, OpenportioError> {
    let message = state.greet(&request.name)?;
    Ok(GrpcHelloResponse { message })
}

OpenportioServer::new()
    .with_state(Arc::new(AppState::local("grpc-dx")))
    .with_grpc_say_hello(say_hello)
    .run()
    .await?;
```

## Context Mode: Validation + DI + Auth Principal

Use `with_grpc_say_hello_with_context(...)` when handler code needs typed dependencies and
authenticated principal access in one place.

```rust
use std::sync::Arc;

use openportio_core::{AppState, OpenportioError};
use openportio_server::{
    grpc::{
        validated_grpc_request, GrpcHandlerContext, GrpcHelloRequest, GrpcHelloResponse,
    },
    OpenportioServer,
};

#[derive(openportio_server::serde::Deserialize, openportio_server::OpenPortIOValidate)]
struct GrpcInput {
    #[validate(length(min = 1))]
    name: String,
}

#[derive(Clone)]
struct ServiceLabel(String);

impl axum::extract::FromRef<Arc<AppState>> for ServiceLabel {
    fn from_ref(state: &Arc<AppState>) -> Self {
        Self(state.config.service_name.clone())
    }
}

async fn say_hello(
    ctx: GrpcHandlerContext,
    request: GrpcHelloRequest,
) -> Result<GrpcHelloResponse, OpenportioError> {
    let input = validated_grpc_request(GrpcInput { name: request.name })?;
    let label = ctx.depends::<ServiceLabel>();
    let base = ctx.state().greet(&input.name)?;
    Ok(GrpcHelloResponse {
        message: format!("[{}:{}] {}", label.0, ctx.principal().subject, base),
    })
}

OpenportioServer::new()
    .with_grpc_say_hello_with_context(say_hello)
    .run()
    .await?;
```

## Why This Exists

- Keep startup and handler code close to FastAPI-style ergonomics.
- Preserve typed request/response contracts.
- Keep auth interceptor and single-port runtime behavior unchanged.

## Escape Hatches (Still Supported)

- `with_grpc_service(...)`: register raw tonic service directly.
- `configure_tonic(...)`: transform tonic `Routes` before final merge.

Choose these when you need custom tonic internals or advanced service composition.

## Migration: tonic Trait -> Function Handler

Before:
- implement `openportio_rpc::Greeter` for a service struct
- wrap with `GreeterServer::new(...)`
- register through `with_grpc_service(...)`

After:
- define one async function:
  - `async fn(Arc<AppState>, GrpcHelloRequest) -> Result<GrpcHelloResponse, OpenportioError>`
- register with `with_grpc_say_hello(...)`

## Runtime Checks

No-auth smoke check:

```bash
grpcurl -plaintext 127.0.0.1:3000 list
grpcurl -plaintext \
  -import-path crates/openportio-rpc/proto \
  -proto service.proto \
  -d '{"name":"Rust"}' \
  127.0.0.1:3000 \
  openportio.v1.Greeter/SayHello
```

Auth behavior is unchanged:
- missing/invalid token in auth-enabled mode -> `UNAUTHENTICATED`
- valid token -> normal handler response

## REST vs gRPC Mapping Cheatsheet

- Validation failure:
  - REST: `400` (`validation_error`)
  - gRPC: `INVALID_ARGUMENT`
- Auth failure:
  - REST: `401`
  - gRPC: `UNAUTHENTICATED`
- Internal domain error:
  - REST: `500` + sanitized message
  - gRPC: `INTERNAL` + sanitized message

## Deep References

- [`docs/fastapi-like-builder.md`](https://github.com/jaeyoung0509/Openportio/blob/develop/docs/fastapi-like-builder.md)
- [`examples/simple-server/src/main.rs`](https://github.com/jaeyoung0509/Openportio/blob/develop/examples/simple-server/src/main.rs)
- [`crates/openportio-server/src/grpc.rs`](https://github.com/jaeyoung0509/Openportio/blob/develop/crates/openportio-server/src/grpc.rs)
