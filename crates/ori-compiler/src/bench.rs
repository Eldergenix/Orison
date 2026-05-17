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
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
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

    // ---------------------------------------------------------------------
    // Type system
    // ---------------------------------------------------------------------
    suites.push(Suite {
        name: "type_check_signatures_latency".to_string(),
        metrics: vec![bench_metric("type_check_medium_ns", "ns", samples, || {
            let source = SourceFile::new("/bench-tc.ori", medium.clone());
            let result = Compiler::check_source(source);
            let _ = crate::type_check::type_check_module(&result.module);
        })],
    });

    suites.push(Suite {
        name: "exhaustive_match_latency".to_string(),
        metrics: vec![bench_metric("exhaustive_medium_ns", "ns", samples, || {
            let source = SourceFile::new("/bench-exh.ori", medium.clone());
            let bodies = crate::body::parse_module_bodies(&source);
            let result = Compiler::check_source(source);
            let _ = crate::exhaustive::check_module_matches(&result.module, &bodies);
        })],
    });

    // ---------------------------------------------------------------------
    // Effects
    // ---------------------------------------------------------------------
    suites.push(Suite {
        name: "effect_propagation_fixpoint_latency".to_string(),
        metrics: vec![bench_metric(
            "effect_propagate_medium_ns",
            "ns",
            samples,
            || {
                let source = SourceFile::new("/bench-eff.ori", medium.clone());
                let bodies = crate::body::parse_module_bodies(&source);
                let result = Compiler::check_source(source);
                let mut graph =
                    crate::effect_propagate::build_effect_graph(&result.module, &bodies);
                let _ = crate::effect_propagate::propagate_effects(&mut graph);
            },
        )],
    });

    suites.push(Suite {
        name: "capability_manifest_latency".to_string(),
        metrics: vec![bench_metric("capability_medium_ns", "ns", samples, || {
            let source = SourceFile::new("/bench-cap.ori", medium.clone());
            let result = Compiler::check_source(source);
            let _ = crate::effect_check::build_capability_manifest(&result.module, &[]);
        })],
    });

    // ---------------------------------------------------------------------
    // Borrow checker
    // ---------------------------------------------------------------------
    suites.push(Suite {
        name: "borrow_check_module_latency".to_string(),
        metrics: vec![bench_metric("borrow_medium_ns", "ns", samples, || {
            let source = SourceFile::new("/bench-borrow.ori", medium.clone());
            let result = Compiler::check_source(source);
            let _ = crate::borrow::borrow_check_module(&result.module);
        })],
    });

    // ---------------------------------------------------------------------
    // Lowering
    // ---------------------------------------------------------------------
    suites.push(Suite {
        name: "hir_mir_lowering_latency".to_string(),
        metrics: vec![
            bench_metric("hir_lower_medium_ns", "ns", samples, || {
                let source = SourceFile::new("/bench-hir.ori", medium.clone());
                let result = Compiler::check_source(source);
                let _ = crate::hir::lower_module(&result.module);
            }),
            bench_metric("mir_lower_medium_ns", "ns", samples, || {
                let source = SourceFile::new("/bench-mir.ori", medium.clone());
                let result = Compiler::check_source(source);
                let hir = crate::hir::lower_module(&result.module);
                let _ = crate::mir::lower_module(&hir);
            }),
        ],
    });

    // ---------------------------------------------------------------------
    // Codegen
    // ---------------------------------------------------------------------
    suites.push(Suite {
        name: "wasm_encode_latency".to_string(),
        metrics: vec![
            bench_metric("wasm_minimal_ns", "ns", samples, || {
                let _ = crate::wasm_encoder::encode_minimal_module();
            }),
            bench_metric("wasm_hello_ns", "ns", samples, || {
                let _ = crate::wasm_encoder::encode_hello_module();
            }),
            bench_metric("wasm_from_mir_ns", "ns", samples, || {
                let source = SourceFile::new("/bench-w.ori", small.clone());
                let result = Compiler::check_source(source);
                let hir = crate::hir::lower_module(&result.module);
                let mir = crate::mir::lower_module(&hir);
                let _ = crate::wasm_encoder::encode_from_mir(&mir);
            }),
        ],
    });

    suites.push(Suite {
        name: "textual_ir_emit_latency".to_string(),
        metrics: vec![bench_metric("textual_ir_medium_ns", "ns", samples, || {
            let source = SourceFile::new("/bench-tir.ori", medium.clone());
            let result = Compiler::check_source(source);
            let hir = crate::hir::lower_module(&result.module);
            let mir = crate::mir::lower_module(&hir);
            let _ = crate::codegen_text::emit_textual_ir(&mir);
        })],
    });

    // ---------------------------------------------------------------------
    // Manifests
    // ---------------------------------------------------------------------
    suites.push(Suite {
        name: "openapi_extract_latency".to_string(),
        metrics: vec![bench_metric("openapi_medium_ns", "ns", samples, || {
            let source = SourceFile::new("/bench-oa.ori", medium.clone());
            let result = Compiler::check_source(source);
            let _ = crate::openapi::extract_openapi(&result.module);
        })],
    });

    suites.push(Suite {
        name: "ui_manifest_latency".to_string(),
        metrics: vec![bench_metric("ui_medium_ns", "ns", samples, || {
            let source = SourceFile::new("/bench-ui.ori", medium.clone());
            let result = Compiler::check_source(source);
            let _ = crate::ui_check::build_ui_manifest(&result.module);
        })],
    });

    suites.push(Suite {
        name: "wasm_component_manifest_latency".to_string(),
        metrics: vec![bench_metric(
            "wasm_manifest_medium_ns",
            "ns",
            samples,
            || {
                let source = SourceFile::new("/bench-wm.ori", medium.clone());
                let result = Compiler::check_source(source);
                let _ = crate::wasm_component::build_wasm_component_manifest(&result.module);
            },
        )],
    });

    suites.push(Suite {
        name: "mobile_manifest_latency".to_string(),
        metrics: vec![bench_metric("mobile_medium_ns", "ns", samples, || {
            let source = SourceFile::new("/bench-mob.ori", medium.clone());
            let result = Compiler::check_source(source);
            let _ = crate::mobile::build_mobile_manifest(
                &result.module,
                "com.bench.demo",
                &["ios", "android"],
            );
        })],
    });

    // ---------------------------------------------------------------------
    // Body parsing
    // ---------------------------------------------------------------------
    suites.push(Suite {
        name: "body_parse_latency".to_string(),
        metrics: vec![bench_metric("body_parse_medium_ns", "ns", samples, || {
            let source = SourceFile::new("/bench-body.ori", medium.clone());
            let _ = crate::body::parse_module_bodies(&source);
        })],
    });

    // ---------------------------------------------------------------------
    // Async runtime
    // ---------------------------------------------------------------------
    suites.push(Suite {
        name: "async_scheduler_throughput".to_string(),
        metrics: vec![bench_metric("async_spawn_100_ns", "ns", samples, || {
            use crate::async_runtime::Scheduler;
            let mut sched = Scheduler::new();
            for i in 0..100u64 {
                sched.spawn(crate::interp_exec::Value::Int(i as i64));
            }
            let mut popped = 0usize;
            while sched.step().is_some() {
                popped += 1;
                if popped >= 100 {
                    break;
                }
            }
            std::hint::black_box(popped);
        })],
    });

    // ---------------------------------------------------------------------
    // Importers
    // ---------------------------------------------------------------------
    suites.push(Suite {
        name: "graphql_import_latency".to_string(),
        metrics: vec![
            bench_metric("graphql_parse_ns", "ns", samples, || {
                let _ = crate::graphql_import::parse_sdl(GRAPHQL_FIXTURE);
            }),
            bench_metric("graphql_emit_ns", "ns", samples, || {
                if let Ok(schema) = crate::graphql_import::parse_sdl(GRAPHQL_FIXTURE) {
                    let _ = crate::graphql_import::to_orison_module(&schema, "bench.gql");
                }
            }),
        ],
    });

    suites.push(Suite {
        name: "rpc_import_latency".to_string(),
        metrics: vec![
            bench_metric("rpc_parse_ns", "ns", samples, || {
                let _ = crate::rpc_import::parse_proto(PROTO_FIXTURE);
            }),
            bench_metric("rpc_emit_ns", "ns", samples, || {
                if let Ok(proto) = crate::rpc_import::parse_proto(PROTO_FIXTURE) {
                    let _ = crate::rpc_import::to_orison_module(&proto, "bench.rpc");
                }
            }),
        ],
    });

    // ---------------------------------------------------------------------
    // Database
    // ---------------------------------------------------------------------
    suites.push(Suite {
        name: "sql_query_check_latency".to_string(),
        metrics: vec![bench_metric("sql_check_medium_ns", "ns", samples, || {
            let source = SourceFile::new("/bench-sql.ori", DEFAULT_SQL.to_string());
            let result = Compiler::check_source(source);
            let _ = crate::sql_check::check_module_queries(&result.module);
        })],
    });

    suites.push(Suite {
        name: "migration_toposort_latency".to_string(),
        metrics: vec![bench_metric("migration_toposort_ns", "ns", samples, || {
            let source = SourceFile::new("/bench-mig.ori", DEFAULT_SQL.to_string());
            let result = Compiler::check_source(source);
            let migrations = crate::migration_graph::extract_migrations(&result.module);
            let _ = crate::migration_graph::topological_order(&migrations);
        })],
    });

    // ---------------------------------------------------------------------
    // Coverage + docs + migrate + preproc
    // ---------------------------------------------------------------------
    suites.push(Suite {
        name: "coverage_report_latency".to_string(),
        metrics: vec![bench_metric("coverage_ns", "ns", samples, || {
            let mut impls = BTreeMap::new();
            impls.insert("/src/m.ori".to_string(), medium.clone());
            let mut tests = BTreeMap::new();
            tests.insert(
                "/tests/m.ori".to_string(),
                "module t\nfn test_smoke() -> Unit\n".to_string(),
            );
            let _ = crate::coverage::coverage_for_files(&impls, &tests);
        })],
    });

    suites.push(Suite {
        name: "docs_generate_latency".to_string(),
        metrics: vec![
            bench_metric("docs_human_ns", "ns", samples, || {
                let source = SourceFile::new("/bench-doc.ori", medium.clone());
                let result = Compiler::check_source(source);
                let _ = crate::docs::generate_human_docs(&[result.module]);
            }),
            bench_metric("docs_agent_budget_1500_ns", "ns", samples, || {
                let source = SourceFile::new("/bench-doc.ori", medium.clone());
                let result = Compiler::check_source(source);
                let _ = crate::docs::generate_agent_docs(&[result.module], 1500);
            }),
        ],
    });

    suites.push(Suite {
        name: "preprocessor_throughput".to_string(),
        metrics: vec![bench_metric("preproc_substitute_ns", "ns", samples, || {
            let cfg = crate::preproc::PreprocessConfig::new().with_const("orison/version", "0.1.1");
            let _ = crate::preproc::preprocess(PREPROC_FIXTURE, &cfg);
        })],
    });

    // ---------------------------------------------------------------------
    // Agent ABI extras
    // ---------------------------------------------------------------------
    suites.push(Suite {
        name: "agent_map_budget_levels".to_string(),
        metrics: vec![
            bench_metric("agent_map_budget_500_ns", "ns", samples, || {
                let source = SourceFile::new("/bench-am.ori", medium.clone());
                let result = Compiler::check_source(source);
                let json = ori_agent_map_for_budget(&result, 500);
                std::hint::black_box(json);
            }),
            bench_metric("agent_map_budget_2000_ns", "ns", samples, || {
                let source = SourceFile::new("/bench-am.ori", medium.clone());
                let result = Compiler::check_source(source);
                let json = ori_agent_map_for_budget(&result, 2000);
                std::hint::black_box(json);
            }),
            bench_metric("agent_map_budget_4000_ns", "ns", samples, || {
                let source = SourceFile::new("/bench-am.ori", medium.clone());
                let result = Compiler::check_source(source);
                let json = ori_agent_map_for_budget(&result, 4000);
                std::hint::black_box(json);
            }),
        ],
    });

    // ---------------------------------------------------------------------
    // Incremental
    // ---------------------------------------------------------------------
    suites.push(Suite {
        name: "incremental_cache_latency".to_string(),
        metrics: vec![bench_metric("incremental_hash_ns", "ns", samples, || {
            let mut cache = crate::incremental::IncrementalCache::new();
            let mut files = BTreeMap::new();
            files.insert("/a.ori".to_string(), small.clone());
            files.insert("/b.ori".to_string(), medium.clone());
            let _ = cache.update(&files);
        })],
    });

    suites.push(Suite {
        name: "query_fingerprint_latency".to_string(),
        metrics: vec![bench_metric(
            "query_fingerprint_medium_ns",
            "ns",
            samples,
            || {
                let source = SourceFile::new("/bench-q.ori", medium.clone());
                let result = Compiler::check_source(source);
                let _ = crate::query::module_fingerprints(&result.module);
            },
        )],
    });

    // ---------------------------------------------------------------------
    // Wave-2/3: UI render, runtime dispatch, capability guard, model loop,
    // parallel async scheduler, body-level borrow check.
    // ---------------------------------------------------------------------
    suites.push(Suite {
        name: "ui_render_latency".to_string(),
        metrics: vec![
            bench_metric("ui_render_small_ns", "ns", samples, || {
                let view = build_bench_view_small();
                let props = build_bench_props_small();
                let _ = crate::ui_render::render_view(&view, &props);
            }),
            bench_metric("ui_render_medium_ns", "ns", samples, || {
                let view = build_bench_view_medium();
                let props = build_bench_props_medium();
                let _ = crate::ui_render::render_view(&view, &props);
            }),
        ],
    });

    suites.push(Suite {
        name: "ui_diff_latency".to_string(),
        metrics: vec![bench_metric("ui_diff_small_ns", "ns", samples, || {
            let old = build_bench_node_old();
            let new = build_bench_node_new();
            let _ = crate::ui_render::diff_trees(&old, &new);
        })],
    });

    suites.push(Suite {
        name: "backend_dispatch_build_latency".to_string(),
        metrics: vec![bench_metric(
            "dispatch_build_medium_ns",
            "ns",
            samples,
            || {
                let source = SourceFile::new("/bench-disp.ori", DISPATCH_FIXTURE.to_string());
                let result = Compiler::check_source(source);
                let _ = crate::backend_dispatch::build_dispatch_table(&result.module);
            },
        )],
    });

    suites.push(Suite {
        name: "backend_dispatch_call_latency".to_string(),
        metrics: vec![bench_metric("dispatch_call_ns", "ns", samples, || {
            // Build a fresh table + request each iteration so the metric is
            // representative of end-to-end dispatch cost on a cold cache.
            let source = SourceFile::new("/bench-disp-c.ori", DISPATCH_FIXTURE.to_string());
            let result = Compiler::check_source(source);
            let table = crate::backend_dispatch::build_dispatch_table(&result.module);
            let req = build_bench_dispatch_request();
            let _ = crate::backend_dispatch::dispatch(&table, &req);
        })],
    });

    suites.push(Suite {
        name: "capability_runtime_guard_latency".to_string(),
        metrics: vec![bench_metric("guard_call_ns", "ns", samples, || {
            let ctx = build_bench_guard_context();
            let _ = crate::capability_runtime::guard_call(&ctx);
        })],
    });

    suites.push(Suite {
        name: "model_loop_envelope_roundtrip_latency".to_string(),
        metrics: vec![bench_metric(
            "model_loop_roundtrip_ns",
            "ns",
            samples,
            || {
                let envelope = build_bench_model_loop_envelope();
                let json = model_loop_envelope_to_json(&envelope);
                let parsed = model_loop_envelope_from_json(&json);
                std::hint::black_box(parsed.is_ok());
            },
        )],
    });

    suites.push(Suite {
        name: "async_parallel_throughput".to_string(),
        metrics: vec![bench_metric(
            "async_parallel_4w_100_tasks_ns",
            "ns",
            samples,
            || {
                use crate::async_runtime::Scheduler;
                let mut sched = Scheduler::new();
                for i in 0..100u64 {
                    sched.spawn(crate::interp_exec::Value::Int(i as i64));
                }
                let report = sched.run_to_completion_parallel(4, 16);
                std::hint::black_box(report);
            },
        )],
    });

    suites.push(Suite {
        name: "borrow_body_check_latency".to_string(),
        metrics: vec![bench_metric("borrow_body_medium_ns", "ns", samples, || {
            let source = SourceFile::new("/bench-bborrow.ori", medium.clone());
            let bodies = crate::body::parse_module_bodies(&source);
            let result = Compiler::check_source(source);
            let _ = crate::borrow::borrow_check_bodies(&result.module, &bodies);
        })],
    });

    BenchmarkReport {
        schema: "ori.benchmark.v1",
        generated_at: iso8601_now(),
        environment: detect_environment(),
        suites,
    }
}

// -------------------------------------------------------------------------
// Wave-2/3 bench fixtures and helpers
// -------------------------------------------------------------------------

fn build_bench_view_small() -> crate::ui_render::ViewDecl {
    use crate::ui_render::{PropSlot, PropValue, ViewDecl, ViewTemplate};
    let body = ViewTemplate::leaf("Heading")
        .with_prop("level", PropValue::Int(1))
        .with_prop_binding("title");
    ViewDecl::placeholder("BenchSmall", vec![PropSlot::required_str("title")]).with_body(body)
}

fn build_bench_props_small() -> BTreeMap<String, crate::ui_render::PropValue> {
    let mut m: BTreeMap<String, crate::ui_render::PropValue> = BTreeMap::new();
    m.insert(
        "title".to_string(),
        crate::ui_render::PropValue::Str("Hello".to_string()),
    );
    m
}

fn build_bench_view_medium() -> crate::ui_render::ViewDecl {
    use crate::ui_render::{PropSlot, PropValue, ViewDecl, ViewTemplate};
    let mut row = ViewTemplate::leaf("Row")
        .with_prop("compact", PropValue::Bool(true))
        .with_prop_binding("user_name");
    for i in 0..6 {
        row = row.with_child(
            ViewTemplate::leaf("Cell")
                .with_key(format!("cell-{i}"))
                .with_prop("index", PropValue::Int(i as i64)),
        );
    }
    let body = ViewTemplate::leaf("Card")
        .with_prop_binding("user_name")
        .with_child(
            ViewTemplate::leaf("Heading")
                .with_prop("level", PropValue::Int(2))
                .with_prop_binding("user_name"),
        )
        .with_child(row);
    ViewDecl::placeholder(
        "BenchMedium",
        vec![
            PropSlot::required_str("user_name"),
            PropSlot::optional("subtitle", "str"),
        ],
    )
    .with_body(body)
}

fn build_bench_props_medium() -> BTreeMap<String, crate::ui_render::PropValue> {
    let mut m: BTreeMap<String, crate::ui_render::PropValue> = BTreeMap::new();
    m.insert(
        "user_name".to_string(),
        crate::ui_render::PropValue::Str("Ada".to_string()),
    );
    m.insert(
        "subtitle".to_string(),
        crate::ui_render::PropValue::Str("engineer".to_string()),
    );
    m
}

fn build_bench_node_old() -> crate::ui_render::ViewNode {
    use crate::ui_render::{PropValue, ViewNode};
    let mut props: BTreeMap<String, PropValue> = BTreeMap::new();
    props.insert("title".to_string(), PropValue::Str("old".to_string()));
    props.insert("count".to_string(), PropValue::Int(3));
    let mut children: Vec<ViewNode> = Vec::new();
    for i in 0..4 {
        let mut child_props: BTreeMap<String, PropValue> = BTreeMap::new();
        child_props.insert("index".to_string(), PropValue::Int(i as i64));
        children.push(ViewNode {
            kind: "Cell".to_string(),
            key: Some(format!("k-{i}")),
            props: child_props,
            children: Vec::new(),
        });
    }
    ViewNode {
        kind: "Card".to_string(),
        key: None,
        props,
        children,
    }
}

fn build_bench_node_new() -> crate::ui_render::ViewNode {
    // Same tree shape as `_old` but with one prop changed at the root so the
    // diff has exactly one op to emit. This keeps the metric focused on the
    // diff machinery itself rather than the prop-mutation cost.
    let mut node = build_bench_node_old();
    node.props.insert(
        "title".to_string(),
        crate::ui_render::PropValue::Str("new".to_string()),
    );
    node
}

const DISPATCH_FIXTURE: &str = "module bench.dispatch\n\
fn get_users() -> Int uses http, db.read\n\
fn post_users() -> Int uses http, db.write\n\
fn get_status() -> Int uses http\n\
fn get_orders() -> Int uses http, db.read\n";

fn build_bench_dispatch_request() -> crate::backend_dispatch::Request {
    use crate::backend_dispatch::{Principal, Request};
    let mut caps: BTreeSet<String> = BTreeSet::new();
    caps.insert("db.read".to_string());
    caps.insert("db.write".to_string());
    let mut req = Request::new("GET", "/users");
    req.principal = Some(Principal {
        id: "bench-principal".to_string(),
        capabilities: caps,
    });
    req
}

fn build_bench_guard_context() -> crate::capability_runtime::CallContext {
    use crate::capability_runtime::{CallContext, CapabilitySet, CapabilityToken};
    let mut tokens: BTreeMap<String, CapabilityToken> = BTreeMap::new();
    tokens.insert(
        "db.read".to_string(),
        CapabilityToken {
            effect: "db.read".to_string(),
            issued_to: "bench-principal".to_string(),
            expires_at: None,
        },
    );
    tokens.insert(
        "db.write".to_string(),
        CapabilityToken {
            effect: "db.write".to_string(),
            issued_to: "bench-principal".to_string(),
            expires_at: None,
        },
    );
    let mut required: BTreeSet<String> = BTreeSet::new();
    required.insert("db.read".to_string());
    required.insert("db.write".to_string());
    CallContext {
        caller_symbol: "sym:bench.handler".to_string(),
        required_effects: required,
        principal_id: "bench-principal".to_string(),
        capabilities: CapabilitySet {
            tokens,
            denials: BTreeSet::new(),
            audit_required: BTreeSet::new(),
        },
    }
}

// Local mirror of the `ori-agent` model_loop envelope shape. We mirror it
// here so the benchmark exercises the same serde_json round-trip pattern
// without pulling `ori-agent` into the bootstrap dependency graph (the
// crate is only a dev-dependency of `ori-compiler`).
const MODEL_LOOP_SCHEMA: &str = "ori.model_loop_telemetry.v1";

#[derive(Serialize, Deserialize, Clone)]
struct BenchLoopIteration {
    iteration: u32,
    started_at: u64,
    completed_at: u64,
    edits_proposed: u32,
    edits_accepted: u32,
    edits_rejected: u32,
    tokens_in: u64,
    tokens_out: u64,
    budget_remaining: u64,
    diagnostics_before: u32,
    diagnostics_after: u32,
}

#[derive(Serialize, Deserialize, Clone)]
struct BenchLoopTotals {
    iterations: u32,
    wall_ms: u64,
    edits_proposed: u32,
    edits_accepted: u32,
    edits_rejected: u32,
    tokens_in: u64,
    tokens_out: u64,
    diagnostics_resolved: i32,
}

#[derive(Serialize, Deserialize, Clone)]
struct BenchLoopTelemetry {
    schema: String,
    session_id: String,
    model_id: String,
    iterations: Vec<BenchLoopIteration>,
    totals: BenchLoopTotals,
}

fn build_bench_model_loop_envelope() -> BenchLoopTelemetry {
    let mut iters: Vec<BenchLoopIteration> = Vec::with_capacity(4);
    for i in 0..4u32 {
        iters.push(BenchLoopIteration {
            iteration: i,
            started_at: 1_000 + (i as u64) * 50,
            completed_at: 1_040 + (i as u64) * 50,
            edits_proposed: 3,
            edits_accepted: 2,
            edits_rejected: 1,
            tokens_in: 256,
            tokens_out: 128,
            budget_remaining: 8_000u64.saturating_sub(((i as u64) + 1) * 384),
            diagnostics_before: 5u32.saturating_sub(i),
            diagnostics_after: 4u32.saturating_sub(i),
        });
    }
    BenchLoopTelemetry {
        schema: MODEL_LOOP_SCHEMA.to_string(),
        session_id: "bench-session".to_string(),
        model_id: "bench-model".to_string(),
        iterations: iters,
        totals: BenchLoopTotals {
            iterations: 4,
            wall_ms: 160,
            edits_proposed: 12,
            edits_accepted: 8,
            edits_rejected: 4,
            tokens_in: 1024,
            tokens_out: 512,
            diagnostics_resolved: 4,
        },
    }
}

fn model_loop_envelope_to_json(env: &BenchLoopTelemetry) -> String {
    crate::json::to_json(env)
}

fn model_loop_envelope_from_json(text: &str) -> Result<BenchLoopTelemetry, String> {
    serde_json::from_str::<BenchLoopTelemetry>(text)
        .map_err(|e| format!("invalid model_loop envelope: {e}"))
}

fn ori_agent_map_for_budget(result: &crate::CompileResult, budget: usize) -> String {
    // Minimal stand-in matching the budget-truncation shape used by ori-agent
    // so the benchmark exercises the same allocation pattern.
    let mut used = 0usize;
    let mut symbols: Vec<&str> = Vec::new();
    for s in &result.module.symbols {
        let est = s.signature.len() + s.id.len() + 48;
        if used + est > budget && !symbols.is_empty() {
            break;
        }
        used += est;
        symbols.push(s.id.as_str());
    }
    format!(
        "{{\"module\":\"{}\",\"budget\":{budget},\"used\":{used},\"n\":{}}}",
        result.module.name,
        symbols.len()
    )
}

const GRAPHQL_FIXTURE: &str = r#"
type Query {
  hello: String!
  user(id: ID!): User
}
type Mutation {
  createUser(name: String!, email: String): User
}
type User {
  id: ID!
  name: String!
  email: String
  posts: [Post!]!
}
type Post {
  id: ID!
  title: String!
  body: String
}
"#;

const PROTO_FIXTURE: &str = r#"
syntax = "proto3";
package bench.rpc;
message User {
  string id = 1;
  string name = 2;
  string email = 3;
}
message Post {
  string id = 1;
  string author = 2;
  string title = 3;
  string body = 4;
}
service Users {
  rpc Get (User) returns (User);
  rpc List (User) returns (stream User);
  rpc Save (stream User) returns (User);
}
"#;

const DEFAULT_SQL: &str = r#"module bench.sql
type UserId
query find_user(id: UserId) -> {id: UserId, name: Str}
query list_users() -> {id: UserId, name: Str}
migration init:
  up "CREATE TABLE users (id text)"
  down "DROP TABLE users"
migration add_index:
  up "CREATE INDEX users_id ON users (id)"
  down "DROP INDEX users_id"
"#;

const PREPROC_FIXTURE: &str = "module x\n// version: @orison/version\nfn f() -> Str";

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
