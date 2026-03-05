# CI Workflow

This project uses focused CI jobs so failures are clearly scoped:

1. `Core Build And Test`
- `cargo check --workspace`
- `cargo test --workspace`
- `cargo test -p production-api -- --nocapture`

2. `Nextest`
- `cargo nextest run --workspace --all-targets`

3. `Coverage`
- `cargo llvm-cov --workspace --summary-only`
- `cargo llvm-cov --workspace --lcov --output-path target/coverage/lcov.info`
- uploads `coverage-report` artifact (summary + lcov)

4. `REST gRPC E2E`
- `cargo test -p openportio-server --test multiplexing -- --nocapture`

5. `Example Smoke Gate`
- `./scripts/example_smoke.sh`
- validates `simple-server` and `production-api` (REST + gRPC + auth fail/success)

6. `Docs Contract Drift Check`
- `./scripts/check_contracts_bundle.sh`
- `cargo test -p openportio-server openapi_json_is_available -- --nocapture`

7. `Contract Compatibility Gate`
- `./scripts/check_contract_compatibility.sh`
- compares current `docs/generated/contracts-bundle.json` with base branch bundle
- fails on backward-incompatible REST/gRPC contract changes

8. `Security Audit`
- `cargo audit`

9. `Production Preflight`
- `./scripts/prod_preflight.sh` with secure-mode CI environment

10. `Release Dry Run`
- `./scripts/release_dry_run.sh`

11. `Perf Regression Gate` (manual workflow)
- workflow: `.github/workflows/perf.yml`
- trigger: `workflow_dispatch`
- runs REST (`k6`) + gRPC (`ghz`) perf smoke with threshold enforcement
- uploads artifact: `perf-report`

12. `Docs Site`
- workflow: `.github/workflows/docs-site.yml`
- validates docs links, builds VitePress, verifies `sitemap.xml` + `robots.txt`, deploys GitHub Pages, then runs post-deploy smoke check

## Local Equivalent

Run:

```bash
./scripts/ci_local.sh
```

This runs the same command set as CI in a single local flow.

Run example smoke gate separately:

```bash
./scripts/example_smoke.sh
```

For dedicated nextest + coverage quality gates, run:

```bash
./scripts/test_quality.sh
```

See `docs/testing-toolchain.md` for installation and outputs.

For local perf regression gate, run:

```bash
./scripts/perf_gate.sh
```

See `docs/performance-gates.md` for thresholds and tuning guidance.

For docs-site failure triage and rollback/retry, see:

```bash
docs/release/docs-site-runbook.md
```
