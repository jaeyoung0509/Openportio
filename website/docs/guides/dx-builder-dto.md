# Builder, DTO, And Validation DX

## Builder API

Openportio builder keeps startup explicit and concise:

```rust
use openportio_server::OpenportioServer;

OpenportioServer::new()
    .with_addr(([127, 0, 0, 1], 3000).into())
    .run()
    .await?;
```

## DTO Modes

### 1) All-In-One Macro

```rust
#[openportio_server::dto]
struct CreateNoteBody {
    #[validate(length(min = 2, max = 120))]
    title: String,
}
```

### 2) Composable Derives

```rust
#[derive(
    openportio_server::serde::Deserialize,
    openportio_server::OpenPortIOValidate,
    openportio_server::OpenPortIOSchema
)]
struct CreateNoteBody {
    #[validate(length(min = 2, max = 120))]
    title: String,
}
```

### 3) Trait-First Escape Hatch

Implement `openportio_server::api::RequestValidation` when validation logic needs custom behavior.

## Extractor Pattern

- `ValidatedJson<T>`
- `ValidatedQuery<T>`
- `ValidatedPath<T>`
- `ValidatedParts<T>`

These extractors enforce validation before handler logic runs.

## Deep References

- [`REST FastAPI-like DX`](/guides/rest-fastapi-dx)
- [`gRPC FastAPI-like DX`](/guides/grpc-fastapi-dx)
- [`docs/fastapi-like-builder.md`](https://github.com/jaeyoung0509/Openportio/blob/develop/docs/fastapi-like-builder.md)
- [`docs/dx-scorecard.md`](https://github.com/jaeyoung0509/Openportio/blob/develop/docs/dx-scorecard.md)
- [`crates/openportio-server/tests/dto_modes.rs`](https://github.com/jaeyoung0509/Openportio/blob/develop/crates/openportio-server/tests/dto_modes.rs)
