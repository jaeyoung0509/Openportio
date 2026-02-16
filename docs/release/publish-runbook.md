# crates.io Publish Runbook

This runbook documents the reproducible release flow for Openportio crates.

Rename context:
- Project name changed from `Meld` to `Openportio`.
- Runtime `MELD_*` env keys remain as compatibility aliases, but all docs/scripts/defaults use `OPENPORTIO_*`.

## Publish Targets

- `openportio-core`
- `openportio-macros`
- `openportio-rpc`
- `openportio-server`

Examples are intentionally non-publishable (`publish = false`).

## Pre-release Checklist

Run from repository root:

```bash
./scripts/check_release_metadata.sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
./scripts/check_contracts_bundle.sh
./scripts/prod_preflight.sh
./scripts/release_dry_run.sh
```

For a specific release tag candidate, run metadata validation with tag consistency:

```bash
OPENPORTIO_RELEASE_TAG=v0.1.0 ./scripts/check_release_metadata.sh
```

`scripts/release_dry_run.sh` does:
- executes `scripts/check_release_metadata.sh` as a mandatory first gate
- `cargo publish --dry-run` for all publishable crates:
  - `openportio-core`
  - `openportio-macros`
  - `openportio-rpc`
  - `openportio-server`
- applies local `patch.crates-io` overrides so dry-run can validate dependent crates
  before first crates.io index propagation.
- writes audit artifacts:
  - `target/release/metadata-summary.md`
  - `target/release/dry-run-summary.md`
  - `target/release/dry-run-logs/*.log`

## Publish Order

Use this order to respect dependency graph:

1. `openportio-core`
2. `openportio-macros`
3. `openportio-rpc`
4. `openportio-server`

Publish commands:

```bash
cargo publish -p openportio-core
cargo publish -p openportio-macros
cargo publish -p openportio-rpc
cargo publish -p openportio-server
```

## Enforced Release Sequence (Develop -> Main -> Tag)

This repository uses a strict release sequence:

1. Merge feature PRs into `develop`.
2. Open and merge a `develop -> main` release PR.
3. Create and push release tag from `main`.
4. Let `.github/workflows/release.yml` publish crates + GitHub release.

The release workflow is tag-driven and rejects tags not reachable from `main`.
It also enforces:
- metadata/tag/changelog consistency (`scripts/check_release_metadata.sh`)
- full quality gates
- publish dry-run gates (`scripts/release_dry_run.sh`)

Prerequisites:
- GitHub Actions secret: `CRATES_IO_TOKEN`
- `release` environment configured with required reviewer(s)
- protected `main` branch

Execution:

1. Merge `develop` into `main` through a PR.
2. Tag from `main` and push:

```bash
git checkout main
git pull --ff-only origin main
git tag v0.1.0
git push origin v0.1.0
```

3. GitHub Actions will:
- re-run release quality gates
- validate metadata/tag/changelog consistency early
- publish crates in dependency order (`openportio-core` -> `openportio-macros` -> `openportio-rpc` -> `openportio-server`)
- create/update GitHub release notes
- upload release preflight artifacts (`target/release/*.md`, dry-run logs)

The workflow rejects tags that are not reachable from `main`.

## First Release Candidate Tag Procedure

1. Ensure `develop` is green on CI and release checklist is complete.
2. Create release candidate tag:

```bash
git tag v0.1.0-rc.1
git push origin v0.1.0-rc.1
```

3. Validate crates.io dry-run one final time.
4. Publish crates in order above.
5. Create final stable tag once publish is confirmed:

```bash
git tag v0.1.0
git push origin v0.1.0
```

## Rollback / Mitigation

crates.io publishes are immutable, so rollback means forward-fix:

- If publish fails mid-sequence:
  - stop immediately
  - document exact failure in release notes
  - patch broken crate(s), bump version, rerun dry-run
- If already published crate has critical issue:
  - publish fixed patch version (`0.1.1`, etc.)
  - yank affected version if needed:

```bash
cargo yank --vers <version> <crate-name>
```
