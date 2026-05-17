## Help
##
## Orison developer Makefile. Targets are idempotent and safe to re-run.
## All targets shell out to existing tools (cargo, python3, ori) and never
## install anything implicitly.
##
## Quality gates (mirror .github/workflows/static.yml, test.yml, release.yml):
##   make gate-fast        Static gate only, no Rust toolchain required (<60s).
##   make gate-pre-commit  Static gate + cargo fmt --check + cargo check.
##   make gate-full        Full local equivalent of CI: static + fmt + clippy + test + CLI contracts.
##
## Build and release:
##   make release-build    cargo build --release -p ori. Produces target/release/ori.
##   make bench            Run ori bench in human form (release binary required; auto-builds).
##   make bench-json       Run ori bench --json and write BENCHMARKS.results.json.
##   make sbom             Run ori sbom --json --format ori-native and write sbom.json.
##   make audit            Run ori audit (workspace dependency audit).
##   make provenance-check Run ori provenance verify against examples/provenance.json if present.
##
## Developer workflow:
##   make check            cargo run -p ori -- check on examples/hello.ori.
##   make test             cargo test --workspace.
##   make fmt              cargo fmt --all (formats in place).
##   make fmt-check        cargo fmt --all --check.
##   make clippy           cargo clippy with -D warnings.
##   make doctor           ori doctor.
##   make agent-map        ori agent map on the canonical fixture.
##   make capsule          ori capsule on the canonical fixture.
##   make patch-check      ori patch check on the canonical fixture.
##   make validate-json    JSON/JSONL/schema contract gate.
##   make lsp-stdio        Launch ori lsp --stdio (interactive; blocking).
##   make docs-human       Emit human-readable module docs for examples/fullstack.
##   make docs-agent       Emit agent-budgeted module docs for examples/fullstack.
##   make migrate-plan     Run ori migrate --dry-run between bootstrap editions.
##   make db-check         Placeholder for future database-shape gate (see docs/CI.md).
##
## Repository hygiene:
##   make install-hooks    Wire .githooks/* into .git/hooks via core.hooksPath.
##   make uninstall-hooks  Unwire .githooks (resets core.hooksPath to default).
##   make help             Print this help block.
##
## See: CONTRIBUTING.md, BENCHMARKS.md, SECURITY.md, docs/ROADMAP.md,
##      docs/language/REFERENCE.md.

.PHONY: help \
        check test fmt fmt-check clippy doctor \
        agent-map capsule patch-check validate-json \
        static-gate pre-commit quality-gate \
        gate-fast gate-pre-commit gate-full \
        install-hooks uninstall-hooks \
        release-build bench bench-json sbom audit provenance-check \
        lsp-stdio docs-human docs-agent migrate-plan db-check

# ---------------------------------------------------------------------------
# Help (default goal)
# ---------------------------------------------------------------------------

.DEFAULT_GOAL := help

help:
	@awk '/^## / { sub(/^## ?/, ""); print }' Makefile

# ---------------------------------------------------------------------------
# Quality gates
# ---------------------------------------------------------------------------

# Legacy names kept for backward compatibility with existing scripts and docs.
static-gate: gate-fast
pre-commit: gate-pre-commit
quality-gate: gate-full

gate-fast:
	python3 scripts/validate_all.py --static-only

gate-pre-commit:
	python3 scripts/validate_all.py --pre-commit

gate-full:
	python3 scripts/validate_all.py --full

# ---------------------------------------------------------------------------
# Developer commands
# ---------------------------------------------------------------------------

check:
	cargo run -p ori -- check --json examples/hello.ori

test:
	cargo test --workspace

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all --check

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

doctor:
	cargo run -p ori -- doctor

agent-map:
	cargo run -p ori -- agent map --budget 2000 --json examples/fullstack/users.ori

capsule:
	cargo run -p ori -- capsule --json examples/fullstack/users.ori

patch-check:
	cargo run -p ori -- patch check --json examples/agent_patch.json

validate-json:
	python3 scripts/validate_all.py --contracts-only

lsp-stdio:
	cargo run -p ori -- lsp --stdio

docs-human:
	cargo run -p ori -- docs --format human examples/fullstack

docs-agent:
	cargo run -p ori -- docs --format agent --budget 2000 examples/fullstack

migrate-plan:
	cargo run -p ori -- migrate --from 0 --to 1 --dry-run --json examples/fullstack

# Placeholder. The database schema-shape gate is gated on the database
# subsystem (see docs/ROADMAP.md). Until then, this target prints a
# no-op message and exits 0 so CI can call it unconditionally. When the
# gate exists, replace the body with the real call.
db-check:
	@echo "db-check: not yet implemented; gated on database subsystem milestone"

# ---------------------------------------------------------------------------
# Release / supply chain
# ---------------------------------------------------------------------------

release-build:
	cargo build --release -p ori

bench: release-build
	target/release/ori bench --samples 50

bench-json: release-build
	target/release/ori bench --samples 50 --json > BENCHMARKS.results.json
	@echo "wrote BENCHMARKS.results.json"

sbom: release-build
	target/release/ori sbom --json --format ori-native > sbom.json
	@echo "wrote sbom.json"

audit:
	cargo run -p ori -- audit --json

# Verifies a sample provenance document if one exists. Safe no-op otherwise.
provenance-check:
	@if [ -f examples/provenance.json ]; then \
		cargo run -p ori -- provenance verify --json examples/provenance.json; \
	else \
		echo "provenance-check: examples/provenance.json not present; skipping"; \
	fi

# ---------------------------------------------------------------------------
# Hooks
# ---------------------------------------------------------------------------

install-hooks:
	./scripts/install_hooks.sh

uninstall-hooks:
	./scripts/uninstall_hooks.sh
