#!/usr/bin/env python3
"""Repository validation gate for Orison.

This script is intentionally dependency-light. It performs static repository,
contract, source-guardrail, hook, and optional Rust quality checks. It is used by
CI, Makefile targets, and Git hooks.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tomllib
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
ERRORS: list[str] = []
WARNINGS: list[str] = []

REQUIRED_FILES = [
    "README.md",
    "CHANGELOG.md",
    "CONTRIBUTING.md",
    "SECURITY.md",
    "BENCHMARKS.md",
    "LICENSE",
    "Cargo.toml",
    "rust-toolchain.toml",
    "ori.toml",
    "Makefile",
    ".editorconfig",
    ".github/workflows/ci.yml",
    "scripts/validate_all.py",
    "scripts/check_json_contracts.sh",
    "scripts/install_hooks.sh",
    ".githooks/pre-commit",
    ".githooks/pre-push",
]

REQUIRED_DIRS = [
    "crates/ori-compiler/src",
    "crates/ori-agent/src",
    "crates/ori-cli/src",
    "docs/language",
    "docs/compiler",
    "docs/frameworks",
    "docs/stdlib",
    "schemas",
    "examples",
    "tests/golden",
    "scripts",
]

SCHEMA_MAP = {
    "examples/agent_patch.json": "schemas/patch.schema.json",
    "examples/change_manifest.json": "schemas/change.schema.json",
}

GOLDEN_JSONL_SCHEMA_BY_DIR = {
    "tests/golden/diagnostics": "schemas/diagnostic.schema.json",
}

ALLOWED_WORKSPACE_DEPS = {"serde", "serde_json"}
FORBIDDEN_SOURCE_PATTERNS = [
    (re.compile(r"\.(unwrap|expect)\s*\("), "Do not use unwrap()/expect() in production Rust source."),
    (re.compile(r"\bpanic!\s*\("), "Do not use panic! in production Rust source."),
    (re.compile(r"\btodo!\s*\("), "Do not leave todo! in production Rust source."),
    (re.compile(r"\bunimplemented!\s*\("), "Do not leave unimplemented! in production Rust source."),
    (re.compile(r"\bdbg!\s*\("), "Do not leave dbg! in production Rust source."),
    (re.compile(r"\bunsafe\s+(fn|impl|trait|\{)"), "Unsafe Rust is not allowed in the bootstrap compiler without an explicit approved exception."),
]


def rel(path: Path) -> str:
    return str(path.relative_to(ROOT))


def error(message: str) -> None:
    ERRORS.append(message)


def warn(message: str) -> None:
    WARNINGS.append(message)


def read_text(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except UnicodeDecodeError as exc:
        error(f"{rel(path)} is not valid UTF-8: {exc}")
        return ""


def load_json(path: Path) -> Any | None:
    try:
        return json.loads(read_text(path))
    except json.JSONDecodeError as exc:
        error(f"{rel(path)} is not valid JSON: line {exc.lineno}, column {exc.colno}: {exc.msg}")
        return None


def check_required_layout() -> None:
    for item in REQUIRED_FILES:
        path = ROOT / item
        if not path.is_file():
            error(f"missing required file: {item}")
        elif path.stat().st_size == 0:
            error(f"required file is empty: {item}")
    for item in REQUIRED_DIRS:
        path = ROOT / item
        if not path.is_dir():
            error(f"missing required directory: {item}")


def check_markdown_contracts() -> None:
    required_headings = {
        "README.md": ["#"],
        "CHANGELOG.md": ["#"],
        "CONTRIBUTING.md": ["#"],
        "SECURITY.md": ["#"],
        "BENCHMARKS.md": ["#"],
    }
    for file_name, headings in required_headings.items():
        path = ROOT / file_name
        if not path.exists():
            continue
        text = read_text(path)
        for heading in headings:
            if heading not in text:
                error(f"{file_name} is missing required heading/content marker: {heading}")


def check_json_contracts() -> None:
    schema_paths = sorted((ROOT / "schemas").glob("*.json"))
    for schema_path in schema_paths:
        schema = load_json(schema_path)
        if not isinstance(schema, dict):
            continue
        if schema.get("$schema") != "https://json-schema.org/draft/2020-12/schema":
            error(f"{rel(schema_path)} must declare Draft 2020-12 JSON Schema")
        if "title" not in schema:
            warn(f"{rel(schema_path)} has no title")
        if "type" not in schema:
            warn(f"{rel(schema_path)} has no root type")

    for json_path in sorted((ROOT / "examples").glob("*.json")):
        load_json(json_path)

    for jsonl_path in sorted((ROOT / "tests").glob("**/*.jsonl")):
        lines = read_text(jsonl_path).splitlines()
        for line_number, line in enumerate(lines, start=1):
            if not line.strip():
                continue
            try:
                json.loads(line)
            except json.JSONDecodeError as exc:
                error(
                    f"{rel(jsonl_path)} line {line_number} is not valid JSONL: "
                    f"column {exc.colno}: {exc.msg}"
                )

    validate_with_optional_jsonschema()


def validate_with_optional_jsonschema() -> None:
    try:
        import jsonschema  # type: ignore
    except Exception:
        warn("jsonschema package is not installed; schema-instance validation was skipped")
        return

    for instance_rel, schema_rel in SCHEMA_MAP.items():
        instance_path = ROOT / instance_rel
        schema_path = ROOT / schema_rel
        if not instance_path.exists() or not schema_path.exists():
            continue
        instance = load_json(instance_path)
        schema = load_json(schema_path)
        if instance is None or schema is None:
            continue
        try:
            jsonschema.Draft202012Validator.check_schema(schema)
            jsonschema.Draft202012Validator(schema).validate(instance)
        except jsonschema.ValidationError as exc:  # type: ignore[attr-defined]
            error(f"{instance_rel} does not validate against {schema_rel}: {exc.message}")
        except jsonschema.SchemaError as exc:  # type: ignore[attr-defined]
            error(f"{schema_rel} is not a valid Draft 2020-12 schema: {exc.message}")

    for dir_rel, schema_rel in GOLDEN_JSONL_SCHEMA_BY_DIR.items():
        schema_path = ROOT / schema_rel
        if not schema_path.exists():
            continue
        schema = load_json(schema_path)
        if schema is None:
            continue
        try:
            validator = jsonschema.Draft202012Validator(schema)
            jsonschema.Draft202012Validator.check_schema(schema)
        except jsonschema.SchemaError as exc:  # type: ignore[attr-defined]
            error(f"{schema_rel} is not a valid Draft 2020-12 schema: {exc.message}")
            continue
        for jsonl_path in sorted((ROOT / dir_rel).glob("*.jsonl")):
            for line_number, line in enumerate(read_text(jsonl_path).splitlines(), start=1):
                if not line.strip():
                    continue
                try:
                    validator.validate(json.loads(line))
                except jsonschema.ValidationError as exc:  # type: ignore[attr-defined]
                    error(f"{rel(jsonl_path)} line {line_number} violates {schema_rel}: {exc.message}")


def check_shell_scripts_and_hooks() -> None:
    shell_files = list((ROOT / "scripts").glob("*.sh"))
    shell_files += [ROOT / ".githooks/pre-commit", ROOT / ".githooks/pre-push"]
    for path in shell_files:
        if not path.exists():
            error(f"missing shell file: {rel(path)}")
            continue
        text = read_text(path)
        if not text.startswith("#!/usr/bin/env bash"):
            error(f"{rel(path)} must use '#!/usr/bin/env bash'")
        if "set -euo pipefail" not in text:
            error(f"{rel(path)} must enable 'set -euo pipefail'")
        mode = path.stat().st_mode
        if not (mode & stat.S_IXUSR):
            error(f"{rel(path)} must be executable")
        result = subprocess.run(["bash", "-n", str(path)], cwd=ROOT, text=True, capture_output=True)
        if result.returncode != 0:
            error(f"{rel(path)} fails bash -n: {result.stderr.strip()}")

    pre_commit = ROOT / ".githooks/pre-commit"
    pre_push = ROOT / ".githooks/pre-push"
    if pre_commit.exists() and "validate_all.py --pre-commit" not in read_text(pre_commit):
        error(".githooks/pre-commit must call scripts/validate_all.py --pre-commit")
    if pre_push.exists() and "validate_all.py --full" not in read_text(pre_push):
        error(".githooks/pre-push must call scripts/validate_all.py --full")


def check_rust_source_guardrails() -> None:
    for path in sorted((ROOT / "crates").glob("*/src/**/*.rs")):
        text = read_text(path)
        for pattern, message in FORBIDDEN_SOURCE_PATTERNS:
            match = pattern.search(text)
            if match:
                error(f"{rel(path)} violates source guardrail: {message} near `{match.group(0)}`")

    workspace_manifest = ROOT / "Cargo.toml"
    if workspace_manifest.exists():
        try:
            manifest = tomllib.loads(read_text(workspace_manifest))
        except tomllib.TOMLDecodeError as exc:
            error(f"Cargo.toml is not valid TOML: {exc}")
            return
        deps = set((manifest.get("workspace", {}).get("dependencies", {}) or {}).keys())
        unapproved = deps - ALLOWED_WORKSPACE_DEPS
        if unapproved:
            error(
                "workspace dependencies require explicit approval in MEMORY.md and CHANGELOG.md: "
                + ", ".join(sorted(unapproved))
            )


def run_command(command: list[str], *, label: str) -> None:
    print(f"[validate] {label}: {' '.join(command)}")
    result = subprocess.run(command, cwd=ROOT, text=True)
    if result.returncode != 0:
        error(f"command failed for {label}: {' '.join(command)}")


def run_rust_gate(mode: str) -> None:
    cargo = shutil.which("cargo")
    if cargo is None:
        error("cargo is required for this validation mode; install Rust or run --static-only for archive-only validation")
        return

    if mode in {"pre-commit", "full"}:
        run_command([cargo, "fmt", "--all", "--check"], label="rustfmt")
        run_command([cargo, "check", "--workspace", "--all-targets"], label="cargo check")

    if mode == "full":
        run_command([cargo, "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"], label="clippy")
        run_command([cargo, "test", "--workspace"], label="tests")
        dynamic_commands = [
            [cargo, "run", "-p", "ori", "--", "doctor"],
            [cargo, "run", "-p", "ori", "--", "check", "--json", "examples/hello.ori"],
            [cargo, "run", "-p", "ori", "--", "agent", "map", "--budget", "2000", "--json", "examples/fullstack/users.ori"],
            [cargo, "run", "-p", "ori", "--", "agent", "explain", "sym:store.users.fetch_user", "--json", "examples/fullstack/users.ori"],
            [cargo, "run", "-p", "ori", "--", "capsule", "--json", "examples/fullstack/users.ori"],
            [cargo, "run", "-p", "ori", "--", "patch", "check", "--json", "examples/agent_patch.json"],
        ]
        for command in dynamic_commands:
            run_command(command, label="cli contract")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Validate Orison repository quality gates")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--static-only", action="store_true", help="run static repository/contract/guardrail checks only")
    mode.add_argument("--contracts-only", action="store_true", help="run JSON/schema contract checks only")
    mode.add_argument("--pre-commit", action="store_true", help="run pre-commit gate")
    mode.add_argument("--full", action="store_true", help="run full quality gate")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.contracts_only:
        check_json_contracts()
    else:
        check_required_layout()
        check_markdown_contracts()
        check_json_contracts()
        check_shell_scripts_and_hooks()
        check_rust_source_guardrails()
        if args.pre_commit:
            run_rust_gate("pre-commit")
        elif args.full or not args.static_only:
            run_rust_gate("full")

    for warning in WARNINGS:
        print(f"warning: {warning}", file=sys.stderr)
    if ERRORS:
        for item in ERRORS:
            print(f"error: {item}", file=sys.stderr)
        return 1
    print("validation passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
