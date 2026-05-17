#!/usr/bin/env python3.13
"""Build the diagnostic ID index for the Orison toolchain.

This script scans every ``crates/*/src/*.rs`` file for diagnostic ID string
constants and emits a deterministic Markdown index at
``docs/diagnostics/INDEX.md``. It is intentionally dependency-light: only the
Python 3.13 standard library (``re``, ``pathlib``, ``argparse``, ``sys``)
is used so the script can run in any sandboxed CI environment.

Two recognised ID shapes:

* short prefix:  ``^[EWBSNAQ]\\d{4}$``        (e.g. ``E0001``)
* long  prefix:  ``^(CAP|RND|DSK|PUB|MOB|PRE|PROTO_E|R|D|AUD)\\d{4}$``
  (e.g. ``CAP0001``, ``PROTO_E0001``)

For every distinct ID we capture the prefix family, the first source location
(sorted deterministically), and a best-effort one-line context message
recovered from the surrounding lines. The output is written in stable sort
order so the file diff is meaningful.

Two modes:

    build_diag_index.py            # rewrites docs/diagnostics/INDEX.md
    build_diag_index.py --check    # exits non-zero if the file is stale
"""
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
CRATES_GLOB = "crates/*/src/*.rs"
INDEX_PATH = REPO_ROOT / "docs" / "diagnostics" / "INDEX.md"

# Recognised diagnostic ID prefixes. Order here also drives the section order
# in the generated index. ``PROTO_E`` must be checked before ``E`` would
# otherwise consume the leading character; the regex anchors the full ID so
# matching is unambiguous, but the alternation order keeps the families
# self-explanatory.
LONG_PREFIXES = ("CAP", "RND", "DSK", "PUB", "MOB", "PRE", "PROTO_E", "AUD")
SHORT_PREFIXES = ("E", "W", "B", "S", "N", "A", "Q", "D", "R")

# A single regex that recognises both shapes. The ID is captured by group 1.
# We intentionally require quote delimiters so we only match string literals,
# which is how diagnostic IDs are spelled in the Rust sources.
_LONG_ALT = "|".join(sorted(LONG_PREFIXES, key=len, reverse=True))
ID_RE = re.compile(
    r'"((?:' + _LONG_ALT + r'|[EWBSNAQDR])\d{4})"'
)

# Heuristics for skipping occurrences that are clearly test assertions or
# equality checks rather than the canonical definition site.
_SKIP_LINE_RE = re.compile(
    r"(assert_eq!|assert!|debug_assert|\.id\(\)\s*==|d\.id\s*==|"
    r"==\s*\"[A-Z_]+\d{4}\"|"
    r"\.any\(\|d\|\s*d\.id\s*==|"
    r"diagnostics\.insert\(|insert_diag\()"
)

# A short context string that follows the ID in a typical builder call,
# e.g. ``Diagnostic::error("E0001", "missing module declaration", span)``.
_MSG_AFTER_ID_RE = re.compile(r'"[A-Z_]+\d{4}"\s*,\s*"([^"\\]*(?:\\.[^"\\]*)*)"')
# A message that lives on the line directly after the ID (multi-line call).
_LEADING_STRING_RE = re.compile(r'^\s*"([^"\\]*(?:\\.[^"\\]*)*)"')
# Format-macro style: ``format!("...", ...)`` on the line after the ID.
_FORMAT_MSG_RE = re.compile(r'format!\(\s*"([^"\\]*(?:\\.[^"\\]*)*)"')


def family_of(diag_id: str) -> str:
    """Return the prefix family of a diagnostic ID."""
    for prefix in sorted(LONG_PREFIXES, key=len, reverse=True):
        if diag_id.startswith(prefix):
            return prefix
    return diag_id[0]


def _clean_message(raw: str) -> str:
    """Trim placeholder noise out of a Rust format string."""
    msg = raw.strip()
    # Collapse internal whitespace runs so wrapped strings render in one line.
    msg = re.sub(r"\s+", " ", msg)
    # Trim trailing punctuation that isn't useful in a table cell.
    msg = msg.rstrip()
    if len(msg) > 160:
        msg = msg[:157].rstrip() + "..."
    return msg


def _comment_above(lines: list[str], idx: int) -> str | None:
    """Return the nearest non-empty ``//``/``///`` comment line above ``idx``."""
    j = idx - 1
    while j >= 0:
        text = lines[j].strip()
        if not text:
            j -= 1
            continue
        if text.startswith("//"):
            return text.lstrip("/").strip()
        return None
    return None


_BARE_STRING_RE = re.compile(r'"([^"\\]*(?:\\.[^"\\]*)*)"')


def _extract_message(lines: list[str], idx: int) -> str:
    """Recover a one-line message describing the diagnostic at ``lines[idx]``."""
    line = lines[idx]
    # 0) Match arm of the form ``Variant { .. } => "ID"`` is the canonical
    #    *code* mapping, but the human-readable string lives in a sibling
    #    ``message()``/``Display`` impl. Try to locate the matching arm in
    #    the same file that maps the variant to a non-ID string literal.
    arm = re.match(
        r'^\s*([A-Za-z0-9_:]+)(\s*\{[^}]*\})?\s*=>\s*"[A-Z_]+\d{4}"',
        line,
    )
    if arm:
        variant = arm.group(1)
        # Just the trailing path segment, e.g. ``Unterminated``.
        short = variant.split("::")[-1]
        msg_re = re.compile(
            rf'^\s*{re.escape(variant)}(\s*\{{[^}}]*\}})?\s*=>\s*"([^"\\]*(?:\\.[^"\\]*)*)"'
        )
        short_re = re.compile(
            rf'^\s*{re.escape(short)}(\s*\{{[^}}]*\}})?\s*=>\s*"([^"\\]*(?:\\.[^"\\]*)*)"'
        )
        # Block-style match arm: ``Variant { .. } => {`` (or ``=> format!(``,
        # ``=> write!(...)``) followed by lines whose first string literal
        # is the message we want.
        block_open_re = re.compile(
            rf'^\s*(?:{re.escape(variant)}|{re.escape(short)})'
            r'(\s*\{[^}]*\})?\s*=>\s*(?:\{|format!\s*\(|write!\s*\()'
        )
        id_literal_re = re.compile(r'^[A-Z_]+\d{4}$')
        for other_idx, other in enumerate(lines):
            if other_idx == idx:
                continue
            for candidate_re in (msg_re, short_re):
                hit = candidate_re.match(other)
                if not hit:
                    continue
                text = hit.group(2)
                if id_literal_re.match(text):
                    continue
                return _clean_message(text)
            # Block-bodied arm: dig forward for the first string literal.
            if block_open_re.match(other):
                for k in range(1, 8):
                    nxt_idx = other_idx + k
                    if nxt_idx >= len(lines):
                        break
                    nxt = lines[nxt_idx]
                    mf = _FORMAT_MSG_RE.search(nxt)
                    if mf and not id_literal_re.match(mf.group(1)):
                        return _clean_message(mf.group(1))
                    mb = _BARE_STRING_RE.search(nxt)
                    if mb and not id_literal_re.match(mb.group(1)):
                        return _clean_message(mb.group(1))
        # If no sibling arm was found, fall through to the standard heuristic.
    # 1) Inline ``"ID", "message"`` pattern on the same line.
    m = _MSG_AFTER_ID_RE.search(line)
    if m:
        return _clean_message(m.group(1))
    # 2) Walk a small window forward looking for the next user-facing message
    #    string. Multi-line builder calls put the message a few lines below
    #    the ID, either as a bare string or inside ``format!("...")``.
    #    Doc-comment lines above a ``pub const`` declaration are also
    #    valuable, so we collect them too as a fallback.
    only_const_decl = bool(
        re.match(r"^\s*pub\s+const\s+\w+\s*:\s*&\s*str\s*=", line)
    )
    if not only_const_decl:
        for off in range(1, 8):
            nxt = idx + off
            if nxt >= len(lines):
                break
            stripped = lines[nxt].strip()
            if not stripped:
                continue
            mf = _FORMAT_MSG_RE.search(lines[nxt])
            if mf:
                return _clean_message(mf.group(1))
            # Skip lines whose only quoted string is another diagnostic ID.
            id_only = re.fullmatch(
                r'[^"]*"[A-Z_]+\d{4}"[^"]*', lines[nxt]
            )
            if id_only and not re.search(r'"[^"]+"\s*,\s*"', lines[nxt]):
                continue
            mb = _BARE_STRING_RE.search(lines[nxt])
            if mb:
                return _clean_message(mb.group(1))
    # 3) Doc-comments stacked above the occurrence (common for ``pub const``).
    block: list[str] = []
    j = idx - 1
    while j >= 0:
        s = lines[j].strip()
        if not s:
            break
        if s.startswith("//"):
            block.insert(0, s.lstrip("/").strip())
            j -= 1
            continue
        break
    if block:
        return _clean_message(" ".join(block))
    # 4) Fallback: the raw line itself, with the ID elided.
    return _clean_message(line.replace('"', ""))


def _is_definition_candidate(lines: list[str], idx: int) -> bool:
    """Return True if ``lines[idx]`` looks like a canonical definition site."""
    if _SKIP_LINE_RE.search(lines[idx]):
        return False
    # Reject occurrences that live inside an ``assert!(...)``/``assert_eq!(...)``
    # call whose opening paren is on a previous line. We walk backwards a
    # generous window — multi-line assertions sometimes interpose ``}`` from
    # struct literals between the ``assert_eq!(`` opening and the ID literal.
    for back in range(1, 12):
        j = idx - back
        if j < 0:
            break
        prev = lines[j]
        if _SKIP_LINE_RE.search(prev):
            return False
        # Stop walking once we leave the current statement.
        stripped = prev.rstrip()
        if stripped.endswith(";"):
            break
    return True


def scan_repository(root: Path) -> dict[str, dict]:
    """Walk the crate sources and collect a diagnostic-ID dictionary."""
    findings: dict[str, list[dict]] = {}
    files = sorted(root.glob(CRATES_GLOB))
    for path in files:
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        lines = text.splitlines()
        for lineno, line in enumerate(lines, start=1):
            for match in ID_RE.finditer(line):
                diag_id = match.group(1)
                # Filter out IDs that are present but don't match either
                # exact shape (e.g. ``D`` is a long prefix only if followed by
                # four digits, which the regex already guarantees).
                fam = family_of(diag_id)
                rel = path.relative_to(root).as_posix()
                findings.setdefault(diag_id, []).append({
                    "family": fam,
                    "path": rel,
                    "line": lineno,
                    "raw": line,
                    "is_def": _is_definition_candidate(lines, lineno - 1),
                    "_lines": lines,
                    "_idx": lineno - 1,
                })
    # Reduce each ID's occurrence list to a single canonical record. We
    # prefer non-assertion sites and, among those, sites where the recovered
    # message is meaningful (not the ID itself).
    canonical: dict[str, dict] = {}
    for diag_id, hits in findings.items():
        defs = [h for h in hits if h["is_def"]] or hits
        defs.sort(key=lambda h: (h["path"], h["line"]))
        # Score each candidate; lower score wins. A useful message contributes
        # zero; a degenerate message (the ID echoed back, ``code:``-style noise)
        # penalises the candidate so a later, richer call site is preferred.
        scored: list[tuple[tuple[int, int, str, int], dict, str]] = []
        for h in defs:
            msg = _extract_message(h["_lines"], h["_idx"])
            penalty = 0
            if diag_id in msg:
                penalty += 10
            if not msg or msg.startswith("code:") or msg.startswith("id:"):
                penalty += 5
            if "=>" in msg and msg.endswith(","):
                penalty += 3
            # Prefer the earliest acceptable definition; ties broken by path.
            scored.append(((penalty, h["line"], h["path"], h["_idx"]), h, msg))
        scored.sort(key=lambda x: x[0])
        _, chosen, message = scored[0]
        canonical[diag_id] = {
            "id": diag_id,
            "family": chosen["family"],
            "path": chosen["path"],
            "line": chosen["line"],
            "message": message,
        }
    return canonical


_FAMILY_DESCRIPTION = {
    "E": "Errors raised by the parser, resolver, and type checker.",
    "W": "Warnings produced by the linter and symbol passes.",
    "B": "Borrow-checker and ownership diagnostics.",
    "S": "String-literal and Unicode lexing diagnostics.",
    "N": "Numeric-literal lexing and overflow diagnostics.",
    "A": "Async runtime and scheduler diagnostics.",
    "Q": "Query / SQL static analysis diagnostics.",
    "D": "Design-token consistency warnings.",
    "R": "Route and HTTP dispatch diagnostics.",
    "CAP": "Capability-runtime denial and policy diagnostics.",
    "RND": "UI renderer (`ori-compiler::ui_render`) diagnostics.",
    "DSK": "Desktop-bundling diagnostics.",
    "PUB": "Package publishing pipeline diagnostics.",
    "MOB": "Mobile permission and packaging diagnostics.",
    "PRE": "Pre-processing / build-time diagnostics (reserved).",
    "PROTO_E": "Protocol negotiation errors (reserved).",
    "AUD": "Workspace audit (security and policy) findings.",
}

_FAMILY_ORDER = (
    "E", "W", "B", "S", "N", "A", "Q", "D", "R",
    "CAP", "RND", "DSK", "PUB", "MOB", "PRE", "PROTO_E", "AUD",
)


def _escape_md(text: str) -> str:
    """Escape characters that would break a Markdown table cell."""
    return text.replace("|", "\\|").replace("\n", " ")


def _source_link(record: dict) -> str:
    """Return a relative link from ``docs/diagnostics/INDEX.md`` to source."""
    rel = f"../../{record['path']}#L{record['line']}"
    return f"[{record['path']}:{record['line']}]({rel})"


def render_index(canonical: dict[str, dict]) -> str:
    """Render the deterministic Markdown index document."""
    by_family: dict[str, list[dict]] = {fam: [] for fam in _FAMILY_ORDER}
    for record in canonical.values():
        by_family.setdefault(record["family"], []).append(record)
    for fam in by_family:
        by_family[fam].sort(key=lambda r: r["id"])

    out: list[str] = []
    out.append("<!-- GENERATED by scripts/build_diag_index.py — do not edit. -->")
    out.append("# Orison Diagnostic Index")
    out.append("")
    out.append(
        "This file is the canonical inventory of every stable diagnostic ID "
        "emitted by the Orison toolchain. It is regenerated from the crate "
        "sources by `scripts/build_diag_index.py`."
    )
    out.append("")
    out.append("## Summary")
    out.append("")
    out.append("| Family | Count |")
    out.append("| --- | ---: |")
    total = 0
    for fam in _FAMILY_ORDER:
        count = len(by_family.get(fam, []))
        if count == 0:
            continue
        out.append(f"| `{fam}` | {count} |")
        total += count
    out.append(f"| **Total** | **{total}** |")
    out.append("")

    for fam in _FAMILY_ORDER:
        records = by_family.get(fam, [])
        if not records:
            continue
        description = _FAMILY_DESCRIPTION.get(fam, "")
        out.append(f"## `{fam}` — {description}")
        out.append("")
        out.append("| ID | Message | First defined |")
        out.append("| --- | --- | --- |")
        for record in records:
            msg = _escape_md(record["message"]) or "_(no message recovered)_"
            link = _source_link(record)
            out.append(f"| `{record['id']}` | {msg} | {link} |")
        out.append("")

    out.append(f"_Total diagnostic IDs: **{total}**._")
    out.append("")
    return "\n".join(out)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="Verify that docs/diagnostics/INDEX.md matches the source tree "
             "without rewriting the file. Exits non-zero on drift.",
    )
    parser.add_argument(
        "--root",
        type=Path,
        default=REPO_ROOT,
        help="Repository root (defaults to the script's parent).",
    )
    args = parser.parse_args(argv)

    canonical = scan_repository(args.root)
    rendered = render_index(canonical)

    target = args.root / "docs" / "diagnostics" / "INDEX.md"
    if args.check:
        if not target.exists():
            print(
                "docs/diagnostics/INDEX.md is missing; run "
                "`scripts/build_diag_index.py` to regenerate.",
                file=sys.stderr,
            )
            return 1
        current = target.read_text(encoding="utf-8")
        if current != rendered:
            print(
                "docs/diagnostics/INDEX.md is out of date; rerun "
                "`scripts/build_diag_index.py` to refresh it.",
                file=sys.stderr,
            )
            return 1
        return 0

    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(rendered, encoding="utf-8")
    print(f"Wrote {target.relative_to(args.root)} ({len(canonical)} IDs).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
