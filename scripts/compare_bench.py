#!/usr/bin/env python3
"""Compare two Orison BENCHMARKS.results.json artefacts and gate on p50 drift.

Inputs are the JSON documents emitted by `ori bench --json`, conforming to
the `ori.benchmark.v1` schema. The script:

1. Loads both files (baseline + current).
2. Joins metrics by `(suite_name, metric_key)`.
3. Computes the percentage delta on `p50` (current relative to baseline).
4. Emits a markdown table to stdout (or to `--markdown <path>`).
5. Exits with status 1 if any metric regresses by more than `--threshold`
   percent (default 20).

Design notes:
- New suites (present in current, missing from baseline) are reported as
  informational rows and never block the gate. Same for new metrics inside
  an existing suite.
- Removed suites (present in baseline, missing from current) are reported
  but also do not block the gate — the bench surface is allowed to shrink.
- A baseline p50 of 0 is treated as "n/a" rather than triggering a
  division-by-zero. This matches `ori bench`'s behaviour for sub-nanosecond
  fixtures (e.g. trivially short wasm encodes).
- The script never imports third-party packages so it can run in the
  static-gate workflow without any pip install.
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any


# --- exit codes -------------------------------------------------------------
EXIT_OK = 0
EXIT_REGRESSION = 1
EXIT_USAGE = 2


def load_report(path: Path) -> dict[str, Any]:
    """Load a bench report, surfacing a clean error for missing/invalid files."""
    if not path.is_file():
        raise SystemExit(f"compare_bench: file not found: {path}")
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise SystemExit(f"compare_bench: invalid JSON in {path}: {exc}") from exc
    if not isinstance(data, dict) or "suites" not in data:
        raise SystemExit(
            f"compare_bench: {path} is not an ori.benchmark.v1 document"
        )
    return data


def index_metrics(report: dict[str, Any]) -> dict[tuple[str, str], dict[str, Any]]:
    """Flatten a report into a {(suite, metric_key): metric_obj} mapping."""
    out: dict[tuple[str, str], dict[str, Any]] = {}
    for suite in report.get("suites", []):
        suite_name = suite.get("name") or "<unnamed>"
        for metric in suite.get("metrics", []):
            key = metric.get("key") or "<unnamed>"
            out[(suite_name, key)] = metric
    return out


def fmt_p50(p50: Any) -> str:
    """Format a p50 value tolerating None/strings."""
    if isinstance(p50, (int, float)):
        # ns precision; keep it compact for the markdown table.
        if p50 >= 1000:
            return f"{p50:,.0f}"
        return f"{p50:.2f}"
    return "n/a"


def pct_delta(baseline: float, current: float) -> float | None:
    """Percentage change of current vs baseline (e.g. +20.0 means 20% slower)."""
    if baseline == 0:
        return None
    return (current - baseline) / baseline * 100.0


def status_for(delta: float | None, threshold: float) -> str:
    if delta is None:
        return "info"
    if delta > threshold:
        return "regression"
    if delta < -threshold:
        return "improvement"
    return "ok"


def render_markdown(rows: list[dict[str, Any]], threshold: float) -> str:
    lines: list[str] = []
    lines.append(f"# Bench regression report (threshold: ±{threshold:.1f}% on p50)")
    lines.append("")
    lines.append("| Suite | Metric | Baseline p50 | Current p50 | Δ% | Status |")
    lines.append("|-------|--------|--------------|-------------|-----|--------|")
    for row in rows:
        delta = row["delta"]
        delta_str = "n/a" if delta is None else f"{delta:+.2f}%"
        lines.append(
            "| {suite} | {metric} | {baseline} | {current} | {delta} | {status} |".format(
                suite=row["suite"],
                metric=row["metric"],
                baseline=fmt_p50(row["baseline_p50"]),
                current=fmt_p50(row["current_p50"]),
                delta=delta_str,
                status=row["status"],
            )
        )
    return "\n".join(lines) + "\n"


def compare(
    baseline_path: Path,
    current_path: Path,
    threshold: float,
) -> tuple[int, str]:
    baseline = load_report(baseline_path)
    current = load_report(current_path)

    base_idx = index_metrics(baseline)
    curr_idx = index_metrics(current)

    rows: list[dict[str, Any]] = []
    regressions: list[dict[str, Any]] = []

    all_keys = sorted(set(base_idx.keys()) | set(curr_idx.keys()))
    for key in all_keys:
        suite, metric = key
        base_metric = base_idx.get(key)
        curr_metric = curr_idx.get(key)

        if base_metric is None:
            row = {
                "suite": suite,
                "metric": metric,
                "baseline_p50": None,
                "current_p50": (curr_metric or {}).get("p50"),
                "delta": None,
                "status": "info (new suite/metric)",
            }
            rows.append(row)
            continue

        if curr_metric is None:
            row = {
                "suite": suite,
                "metric": metric,
                "baseline_p50": base_metric.get("p50"),
                "current_p50": None,
                "delta": None,
                "status": "info (removed suite/metric)",
            }
            rows.append(row)
            continue

        base_p50 = base_metric.get("p50")
        curr_p50 = curr_metric.get("p50")
        if not isinstance(base_p50, (int, float)) or not isinstance(
            curr_p50, (int, float)
        ):
            rows.append({
                "suite": suite,
                "metric": metric,
                "baseline_p50": base_p50,
                "current_p50": curr_p50,
                "delta": None,
                "status": "info (non-numeric p50)",
            })
            continue

        delta = pct_delta(float(base_p50), float(curr_p50))
        status = status_for(delta, threshold)
        row = {
            "suite": suite,
            "metric": metric,
            "baseline_p50": base_p50,
            "current_p50": curr_p50,
            "delta": delta,
            "status": status,
        }
        rows.append(row)
        if status == "regression":
            regressions.append(row)

    md = render_markdown(rows, threshold)

    if regressions:
        md += "\n## Regressions exceeding threshold\n\n"
        for r in regressions:
            md += (
                f"- `{r['suite']} / {r['metric']}`: "
                f"{fmt_p50(r['baseline_p50'])} ns -> {fmt_p50(r['current_p50'])} ns "
                f"({r['delta']:+.2f}%)\n"
            )

    return (EXIT_REGRESSION if regressions else EXIT_OK), md


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="compare_bench",
        description=(
            "Compare two ori bench JSON artefacts and exit non-zero if any "
            "metric's p50 regresses by more than --threshold percent."
        ),
    )
    parser.add_argument(
        "--baseline",
        type=Path,
        required=True,
        help="Path to the baseline ori bench JSON.",
    )
    parser.add_argument(
        "--current",
        type=Path,
        required=True,
        help="Path to the current-run ori bench JSON.",
    )
    parser.add_argument(
        "--threshold",
        type=float,
        default=20.0,
        help="Regression threshold in percent (default: 20).",
    )
    parser.add_argument(
        "--markdown",
        type=Path,
        default=None,
        help="Optional path to write the markdown report to (in addition to stdout).",
    )

    args = parser.parse_args(argv)

    if args.threshold < 0:
        parser.error("--threshold must be non-negative")

    exit_code, md = compare(args.baseline, args.current, args.threshold)
    sys.stdout.write(md)
    if args.markdown is not None:
        args.markdown.write_text(md, encoding="utf-8")
    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
