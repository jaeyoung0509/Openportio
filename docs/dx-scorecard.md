# DX Upgrade Scorecard (Openportio vs FastAPI-like Workflows)

This document tracks the developer-experience upgrade delivered for issue `#48`.

## 1) Validation DTO Boilerplate

Before:

```rust
#[derive(serde::Deserialize, validator::Validate, utoipa::ToSchema)]
struct CreateNoteBody {
    #[validate(length(min = 2, max = 120))]
    title: String,
}
```

After:

```rust
#[openportio_server::dto]
struct CreateNoteBody {
    #[validate(length(min = 2, max = 120))]
    title: String,
}
```

Result:
- DTO annotation reduced to one attribute
- Serde decode + validator + OpenAPI schema derive are applied together

## 2) Validation Error Shape

Openportio now returns validation failures with stable fields inspired by FastAPI-style payloads:

```json
{
  "code": "validation_error",
  "message": "request validation failed",
  "detail": [
    { "loc": ["body", "title"], "msg": "length", "type": "length" }
  ]
}
```

Result:
- Client-side field-level parsing is straightforward (`detail[*].loc/msg/type`)
- Existing `code/message` structure remains stable

## 3) OpenAPI Boilerplate

Openportio OpenAPI generation now auto-injects shared error responses:
- `400` (validation / bad request)
- `500` (internal error)
- `401` for `/protected/*` routes

Result:
- Less per-handler response annotation boilerplate
- Uniform docs for error models across endpoints

## 4) DI Ergonomics

Added provider-style override utilities:
- `OpenportioServer::with_dependency(value)`
- `openportio_server::di::with_dependency(...)`
- `openportio_server::di::with_dependency_overrides(...)`

Result:
- Cleaner test-time wiring for multiple dependencies
- Request-scoped dependency caching still guaranteed

## 5) Error Mapping Policy

REST and gRPC now share one domain-error mapping policy:
- Validation errors preserve actionable messages
- Internal errors are sanitized for clients and fully logged on server side

Mapping is now declared once in `openportio-core` and reused by both transports.
Current declaration pattern:

```rust
impl_domain_error_mapping!(OpenportioError {
    Validation => {
        rest_status: BadRequest,
        rest_code: "validation_error",
        grpc_code: InvalidArgument,
        expose_message: true,
        issue_type: Some("domain_validation")
    },
    Internal => {
        rest_status: InternalServerError,
        rest_code: "internal_error",
        grpc_code: Internal,
        expose_message: false,
        issue_type: None
    }
});
```

Result:
- Safer external error surface
- Better observability without leaking internals

## 6) Composable DTO + Trait-First Escape Hatch

Openportio now supports explicit DTO composition without the all-in-one macro:

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

For advanced validation logic that cannot be expressed with `validator` attributes, DTOs can implement:
- `openportio_server::api::RequestValidation`

Result:
- All-in-one convenience (`#[dto]`) remains intact
- Advanced teams get trait-first control without abandoning Openportio extractors
