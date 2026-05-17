# Chapter 01: Install

**What you'll build.** A working `ori` binary on your `PATH`, a green
`ori doctor` health report, and a verified release toolchain. By the end of this
chapter you will be able to invoke every CLI subcommand referenced in the rest of
the tutorial.

**Time:** ~10 minutes (the release build dominates).

## 1. Install Rust 1.92

Orison pins its toolchain in [`rust-toolchain.toml`](../../rust-toolchain.toml).
`cargo` will read that file automatically once you have any working `rustup`
installation. If you do not yet have one:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

Verify the toolchain:

```bash
rustc --version
# rustc 1.92.0 (ded5c06cf 2025-12-08)
cargo --version
# cargo 1.92.0 (344c4567c 2025-10-21)
```

If `rustc` reports a different version, run `rustup show` inside the repository
once you have cloned it — the pinned override will install 1.92 automatically on
first use.

## 2. Clone the repository

```bash
git clone https://github.com/Eldergenix/Orison.git
cd Orison
```

You should see five workspace crates and the supporting tree:

```bash
ls -F
# BENCHMARKS.md          README.md       crates/         schemas/
# BENCHMARKS.results.json SECURITY.md    docs/           scripts/
# CHANGELOG.md           Cargo.lock      examples/       stdlib/
# CONTRIBUTING.md        Cargo.toml      ori.toml        target/
# GOAL.md                LICENSE         rust-toolchain.toml tests/
# Makefile               CONTRIBUTING.md
```

Workspace crates:

```bash
ls crates/
# ori-agent  ori-cli  ori-compiler  ori-lsp  ori-pkg
```

## 3. Install the git hooks (optional but recommended)

```bash
./scripts/install_hooks.sh
```

The hooks call [`scripts/validate_all.py`](../../scripts/validate_all.py) on
pre-commit and pre-push. They prevent you from committing source that violates
the bootstrap's source guardrails (no `unwrap`, no `panic!`, no untracked
workspace deps). Skipping this step is fine for a read-only tutorial run.

## 4. Build the CLI

```bash
cargo build --release -p ori
```

Cold build takes about two minutes on Apple Silicon. The binary lands at
`target/release/ori`.

For convenience, alias the binary for the rest of the tutorial:

```bash
alias ori="$PWD/target/release/ori"
```

Or copy it onto your `PATH`:

```bash
install target/release/ori ~/.local/bin/ori
```

## 5. Run `ori doctor`

`ori doctor` is the self-check. It reports the compiler version, the loaded
schemas, and a summary of capability enforcement.

```bash
ori doctor
```

You will see a single JSON object on stdout. Pretty-printed via `jq`:

```bash
ori doctor | jq .
```

```json
{
  "schema": "ori.doctor.v1",
  "status": "ok",
  "compiler": "bootstrap",
  "language": "Orison",
  "version": "0.1.1",
  "rust_toolchain": "stable-aarch64-apple-darwin",
  "checks": [
    { "name": "schemas_published", "status": "ok", "detail": "34 stable contracts" },
    { "name": "compiler_modules",  "status": "ok", "detail": "lexer, parser, cst, resolver, ..." }
  ],
  "capabilities_summary": [
    "package-level effect policy enforced statically via `ori capability --policy ...`",
    "bootstrap compiler does not yet enforce capability runtime"
  ],
  "schema_versions": {
    "agent_changed":   "ori.agent_changed.v1",
    "agent_diagnose":  "ori.agent_diagnose.v1",
    "diagnostic":      "ori.diagnostic.v1",
    "doctor":          "ori.doctor.v1",
    "patch":           "ori.patch.v1"
  }
}
```

If `status` is anything other than `"ok"`, stop and read the failing entry under
`checks`. A common failure is a stale `target/` after a toolchain upgrade — run
`cargo clean -p ori-compiler -p ori-cli && cargo build --release -p ori`.

## 6. Run a sanity check on a known-good source file

```bash
ori check examples/hello.ori
```

Expected output:

```
ok: hello
```

Now do the same with the JSON envelope variant — every chapter from here on uses
`--json`, so you may as well see the shape now:

```bash
ori check --json examples/hello.ori
echo "exit=$?"
```

Successful checks print nothing to stdout and exit with code 0:

```
exit=0
```

A file with a forbidden construct prints one JSON diagnostic per line and exits
non-zero. The repository ships an example you can use to verify this without
writing anything yourself:

```bash
ori check --json examples/bad_null.ori | jq .
```

```json
{
  "schema": "ori.diagnostic.v1",
  "id": "E0100",
  "level": "error",
  "message": "`null` is not part of Orison; use Option[T]",
  "span": {
    "file": "examples/bad_null.ori",
    "start": { "line": 4, "column": 14 },
    "end":   { "line": 4, "column": 18 }
  },
  "expected": ["Option[T]", "None", "Some(value)"],
  "found":    ["null"],
  "fixes": [
    {
      "kind": "replace_null",
      "description": "Replace `null` with `None` or an explicit Option value.",
      "confidence": 0.82
    }
  ],
  "agent": {
    "summary": "Replace null with Option semantics.",
    "minimal_context": [],
    "docs": ["doc:types.option"]
  }
}
```

That envelope conforms to
[`schemas/diagnostic.schema.json`](../../schemas/diagnostic.schema.json) — every
tool that reads `ori check --json` can parse the same fields.

## 7. Verify the JSON contract gate

The repository's full quality gate is a single Python script. Run it now so you
catch any environment problems before chapter 02:

```bash
python3.13 scripts/validate_all.py --static-only
```

Expected last line:

```
validation passed
```

The `--full` variant additionally invokes `cargo fmt --check`, `cargo clippy`,
`cargo test --workspace`, and several `cargo run -p ori -- ...` smoke checks.
You can run it now if you want maximum confidence; it adds about three minutes:

```bash
python3.13 scripts/validate_all.py --full
```

## Common errors

| Diagnostic       | Likely cause                                                       | Fix                                                                              |
|------------------|--------------------------------------------------------------------|----------------------------------------------------------------------------------|
| `error: linker 'cc' not found` during `cargo build` | No system C toolchain                                       | Install build essentials. On macOS: `xcode-select --install`. On Debian: `apt install build-essential`. |
| `ori: command not found`                            | Alias not set or `target/release/ori` not on `PATH`         | Re-run the `alias` line from step 4 or copy the binary into `~/.local/bin`.        |
| `rustc 1.91.x` or older                             | An older default toolchain shadowed the override            | `cd Orison && rustup show` then `rustup install 1.92` if it asks.                  |
| `ori doctor` returns `status: "warn"` for `schemas_published` | Schema file deleted or renamed in your local tree | `git status schemas/` and restore missing files; the bootstrap expects all 34.   |
| `python3.13: command not found`                     | Older Python installed                                       | The validator targets 3.13 specifically. Install with `pyenv install 3.13.0` or your distro's package manager. |

## Recap

- You installed Rust 1.92, cloned the repository, and built `ori` in release mode.
- You confirmed the binary is on your `PATH` and that `ori doctor` reports
  `status: "ok"`.
- You saw a successful `ori check` produce no stdout and exit 0.
- You saw a failing `ori check --json` produce a single `ori.diagnostic.v1` JSON
  object (here, `E0100` for `null`) and exit non-zero.
- You ran `scripts/validate_all.py --static-only` and saw `validation passed`.

## Next

Continue with [chapter 02: Hello world](./02-hello-world.md), where you will
write your first `.ori` module, run it, and read the `ori.run.v1` envelope.

For deeper context while you work, keep
[`docs/language/REFERENCE.md`](../language/REFERENCE.md) open in a second tab.
