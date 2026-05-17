//! Benchmark harness.
//!
//! The bootstrap benchmark suite is intentionally hand-rolled: no criterion,
//! no `Bencher`, no async, just a deterministic loop that times the public
//! entrypoints we care about for the edit-check-repair loop. Each sample
//! returns nanoseconds; aggregates compute mean / median / p95 / min / max.

use crate::cst::parse_cst;
use crate::patch_apply::apply_patch;
use crate::source::SourceFile;
use crate::Compiler;
use serde::Serialize;
use std::time::Instant;

/// Full benchmark report produced by [`run_default_suite`].
#[derive(Debug, Serialize)]
pub struct BenchmarkReport {
    /// Stable schema identifier (`"ori.benchmark.v1"`).
    pub schema: &'static str,
    /// Timestamp the report was generated at; deterministic format that
    /// avoids any third-party time crate.
    pub generated_at: String,
    /// Host environment metadata so cross-run comparisons stay honest.
    pub environment: Environment,
    /// One [`Suite`] per benchmark scenario.
    pub suites: Vec<Suite>,
}

/// Host environment metadata captured alongside each benchmark run.
#[derive(Debug, Serialize)]
pub struct Environment {
    /// `std::env::consts::OS`.
    pub os: String,
    /// `std::env::consts::ARCH`.
    pub arch: String,
    /// Rust toolchain version recorded at build time, if available.
    pub rustc_version: String,
    /// Optional CPU description (currently unset by the bootstrap harness).
    pub cpu: Option<String>,
}

/// A named group of related [`Metric`] records.
#[derive(Debug, Serialize)]
pub struct Suite {
    /// Short, stable suite name.
    pub name: String,
    /// One [`Metric`] per measurement inside the suite.
    pub metrics: Vec<Metric>,
}

/// A single timing measurement with summary statistics.
#[derive(Debug, Serialize)]
pub struct Metric {
    /// Stable metric key (e.g. `"check_small_ns"`).
    pub key: String,
    /// Unit string ("ns" today).
    pub unit: String,
    /// Number of samples taken (post-warmup).
    pub samples: usize,
    /// Arithmetic mean across `samples`.
    pub mean: f64,
    /// Median (50th percentile).
    pub p50: f64,
    /// 95th percentile.
    pub p95: f64,
    /// Largest single observation.
    pub max: f64,
    /// Smallest single observation.
    pub min: f64,
}

impl BenchmarkReport {
    /// Render the report as a JSON string using the shared canonical encoder.
    pub fn to_json(&self) -> String {
        crate::json::to_json(self)
    }
}

/// Minimum samples per metric. Anything smaller produces noisy percentiles.
const MIN_SAMPLES: usize = 3;
/// Number of warmup iterations executed and discarded before measurements
/// begin, used to amortise one-time costs (JIT, page faults, branch caches).
const BENCH_WARMUP_ITERS: usize = 2;
/// Percentile rank used for the `p95` summary statistic.
const P95_RANK: f64 = 0.95;

/// Run the default benchmark suite for `samples` measurements per metric and
/// return a [`BenchmarkReport`].
pub fn run_default_suite(samples: usize) -> BenchmarkReport {
    let samples = samples.max(MIN_SAMPLES);
    let small = include_str_or_default("examples/hello.ori", DEFAULT_SMALL);
    let medium = include_str_or_default("examples/fullstack/users.ori", DEFAULT_MEDIUM);

    let mut suites = Vec::new();

    suites.push(Suite {
        name: "cold_check_latency".to_string(),
        metrics: vec![bench_metric("check_small_ns", "ns", samples, || {
            let source = SourceFile::new("/bench-small.ori", small.clone());
            let _ = Compiler::check_source(source);
        })],
    });

    suites.push(Suite {
        name: "warm_check_latency".to_string(),
        metrics: vec![bench_metric("check_medium_ns", "ns", samples, || {
            let source = SourceFile::new("/bench-medium.ori", medium.clone());
            let _ = Compiler::check_source(source);
        })],
    });

    suites.push(Suite {
        name: "cst_parse_latency".to_string(),
        metrics: vec![bench_metric("cst_medium_ns", "ns", samples, || {
            let source = SourceFile::new("/bench-cst.ori", medium.clone());
            let _ = parse_cst(&source);
        })],
    });

    suites.push(Suite {
        name: "agent_map_token_density".to_string(),
        metrics: vec![bench_metric("agent_map_medium_ns", "ns", samples, || {
            let source = SourceFile::new("/bench-agent.ori", medium.clone());
            let result = Compiler::check_source(source);
            let json = ori_agent_map_for(&result);
            std::hint::black_box(json);
        })],
    });

    suites.push(Suite {
        name: "patch_validation_latency".to_string(),
        metrics: vec![bench_metric("patch_check_ns", "ns", samples, || {
            let _ = crate::patch::check_patch_json("/bench-patch.json", SMALL_PATCH_FIXTURE);
        })],
    });

    suites.push(Suite {
        name: "patch_apply_latency".to_string(),
        metrics: vec![bench_metric("patch_apply_dry_ns", "ns", samples, || {
            let source = SourceFile::new("/bench-apply.ori", small.clone());
            let patch = patch_against_first_fn(&source);
            let _ = apply_patch(&source, &patch, true);
        })],
    });

    suites.push(Suite {
        name: "formatter_throughput".to_string(),
        metrics: vec![bench_metric("format_medium_ns", "ns", samples, || {
            let _ = Compiler::format_source(&medium);
        })],
    });

    suites.push(Suite {
        name: "capsule_generation_latency".to_string(),
        metrics: vec![bench_metric("capsule_medium_ns", "ns", samples, || {
            let source = SourceFile::new("/bench-capsule.ori", medium.clone());
            let result = Compiler::check_source(source);
            let _ = Compiler::capsule_json(&result);
        })],
    });

    BenchmarkReport {
        schema: "ori.benchmark.v1",
        generated_at: iso8601_now(),
        environment: detect_environment(),
        suites,
    }
}

fn bench_metric<F: FnMut()>(name: &str, unit: &str, samples: usize, mut body: F) -> Metric {
    // Warm-up so JIT-friendly inner caches stabilise before any sample is
    // captured.
    for _ in 0..BENCH_WARMUP_ITERS {
        body();
    }
    let mut measurements: Vec<f64> = Vec::with_capacity(samples);
    for _ in 0..samples {
        let start = Instant::now();
        body();
        measurements.push(start.elapsed().as_nanos() as f64);
    }
    measurements.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = measurements.len();
    let min = measurements.first().copied().unwrap_or(0.0);
    let max = measurements.last().copied().unwrap_or(0.0);
    let p50 = measurements[n / 2];
    let p95_idx = ((n as f64) * P95_RANK).floor() as usize;
    let p95 = measurements[p95_idx.min(n - 1)];
    let mean = measurements.iter().sum::<f64>() / n as f64;
    Metric {
        key: name.to_string(),
        unit: unit.to_string(),
        samples,
        mean,
        p50,
        p95,
        max,
        min,
    }
}

fn detect_environment() -> Environment {
    Environment {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        rustc_version: option_env!("RUSTC_VERSION")
            .unwrap_or("unknown")
            .to_string(),
        cpu: None,
    }
}

fn iso8601_now() -> String {
    // Deterministic placeholder; the real harness in CI should override this
    // via an env override. We avoid a chrono dependency in the bootstrap.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("@unix:{now}")
}

fn include_str_or_default(rel: &str, default_text: &str) -> String {
    std::fs::read_to_string(rel).unwrap_or_else(|_| default_text.to_string())
}

fn ori_agent_map_for(result: &crate::CompileResult) -> String {
    // Minimal local stand-in to avoid a circular dep on ori-agent.
    let symbols: Vec<&str> = result
        .module
        .symbols
        .iter()
        .map(|s| s.id.as_str())
        .collect();
    format!(
        "{{\"module\":\"{}\",\"n\":{}}}",
        result.module.name,
        symbols.len()
    )
}

fn patch_against_first_fn(source: &SourceFile) -> String {
    let cst = parse_cst(source);
    let target = cst
        .nodes
        .iter()
        .find(|n| matches!(n.kind, crate::cst::CstNodeKind::Function))
        .map(|n| n.id.as_str().to_string())
        .unwrap_or_else(|| "node:missing".to_string());
    serde_json::json!({
        "schema": "ori.patch.v1",
        "intent": "bench-noop",
        "operations": [
            {
                "op": "insert_after",
                "target": target,
                "text": "// benchmark noop"
            }
        ]
    })
    .to_string()
}

const DEFAULT_SMALL: &str = "module bench.small\nfn hello() -> Unit uses log\n";
const DEFAULT_MEDIUM: &str = r#"module bench.medium
import std.json
import std.logging
type Product
type ProductId
fn fetch(id: ProductId) -> Result[Product, Str] uses db.read
fn list() -> List[Product] uses db.read
fn search(query: Str) -> List[Product] uses db.read
fn upsert(product: Product) -> Unit uses db.write
service Catalog
view ProductList(products: List[Product]) uses ui
"#;

const SMALL_PATCH_FIXTURE: &str = r#"{
  "schema": "ori.patch.v1",
  "intent": "bench-noop",
  "operations": [
    { "op": "add_import", "text": "import std.logging" }
  ],
  "tests": { "run": [] }
}"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_default_suite_and_returns_metrics() {
        let report = run_default_suite(3);
        assert_eq!(report.schema, "ori.benchmark.v1");
        assert!(report.suites.len() >= 7);
        for suite in &report.suites {
            for metric in &suite.metrics {
                assert!(metric.samples >= 3);
                assert!(metric.mean >= 0.0);
                assert!(metric.p95 >= metric.p50);
            }
        }
    }

    #[test]
    fn report_serialises_to_valid_json() {
        let report = run_default_suite(3);
        let json = report.to_json();
        assert!(json.contains("\"schema\":\"ori.benchmark.v1\""));
        assert!(json.contains("\"suites\""));
    }
}
