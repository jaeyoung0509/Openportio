# Contract Compatibility Policy

Openportio uses a contract compatibility gate in PR CI to block backward-incompatible REST/gRPC changes before merge.

## Scope

Compatibility is checked against the target base branch contract bundle (`docs/generated/contracts-bundle.json`).

Gate command:

```bash
./scripts/check_contract_compatibility.sh
```

Core checker:

```bash
python3 scripts/check_contract_compatibility.py --base-ref origin/develop
```

## What Is Considered Breaking

The current gate treats the following as breaking:

- REST operation removal:
  - `rest.operation.removed:<operation_id>`
- REST operation signature change (method/path changed for same `operation_id`):
  - `rest.operation.signature_changed:<operation_id>`
- gRPC method removal:
  - `grpc.method.removed:<package.Service/Method>`
- gRPC method shape change in generated bridge contract:
  - `grpc.method.shape_changed:<package.Service/Method>`
  - checked fields: `http_method`, `path`, `request_schema_ref`, `response_schema_ref`
- REST<->gRPC contract mapping link removal:
  - `contracts.link.removed:<rest_operation_id>-><grpc_method>`

These are conservative-by-default because they can break existing clients and integration contracts.

## Non-Breaking (Examples)

- adding a new REST operation
- adding a new gRPC method
- adding metadata/docs fields without changing existing method/path/schema references

## Intentional Breaks: Controlled Waiver Process

When a break is intentional, add a temporary waiver in:

- `contracts/compat_exceptions.toml`

Template:

```toml
[[exceptions]]
id = "rest.operation.removed:hello"
reason = "Intentional removal tracked in issue #123 with migration notes."
expires_on = "2026-12-31"
```

Rules:

- `id` must exactly match CI output.
- `reason` is required and should reference migration context (issue/PR/runbook).
- `expires_on` is optional but recommended.
- expired waivers are treated as failures.

The checker prints unused waivers as warnings so stale exceptions can be removed.

## CI Wiring

- Workflow: `.github/workflows/ci.yml`
- Required PR job: `Contract Compatibility Gate`
- The job fetches the PR base branch and compares current bundle against that baseline.
