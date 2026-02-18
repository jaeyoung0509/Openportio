---
layout: home

hero:
  name: Openportio
  text: Docs-First REST + gRPC For Rust
  tagline: Launch REST (HTTP/1.1) and gRPC (HTTP/2) with a focused builder API and contract artifacts.
  image:
    src: /openportio-logo.svg
    alt: Openportio
  actions:
    - theme: brand
      text: Get Started
      link: /getting-started
    - theme: alt
      text: View GitHub
      link: https://github.com/jaeyoung0509/Openportio

features:
  - title: Single-Port Or Dual-Port
    details: Run REST + gRPC on one listener by default, then split ports explicitly when infrastructure requires it.
  - title: FastAPI-Like DX
    details: Route macros, DTO ergonomics, validation extractors, and trait-first escape hatches for complex logic.
  - title: Contract-Driven Delivery
    details: OpenAPI + gRPC contract artifacts with drift checks for safer team integrations.
  - title: Production Failure Drills
    details: Auth, dependency outage, timeout, and readiness-degradation drills with reproducible runbooks.
---

## Docs-Only Navigation

This portal is intentionally focused on documentation only.
Use the top navigation to jump straight to setup, runtime guides, DX patterns, and contract references.

Recommended quick path:
- Runtime topology: `/guides/rest-grpc`
- REST authoring DX: `/guides/rest-fastapi-dx`
- gRPC authoring DX: `/guides/grpc-fastapi-dx`

## Source Mapping

Openportio keeps deep technical markdown in the repository and surfaces curated entry points here.
Key source docs include:
- [`README.md`](https://github.com/jaeyoung0509/Openportio/blob/develop/README.md)
- [`docs/fastapi-like-builder.md`](https://github.com/jaeyoung0509/Openportio/blob/develop/docs/fastapi-like-builder.md)
- [`docs/production/deployment.md`](https://github.com/jaeyoung0509/Openportio/blob/develop/docs/production/deployment.md)
- [`docs/dx-scorecard.md`](https://github.com/jaeyoung0509/Openportio/blob/develop/docs/dx-scorecard.md)
- [`docs/adr/0001-contract-ssot.md`](https://github.com/jaeyoung0509/Openportio/blob/develop/docs/adr/0001-contract-ssot.md)
