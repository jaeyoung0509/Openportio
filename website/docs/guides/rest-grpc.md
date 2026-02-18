# REST + gRPC Runtime

## Default: Single-Port Multiplexing

Openportio serves REST and gRPC from one listener by default.

```rust
use openportio_server::OpenportioServer;

OpenportioServer::new().run().await?;
```

## Explicit Dual-Port Mode

When infra policy or traffic isolation requires split listeners:

```rust
use openportio_server::OpenportioServer;

OpenportioServer::new()
    .with_rest_addr(([0, 0, 0, 0], 3000).into())
    .with_grpc_addr(([0, 0, 0, 0], 50051).into())
    .run()
    .await?;
```

## Operational Guidance

- Use single-port for fast local development and simple deployments.
- Use dual-port for strict ingress routing, separate service policies, or clear protocol boundaries.
- Keep readiness checks on REST endpoint and gRPC contract checks in CI.

## Deep References

- [`REST FastAPI-like DX guide`](/guides/rest-fastapi-dx)
- [`gRPC FastAPI-like DX guide`](/guides/grpc-fastapi-dx)
- [`README.md` dual-port section](https://github.com/jaeyoung0509/Openportio/blob/develop/README.md)
- [`docs/production/deployment.md`](https://github.com/jaeyoung0509/Openportio/blob/develop/docs/production/deployment.md)
- [`crates/openportio-server/tests/multiplexing.rs`](https://github.com/jaeyoung0509/Openportio/blob/develop/crates/openportio-server/tests/multiplexing.rs)
