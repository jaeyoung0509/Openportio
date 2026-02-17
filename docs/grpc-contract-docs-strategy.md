# gRPC Contract Docs Strategy

## Goal

Provide human-readable and Swagger-compatible discovery docs for gRPC contracts defined in `.proto`.

## Approaches Evaluated

1. Proto-first docs portal (descriptor/reflection)
- Pros: closest to gRPC source-of-truth
- Cons: less familiar for teams expecting Swagger/OpenAPI UX

2. gRPC-gateway/OpenAPI facade
- Pros: native OpenAPI output for HTTP facade
- Cons: adds gateway runtime and can drift from pure gRPC behavior

3. Buf/protoc plugin docs generation
- Pros: rich ecosystem and mature tooling
- Cons: extra toolchain management and onboarding cost

## Chosen Strategy (Canonical For Openportio)

Use a descriptor-based generator that emits:
- Markdown contract docs (`docs/generated/grpc-contracts.md`)
- OpenAPI-compatible bridge JSON (`docs/generated/grpc-openapi-bridge.json`)

Why this choice:
- Keeps `.proto` as source-of-truth
- Parses protobuf descriptors instead of regex, making complex proto syntax safer to handle
- Produces browsable docs without introducing an HTTP gateway runtime
- Works with current Openportio stack and can run in CI as a deterministic check

## Tooling

- Generator: `scripts/generate_grpc_contract_docs.sh` (calls `cargo run -p openportio-rpc --bin grpc-docgen`)
- Bundled generator flow: `scripts/generate_contracts_bundle.sh`
- Drift check used in CI: `scripts/check_contracts_bundle.sh`
- Compatibility gate used in PR CI: `scripts/check_contract_compatibility.sh`

Reproducible flow:
1. Run `./scripts/generate_contracts_bundle.sh`
2. Commit updated artifacts under `docs/generated/`
3. In CI, run `scripts/check_contracts_bundle.sh` and fail on drift
4. In PR CI, run `scripts/check_contract_compatibility.sh` and fail on breaking changes

## Published Paths

- Artifact paths:
  - `docs/generated/grpc-contracts.md`
  - `docs/generated/grpc-openapi-bridge.json`
  - `docs/generated/rest-openapi.json`
  - `docs/generated/contracts-bundle.json`
- Runtime endpoints:
  - `GET /grpc/contracts` (rendered HTML)
  - `GET /grpc/contracts.md` (raw markdown)
  - `GET /grpc/contracts/openapi.json`

## Limitations

- OpenAPI bridge is contract-discovery oriented, not a drop-in REST execution spec.
- Protobuf coverage is broader than regex parsing, but this is still not a full HTTP transcoding layer.
