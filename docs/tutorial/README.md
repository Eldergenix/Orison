# Orison Tutorial

A step-by-step walkthrough that takes a new developer from `git clone` to a running
demo storefront in about one hour. Each chapter is self-contained: read it, run the
commands, observe the JSON output, then move on.

The tutorial targets the **bootstrap / alpha** compiler shipped in this repository.
Behaviour described here is what `ori check`, `ori capsule`, `ori run`, `ori openapi`,
`ori ui`, `ori capability`, `ori db check`, `ori patch`, `ori test`, `ori coverage`,
`ori bench`, and `ori build` actually do today. For the *intended* full language see
[`docs/language/SPECIFICATION.md`](../language/SPECIFICATION.md); for what the
bootstrap actually parses see [`docs/language/REFERENCE.md`](../language/REFERENCE.md).

## Prerequisites

| Requirement       | Version                                                       |
|-------------------|---------------------------------------------------------------|
| Rust toolchain    | 1.92 (pinned in [`rust-toolchain.toml`](../../rust-toolchain.toml)) |
| Python            | 3.13 for [`scripts/validate_all.py`](../../scripts/validate_all.py) |
| `jq` (optional)   | Pretty-prints the JSON envelopes shown in every chapter        |
| Shell             | `bash` or `zsh`; commands assume POSIX                         |

You will need ~150 MB free for the `target/` directory and roughly two minutes for
the first release build. Subsequent builds are incremental.

## Chapters

| #  | Chapter                                                           | Time        |
|----|-------------------------------------------------------------------|-------------|
| 01 | [Install](./01-install.md)                                        | ~10 minutes |
| 02 | [Hello world](./02-hello-world.md)                                | ~5 minutes  |
| 03 | [Types: newtypes, records, variants, Option, Result](./03-types.md) | ~10 minutes |
| 04 | [Effects and capabilities](./04-effects.md)                       | ~10 minutes |
| 05 | [Functions and services](./05-functions-and-services.md)          | ~5 minutes  |
| 06 | [Views and UI](./06-views-and-ui.md)                              | ~5 minutes  |
| 07 | [Queries and migrations](./07-queries-and-migrations.md)          | ~5 minutes  |
| 08 | [Patches and agents](./08-patches-and-agents.md)                  | ~10 minutes |
| 09 | [Testing and benchmarks](./09-testing-and-benchmarks.md)          | ~5 minutes  |
| 10 | [Shipping the demo storefront](./10-shipping-the-demo-storefront.md) | ~10 minutes |

Total: ~75 minutes if you run every command verbatim. Skim-reading without running
commands takes around 30 minutes.

## Conventions

Every chapter follows the same shape:

- **What you'll build** — a one-paragraph statement of intent.
- **Time** — the estimate above.
- **Steps** — numbered prose interleaved with shell blocks and `.ori` source.
- **Common errors** — JSON diagnostic snippets keyed by ID.
- **Recap** — three to five bullets summarising what you just did.
- **Cross-links** — pointers to the next chapter and to the language reference.

All shell blocks assume your working directory is the repository root and that
`ori` is on your `PATH` (chapter 01 sets up an alias if you prefer). JSON output
shown in the tutorial is the literal stdout of the corresponding command; line
breaks have been inserted for readability but the value sequence is unchanged.

The tutorial does not use colour, emoji, or marketing language. Diagnostics are
documented by ID (`E0100`, `E0410`, `Q0010`, `P1010`, ...) so you can search the
compiler source and the cross-references in
[`docs/compiler/DIAGNOSTICS.md`](../compiler/DIAGNOSTICS.md).

## Reference material

- [`CHEATSHEET.md`](./CHEATSHEET.md) — one-page summary of every CLI subcommand,
  every diagnostic ID prefix, and every keyword. Print it and keep it nearby.
- [`docs/language/REFERENCE.md`](../language/REFERENCE.md) — the authoritative
  list of syntax the bootstrap parser recognises.
- [`docs/language/SPECIFICATION.md`](../language/SPECIFICATION.md) — the long-form
  intended language. Use this when the tutorial says "future" or "wave 2".
- [`docs/compiler/DIAGNOSTICS.md`](../compiler/DIAGNOSTICS.md) — diagnostic
  envelope shape and code family conventions.
- [`docs/compiler/PATCH_IR.md`](../compiler/PATCH_IR.md) — the Patch IR contract
  used in chapter 08.
- [`docs/language/EFFECTS_AND_CAPABILITIES.md`](../language/EFFECTS_AND_CAPABILITIES.md)
  — chapter 04 follow-up reading.
- [`README.md`](../../README.md) — top-level project README.

## Reporting tutorial bugs

If a command in this tutorial does not produce the documented output on the
pinned toolchain, open an issue at the repository's tracker
(<https://github.com/Eldergenix/Orison>) with the chapter number, the exact
command run, the observed output, and your `ori doctor --json` envelope. The
tutorial is verified against the same `python3.13 scripts/validate_all.py --full`
gate that CI runs, so a divergence is a real defect.

## What this tutorial does not cover

The bootstrap intentionally ships less than the full specification. The tutorial
mirrors that scope:

- Full Hindley–Milner inference inside arbitrary expression bodies. The
  bootstrap supports item-level signatures and the simplified body forms in
  [`docs/language/REFERENCE.md`](../language/REFERENCE.md).
- Region-inference borrow checking. The borrow checker prototype emits the
  `B00**` family but is not invoked by `ori check` for every binding pattern.
- Optimising native AOT codegen. `ori build --target llvm-text` writes textual
  scaffolding; native binaries are not produced today.
- A real HTTP, websocket, queue, or mail runtime. The modules under
  [`stdlib/std/`](../../stdlib/std) are declarations; `ori run` evaluates
  pure code only.

When you finish the tutorial and want to dig into items above, start with
[`docs/ROADMAP.md`](../ROADMAP.md).

---

Begin with [chapter 01: Install](./01-install.md).
