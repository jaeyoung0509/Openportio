#!/usr/bin/env python3
"""Check REST+gRPC contract compatibility against a base git ref."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - fallback for older local Python
    import tomli as tomllib  # type: ignore[no-redef]


@dataclass(frozen=True)
class BreakingChange:
    change_id: str
    message: str
    why_breaking: str


@dataclass(frozen=True)
class Waiver:
    change_id: str
    reason: str
    expires_on: dt.date | None


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Compare docs/generated/contracts-bundle.json against a base git ref and fail on "
            "backward-incompatible REST/gRPC contract changes."
        )
    )
    parser.add_argument(
        "--base-ref",
        default="origin/develop",
        help="Git ref used as compatibility baseline (default: origin/develop)",
    )
    parser.add_argument(
        "--head-bundle",
        default="docs/generated/contracts-bundle.json",
        help="Bundle path for current branch (repo-relative)",
    )
    parser.add_argument(
        "--base-bundle",
        default="docs/generated/contracts-bundle.json",
        help="Bundle path read from base ref via git show (repo-relative)",
    )
    parser.add_argument(
        "--exceptions",
        default="contracts/compat_exceptions.toml",
        help="Compatibility waiver config path (repo-relative)",
    )
    return parser.parse_args()


def repo_root() -> Path:
    return Path(__file__).resolve().parent.parent


def ensure_repo_relative_path(raw: str, root: Path, *, arg_name: str) -> Path:
    path = Path(raw)
    if path.is_absolute():
        raise ValueError(f"{arg_name} must be a repository-relative path, got absolute path")

    resolved = (root / path).resolve()
    try:
        resolved.relative_to(root)
    except ValueError as exc:
        raise ValueError(f"{arg_name} escapes repository root: {raw}") from exc
    return resolved


def load_json_file(path: Path) -> dict[str, Any]:
    with path.open("r", encoding="utf-8") as handle:
        payload = json.load(handle)
    if not isinstance(payload, dict):
        raise ValueError(f"{path} must be a JSON object")
    return payload


def load_json_from_git(base_ref: str, repo_relative_path: str) -> dict[str, Any]:
    git_path = f"{base_ref}:{repo_relative_path}"
    process = subprocess.run(
        ["git", "show", git_path],
        check=False,
        capture_output=True,
        text=True,
    )
    if process.returncode != 0:
        stderr = process.stderr.strip() or "unknown git show error"
        raise RuntimeError(
            f"failed to read `{repo_relative_path}` from `{base_ref}` via git show: {stderr}"
        )
    try:
        payload = json.loads(process.stdout)
    except json.JSONDecodeError as exc:
        raise RuntimeError(
            f"{repo_relative_path} at {base_ref} is not valid JSON: {exc}"
        ) from exc
    if not isinstance(payload, dict):
        raise RuntimeError(f"{repo_relative_path} at {base_ref} must be a JSON object")
    return payload


def rest_operations(bundle: dict[str, Any]) -> dict[str, dict[str, str]]:
    rest = bundle.get("rest")
    if not isinstance(rest, dict):
        raise ValueError("bundle is missing object field: rest")

    operations = rest.get("operations")
    if not isinstance(operations, list):
        raise ValueError("bundle is missing list field: rest.operations")

    result: dict[str, dict[str, str]] = {}
    for index, operation in enumerate(operations):
        if not isinstance(operation, dict):
            raise ValueError(f"rest.operations[{index}] must be an object")
        operation_id = operation.get("operation_id")
        method = operation.get("method")
        path = operation.get("path")
        if not isinstance(operation_id, str) or not operation_id:
            raise ValueError(f"rest.operations[{index}] has invalid operation_id")
        if not isinstance(method, str) or not method:
            raise ValueError(f"rest.operations[{index}] has invalid method")
        if not isinstance(path, str) or not path:
            raise ValueError(f"rest.operations[{index}] has invalid path")
        if operation_id in result:
            raise ValueError(f"duplicate rest operation_id in bundle: {operation_id}")
        result[operation_id] = {"method": method, "path": path}
    return result


def grpc_methods(bundle: dict[str, Any]) -> dict[str, dict[str, str | None]]:
    grpc = bundle.get("grpc")
    if not isinstance(grpc, dict):
        raise ValueError("bundle is missing object field: grpc")

    methods = grpc.get("methods")
    if not isinstance(methods, list):
        raise ValueError("bundle is missing list field: grpc.methods")

    result: dict[str, dict[str, str | None]] = {}
    for index, method_entry in enumerate(methods):
        if not isinstance(method_entry, dict):
            raise ValueError(f"grpc.methods[{index}] must be an object")
        method = method_entry.get("grpc_method")
        if not isinstance(method, str) or not method:
            raise ValueError(f"grpc.methods[{index}] has invalid grpc_method")
        if method in result:
            raise ValueError(f"duplicate grpc method in bundle: {method}")
        result[method] = {
            "http_method": as_opt_string(method_entry.get("http_method")),
            "path": as_opt_string(method_entry.get("path")),
            "request_schema_ref": as_opt_string(method_entry.get("request_schema_ref")),
            "response_schema_ref": as_opt_string(method_entry.get("response_schema_ref")),
        }
    return result


def contract_links(bundle: dict[str, Any]) -> set[tuple[str, str]]:
    links = bundle.get("links")
    if not isinstance(links, list):
        raise ValueError("bundle is missing list field: links")

    result: set[tuple[str, str]] = set()
    for index, link in enumerate(links):
        if not isinstance(link, dict):
            raise ValueError(f"links[{index}] must be an object")
        rest_operation_id = link.get("rest_operation_id")
        grpc_method = link.get("grpc_method")
        if not isinstance(rest_operation_id, str) or not rest_operation_id:
            raise ValueError(f"links[{index}] has invalid rest_operation_id")
        if not isinstance(grpc_method, str) or not grpc_method:
            raise ValueError(f"links[{index}] has invalid grpc_method")
        result.add((rest_operation_id, grpc_method))
    return result


def as_opt_string(value: Any) -> str | None:
    return value if isinstance(value, str) and value else None


def compare_rest(
    base_ops: dict[str, dict[str, str]], head_ops: dict[str, dict[str, str]]
) -> list[BreakingChange]:
    changes: list[BreakingChange] = []

    removed_ids = sorted(set(base_ops) - set(head_ops))
    for operation_id in removed_ids:
        old = base_ops[operation_id]
        changes.append(
            BreakingChange(
                change_id=f"rest.operation.removed:{operation_id}",
                message=(
                    f"REST operation `{operation_id}` was removed "
                    f"({old['method']} {old['path']})."
                ),
                why_breaking=(
                    "Existing REST clients that call this operation can fail immediately."
                ),
            )
        )

    shared_ids = sorted(set(base_ops) & set(head_ops))
    for operation_id in shared_ids:
        old = base_ops[operation_id]
        new = head_ops[operation_id]
        diffs: list[str] = []
        if old["method"] != new["method"]:
            diffs.append(f"method `{old['method']}` -> `{new['method']}`")
        if old["path"] != new["path"]:
            diffs.append(f"path `{old['path']}` -> `{new['path']}`")
        if diffs:
            changes.append(
                BreakingChange(
                    change_id=f"rest.operation.signature_changed:{operation_id}",
                    message=(
                        f"REST operation `{operation_id}` changed signature: "
                        + ", ".join(diffs)
                    ),
                    why_breaking=(
                        "Clients generated or implemented against the previous route contract "
                        "may no longer interoperate."
                    ),
                )
            )

    return changes


def compare_grpc(
    base_methods: dict[str, dict[str, str | None]],
    head_methods: dict[str, dict[str, str | None]],
) -> list[BreakingChange]:
    changes: list[BreakingChange] = []

    removed = sorted(set(base_methods) - set(head_methods))
    for method in removed:
        old = base_methods[method]
        changes.append(
            BreakingChange(
                change_id=f"grpc.method.removed:{method}",
                message=(
                    f"gRPC method `{method}` was removed "
                    f"({old['http_method']} {old['path']})."
                ),
                why_breaking=(
                    "gRPC clients compiled against the previous service definition can fail with "
                    "UNIMPLEMENTED or routing errors."
                ),
            )
        )

    shared = sorted(set(base_methods) & set(head_methods))
    for method in shared:
        old = base_methods[method]
        new = head_methods[method]
        changed_fields: list[str] = []
        for field in (
            "http_method",
            "path",
            "request_schema_ref",
            "response_schema_ref",
        ):
            if old[field] != new[field]:
                changed_fields.append(f"{field}: `{old[field]}` -> `{new[field]}`")
        if changed_fields:
            changes.append(
                BreakingChange(
                    change_id=f"grpc.method.shape_changed:{method}",
                    message=(
                        f"gRPC method `{method}` changed bridge shape: "
                        + ", ".join(changed_fields)
                    ),
                    why_breaking=(
                        "Request/response wire contract changes can break existing gRPC clients "
                        "and generated SDK assumptions."
                    ),
                )
            )

    return changes


def compare_links(
    base_links: set[tuple[str, str]], head_links: set[tuple[str, str]]
) -> list[BreakingChange]:
    changes: list[BreakingChange] = []
    removed_links = sorted(base_links - head_links)
    for rest_operation_id, grpc_method in removed_links:
        changes.append(
            BreakingChange(
                change_id=f"contracts.link.removed:{rest_operation_id}->{grpc_method}",
                message=(
                    "REST<->gRPC mapping link was removed: "
                    f"`{rest_operation_id}` -> `{grpc_method}`."
                ),
                why_breaking=(
                    "Contract consumers relying on unified mapping/discovery can lose a previously "
                    "supported interoperability path."
                ),
            )
        )
    return changes


def parse_waivers(path: Path) -> dict[str, Waiver]:
    if not path.exists():
        return {}

    with path.open("rb") as handle:
        payload = tomllib.load(handle)

    if not isinstance(payload, dict):
        raise ValueError(f"{path} must be a TOML table")

    raw_exceptions = payload.get("exceptions", [])
    if not isinstance(raw_exceptions, list):
        raise ValueError(f"{path}: `exceptions` must be an array of tables")

    waivers: dict[str, Waiver] = {}
    for index, raw in enumerate(raw_exceptions):
        if not isinstance(raw, dict):
            raise ValueError(f"{path}: exceptions[{index}] must be a table")

        change_id = raw.get("id")
        reason = raw.get("reason")
        raw_expires_on = raw.get("expires_on")

        if not isinstance(change_id, str) or not change_id:
            raise ValueError(f"{path}: exceptions[{index}].id must be a non-empty string")
        if not isinstance(reason, str) or not reason.strip():
            raise ValueError(
                f"{path}: exceptions[{index}].reason must be a non-empty string"
            )

        expires_on: dt.date | None = None
        if raw_expires_on is not None:
            if not isinstance(raw_expires_on, str):
                raise ValueError(
                    f"{path}: exceptions[{index}].expires_on must be YYYY-MM-DD string"
                )
            try:
                expires_on = dt.date.fromisoformat(raw_expires_on)
            except ValueError as exc:
                raise ValueError(
                    f"{path}: exceptions[{index}].expires_on must be YYYY-MM-DD"
                ) from exc

        if change_id in waivers:
            raise ValueError(f"{path}: duplicate waiver id `{change_id}`")

        waivers[change_id] = Waiver(
            change_id=change_id, reason=reason.strip(), expires_on=expires_on
        )
    return waivers


def print_breaking(changes: list[BreakingChange], waivers: dict[str, Waiver]) -> int:
    today = dt.date.today()
    active_ids: set[str] = set()
    unwaived: list[BreakingChange] = []

    for change in changes:
        waiver = waivers.get(change.change_id)
        if waiver is None:
            print(f"[BREAKING] {change.change_id}")
            print(f"  - {change.message}")
            print(f"  - Why: {change.why_breaking}")
            unwaived.append(change)
            continue

        if waiver.expires_on is not None and waiver.expires_on < today:
            print(f"[BREAKING][EXPIRED WAIVER] {change.change_id}")
            print(f"  - {change.message}")
            print(f"  - Why: {change.why_breaking}")
            print(
                "  - Waiver expired on "
                f"{waiver.expires_on.isoformat()}: {waiver.reason}"
            )
            unwaived.append(change)
            continue

        active_ids.add(change.change_id)
        expiry_text = waiver.expires_on.isoformat() if waiver.expires_on else "none"
        print(f"[WAIVED] {change.change_id}")
        print(f"  - {change.message}")
        print(f"  - Waiver reason: {waiver.reason}")
        print(f"  - Waiver expires_on: {expiry_text}")

    stale_waivers = sorted(set(waivers) - active_ids)
    for waiver_id in stale_waivers:
        waiver = waivers[waiver_id]
        expiry_text = waiver.expires_on.isoformat() if waiver.expires_on else "none"
        print(
            "[WARN] unused waiver entry: "
            f"{waiver_id} (reason={waiver.reason}, expires_on={expiry_text})"
        )

    return 1 if unwaived else 0


def main() -> int:
    args = parse_args()
    root = repo_root()

    head_bundle_path = ensure_repo_relative_path(
        args.head_bundle, root, arg_name="--head-bundle"
    )
    base_bundle_path = ensure_repo_relative_path(
        args.base_bundle, root, arg_name="--base-bundle"
    )
    exceptions_path = ensure_repo_relative_path(
        args.exceptions, root, arg_name="--exceptions"
    )

    if not head_bundle_path.exists():
        raise RuntimeError(
            f"head bundle does not exist: {head_bundle_path} "
            "(run ./scripts/generate_contracts_bundle.sh first)"
        )

    head_bundle = load_json_file(head_bundle_path)
    base_bundle = load_json_from_git(
        args.base_ref,
        str(base_bundle_path.relative_to(root).as_posix()),
    )
    waivers = parse_waivers(exceptions_path)

    changes: list[BreakingChange] = []
    changes.extend(compare_rest(rest_operations(base_bundle), rest_operations(head_bundle)))
    changes.extend(compare_grpc(grpc_methods(base_bundle), grpc_methods(head_bundle)))
    changes.extend(compare_links(contract_links(base_bundle), contract_links(head_bundle)))

    if not changes:
        print(
            f"[PASS] No breaking contract changes detected against `{args.base_ref}` "
            f"using `{base_bundle_path.relative_to(root)}`."
        )
        if waivers:
            unused = ", ".join(sorted(waivers))
            print(f"[WARN] waiver file has entries but no current breakages: {unused}")
        return 0

    print(f"Detected {len(changes)} potential breaking contract change(s).")
    print(
        "If a break is intentional, add a temporary waiver entry to "
        "`contracts/compat_exceptions.toml` with reason and expires_on."
    )
    return print_breaking(changes, waivers)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as err:  # pragma: no cover - top-level guard
        print(f"[FAIL] contract compatibility check failed: {err}", file=sys.stderr)
        raise SystemExit(1)
