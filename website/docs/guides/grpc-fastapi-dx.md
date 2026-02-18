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

## Deep References

- [`docs/fastapi-like-builder.md`](https://github.com/jaeyoung0509/Openportio/blob/develop/docs/fastapi-like-builder.md)
- [`examples/simple-server/src/main.rs`](https://github.com/jaeyoung0509/Openportio/blob/develop/examples/simple-server/src/main.rs)
- [`crates/openportio-server/src/grpc.rs`](https://github.com/jaeyoung0509/Openportio/blob/develop/crates/openportio-server/src/grpc.rs)
