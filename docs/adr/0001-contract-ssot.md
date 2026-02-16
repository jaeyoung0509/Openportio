# ADR 0001: REST + gRPC Contract Source Of Truth

- Status: Accepted
- Date: 2026-02-16
- Owners: Openportio maintainers
- Related issue: #90

## Context

Openportio supports REST and gRPC in one runtime. As the framework grows, contract drift risk grows too:

- gRPC proto contracts can evolve separately from REST DTOs.
- Generated contract artifacts can diverge from implementation expectations.
- Follow-up work (error mapping automation, macro transparency, compatibility gates) needs a single contract baseline.

To keep FastAPI-like DX while preserving Rust-style correctness, we need one explicit source of truth for shared API contracts.

## Decision

Openportio adopts a **schema-first contract model** for shared REST + gRPC payloads:

- Authoritative schema: `crates/openportio-rpc/proto/*.proto`
- Authoritative service definitions: gRPC services in proto files
- Authoritative generated Rust types: `openportio-rpc` codegen outputs from proto

For payloads that are used by both REST and gRPC, proto is the source of truth.  
REST-only payloads can remain local Rust DTOs when they are not part of the shared external contract.

## Contract Ownership Boundaries

1. gRPC transport contract:
   - Owned by proto definitions in `crates/openportio-rpc/proto`.
2. Shared message contract (REST + gRPC):
   - Owned by proto definitions and generated Rust types.
3. REST-only validation/input ergonomics:
   - Owned by server-side DTOs/macros (`openportio-server`) where no shared proto contract is required.
4. Derived docs/artifacts:
   - Generated from source contracts and validated in CI.

## Required Generation And Validation Flow

1. Edit proto schema in `crates/openportio-rpc/proto`.
2. Regenerate/compile gRPC bindings via standard build/test flow.
3. Regenerate contract artifacts:
   - `./scripts/generate_contracts_bundle.sh`
4. Validate drift:
   - `./scripts/check_contracts_bundle.sh`
5. Run CI/local quality checks before merge:
   - `cargo test --workspace`
   - `./scripts/ci_local.sh` (recommended)

## Drift Policy

- Any change to proto or contract-generation logic must keep generated artifacts up to date.
- PRs that introduce artifact drift are blocked by CI.
- Breaking-change policy enforcement is handled by dedicated compatibility gate work (see follow-ups).

## Migration Impact

- Existing REST DTO ergonomics (`#[openportio_server::dto]`, trait-first validation) stay supported.
- Teams should avoid redefining shared wire contracts in parallel Rust structs when proto already defines them.
- Existing compatibility env aliases and runtime behavior are unaffected by this ADR.

## Alternatives Considered

### A) Code-first (`Rust -> proto`)

Pros:
- Rust-first authoring feels natural for framework users.

Cons:
- Harder to model gRPC-first semantics (service definitions, streaming, ecosystem tooling) as primary source.
- Higher risk of generated proto mismatch or lossy conversion.

Decision: Rejected for primary SSoT.

### B) Hybrid (mixed ownership)

Pros:
- Flexible per-team.

Cons:
- Ambiguous ownership and recurring drift disputes.
- Harder to enforce in CI and review policy.

Decision: Rejected due to governance overhead.

## Consequences

Positive:
- Clear contract ownership for teams and reviewers.
- Easier CI policy design for compatibility and drift gates.
- Stronger enterprise story for contract-driven integration.

Trade-offs:
- Proto evolution discipline becomes mandatory for shared contracts.
- Some DTO-heavy flows need adapter layers rather than free-form parallel modeling.

## Follow-up Work

- #91: declarative domain error mapping for REST + gRPC
- #92: observability defaults with optional OTel export
- #93: macro transparency and debug ergonomics
- #94: typed required-dependency guards for DI
- #87: explicit breaking-change compatibility gate in CI
