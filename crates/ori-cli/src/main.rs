use ori_agent::{
    agent_diagnose_json, agent_map_json, agent_symbol_list_json, doctor_report_json,
    explain_symbol_json, parse_telemetry_json, telemetry_json, AgentMapOptions,
};
use ori_compiler::ast::Module;
use ori_compiler::ast::SymbolKind;
use ori_compiler::backend_dispatch::{build_dispatch_table, dispatch_report_json};
use ori_compiler::bench::run_default_suite;
use ori_compiler::body::parse_module_bodies;
use ori_compiler::capability_runtime::{
    guard_call_at, guard_report_json, CallContext, CapabilitySet, CapabilityToken,
};
use ori_compiler::codegen_text::emit_textual_ir;
use ori_compiler::coverage::coverage_for_files;
use ori_compiler::design_tokens::{
    check_module as check_design_tokens, report_to_diagnostics as design_tokens_diagnostics,
    TokenSet,
};
use ori_compiler::docs::{generate_agent_docs, generate_human_docs};
use ori_compiler::effect_check::build_capability_manifest;
use ori_compiler::graphql_import::{
    parse_sdl, to_orison_module as graphql_to_orison_module, ImportReport as GraphqlImportReport,
};
use ori_compiler::hir::lower_module as lower_hir;
use ori_compiler::incremental::select_affected_tests;
use ori_compiler::interp::run_module;
use ori_compiler::interp_exec::{exec_program, Value};
use ori_compiler::json::to_json;
use ori_compiler::migrate::{plan_migration, unsupported_edition_error};
use ori_compiler::migration_graph::build_migration_report;
use ori_compiler::mir::lower_module as lower_mir;
use ori_compiler::mobile::{build_mobile_manifest_with_ui, validate_manifest, SUPPORTED_PLATFORMS};
use ori_compiler::mobile_ui_ir::NativeUiKind;
use ori_compiler::openapi::extract_openapi;
use ori_compiler::patch::check_patch_json;
use ori_compiler::patch_apply::apply_patch;
use ori_compiler::rpc_import::{parse_proto, to_orison_module, RpcImportReport};
use ori_compiler::source::SourceFile;
use ori_compiler::sql_check::check_module_queries;
use ori_compiler::ui_check::build_ui_manifest;
use ori_compiler::wasm_component::build_wasm_component_manifest;
use ori_compiler::wasm_encoder::{encode_from_mir, encode_hello_module};
use ori_compiler::Compiler;
use ori_pkg::{
    build_lockfile, build_sbom, resolve, run_audit, verify_provenance, LocalRegistry, Manifest,
    PackageEntry, PublishReceipt, RegistryError, SbomFormat, REGISTRY_LIST_SCHEMA,
};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || matches!(args[0].as_str(), "help" | "--help" | "-h") {
        print_help();
        return;
    }

    let code = match args[0].as_str() {
        "check" => cmd_check(&args[1..]),
        "fmt" => cmd_fmt(&args[1..]),
        "agent" => cmd_agent(&args[1..]),
        "capsule" => cmd_capsule(&args[1..]),
        "patch" => cmd_patch(&args[1..]),
        "lsp" => cmd_lsp(&args[1..]),
        "package" => cmd_package(&args[1..]),
        "audit" => cmd_audit(&args[1..]),
        "sbom" => cmd_sbom(&args[1..]),
        "provenance" => cmd_provenance(&args[1..]),
        "run" => cmd_run(&args[1..]),
        "build" => cmd_build(&args[1..]),
        "bench" => cmd_bench(&args[1..]),
        "openapi" => cmd_openapi(&args[1..]),
        "ui" => cmd_ui(&args[1..]),
        "design" => cmd_design(&args[1..]),
        "wasm" => cmd_wasm(&args[1..]),
        "capability" => cmd_capability(&args[1..]),
        "test" => cmd_test(&args[1..]),
        "coverage" => cmd_coverage(&args[1..]),
        "docs" => cmd_docs(&args[1..]),
        "migrate" => cmd_migrate(&args[1..]),
        "db" => cmd_db(&args[1..]),
        "publish" => cmd_publish(&args[1..]),
        "fetch" => cmd_fetch(&args[1..]),
        "registry" => cmd_registry(&args[1..]),
        "schema" => cmd_schema(&args[1..]),
        "preprocess" => cmd_preprocess(&args[1..]),
        "serve" => cmd_serve(&args[1..]),
        "doctor" => cmd_doctor(),
        other => {
            eprintln!("unknown command `{other}`");
            print_help();
            2
        }
    };
    if code != 0 {
        process::exit(code);
    }
}

fn print_help() {
    println!("Orison bootstrap CLI");
    println!();
    println!("Usage:");
    println!("  ori check [--json] <file.ori>");
    println!("  ori fmt <file.ori>");
    println!("  ori capsule [--json] <file.ori>");
    println!("  ori agent map [--budget N] [--json] <file.ori>");
    println!("  ori agent explain <symbol-id-or-name> [--json] <file.ori>");
    println!("  ori agent symbols [--changed] [--json] <file.ori>");
    println!("  ori agent diagnose [--json] <file.ori>");
    println!("  ori agent tests --affected [--changed-name <name>...] [--json] <root>");
    println!("  ori agent changed [--prev <prev.json>] [--json] <path>");
    println!("  ori agent telemetry --in <session.json | -> [--json]");
    println!("  ori patch check [--json] <patch.json>");
    println!("  ori patch apply [--dry-run] [--json] <patch.json> <source.ori>");
    println!("  ori patch dry-run [--json] <patch.json> <source.ori>");
    println!("  ori lsp --stdio");
    println!("  ori package check [--json] [path]");
    println!("  ori audit [--json] [path]");
    println!("  ori sbom [--json] [--format <ori-native|spdx|cyclonedx>] [path]");
    println!("  ori provenance verify [--json] <file.json>");
    println!("  ori run [--entry <name>] [--json] <file.ori>");
    println!("  ori build [--target dev|release|wasm-component|llvm-text|mobile] [--app-id <id>] [--platforms ios,android] [--ui-kind ios-uikit|ios-swiftui|android-compose|android-view] [--json] <file.ori>");
    println!("  ori bench [--json] [--samples N]");
    println!("  ori openapi [--json] <file.ori>");
    println!("  ori ui [--json] <file.ori>");
    println!("  ori ui render --dry-run --module <path> --view <symbol>");
    println!("  ori design check [--tokens <file>] [--json] <module.ori>");
    println!("  ori wasm [--json] <file.ori>");
    println!("  ori capability [--policy a,b,c] [--json] <file.ori>");
    println!("  ori capability check --dry-run --module <path> --principal <id> --has <a,b>");
    println!("  ori test [--changed] [--json] <root>");
    println!("  ori coverage [--json] <path>");
    println!("  ori docs [--format human|agent] [--budget N] [--json] [path]");
    println!("  ori migrate --from <X> --to <Y> [--dry-run] [--json] [path]");
    println!("  ori db check [--json] <file.ori>");
    println!("  ori publish --registry <path> --tarball <file> [--json] [path]");
    println!("  ori fetch --registry <path> <name>@<version> [--out <file>] [--json]");
    println!("  ori registry list --registry <path> [--json]");
    println!("  ori registry yank --registry <path> <name>@<version> --reason <r> [--json]");
    println!("  ori schema import grpc <file.proto> --module <name> [--json]");
    println!("  ori schema import graphql <file.graphql> --module <name> [--json]");
    println!("  ori preprocess [--const k=v ...] [--allow-env X,Y] [--json] <file.ori>");
    println!("  ori serve --dry-run --module <file.ori>");
    println!("  ori doctor [--json]");
}

fn cmd_lsp(args: &[String]) -> i32 {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("Usage: ori lsp --stdio");
        println!();
        println!("Run the Orison Language Server over stdin/stdout using LSP base");
        println!("protocol framing (Content-Length headers). Editors and IDEs connect");
        println!("by spawning this command.");
        return 0;
    }
    if !args.iter().any(|arg| arg == "--stdio") {
        eprintln!("usage: ori lsp --stdio");
        return 2;
    }
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    match ori_lsp::Server::new().run(stdin.lock(), stdout.lock()) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("lsp server failed: {err}");
            1
        }
    }
}

fn cmd_check(args: &[String]) -> i32 {
    let json = args.iter().any(|arg| arg == "--json");
    let Some(file) = args.iter().find(|arg| !arg.starts_with('-')) else {
        eprintln!("missing file");
        return 2;
    };
    match Compiler::check_file(file.as_str()) {
        Ok(result) => {
            if json {
                let lines = Compiler::diagnostics_json_lines(&result);
                if !lines.is_empty() {
                    println!("{lines}");
                }
            } else if result.diagnostics.is_empty() {
                println!("ok: {}", result.module.name);
            } else {
                for diagnostic in &result.diagnostics {
                    println!(
                        "{} {}: {}",
                        diagnostic.level.as_str(),
                        diagnostic.id,
                        diagnostic.message
                    );
                }
            }
            if result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.is_error())
            {
                1
            } else {
                0
            }
        }
        Err(err) => {
            eprintln!("failed to read {file}: {err}");
            2
        }
    }
}

fn cmd_fmt(args: &[String]) -> i32 {
    let Some(file) = args.first() else {
        eprintln!("missing file");
        return 2;
    };
    match fs::read_to_string(file.as_str()) {
        Ok(text) => {
            print!("{}", Compiler::format_source(&text));
            0
        }
        Err(err) => {
            eprintln!("failed to read {file}: {err}");
            2
        }
    }
}

fn cmd_capsule(args: &[String]) -> i32 {
    let Some(file) = args.iter().find(|arg| !arg.starts_with('-')) else {
        eprintln!("missing file");
        return 2;
    };
    match Compiler::check_file(file.as_str()) {
        Ok(result) => {
            println!("{}", Compiler::capsule_json(&result));
            if result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.is_error())
            {
                1
            } else {
                0
            }
        }
        Err(err) => {
            eprintln!("failed to read {file}: {err}");
            2
        }
    }
}

fn cmd_agent(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("missing agent subcommand");
        return 2;
    }
    match args[0].as_str() {
        "map" => cmd_agent_map(&args[1..]),
        "explain" => cmd_agent_explain(&args[1..]),
        "capsule" => cmd_capsule(&args[1..]),
        "symbols" => cmd_agent_symbols(&args[1..]),
        "diagnose" => cmd_agent_diagnose(&args[1..]),
        "tests" => cmd_agent_tests(&args[1..]),
        "changed" => cmd_agent_changed(&args[1..]),
        "telemetry" => cmd_agent_telemetry(&args[1..]),
        other => {
            eprintln!("unknown agent subcommand `{other}`");
            2
        }
    }
}

fn cmd_agent_changed(args: &[String]) -> i32 {
    let mut prev_path: Option<String> = None;
    let mut path: Option<String> = None;
    let mut idx = 0usize;
    while idx < args.len() {
        match args[idx].as_str() {
            "--prev" => {
                let Some(value) = args.get(idx + 1) else {
                    eprintln!("--prev requires a value");
                    return 2;
                };
                prev_path = Some(value.clone());
                idx += 2;
            }
            "--json" => idx += 1,
            value if value.starts_with("--") => {
                eprintln!("unknown flag `{value}`");
                return 2;
            }
            value => {
                path = Some(value.to_string());
                idx += 1;
            }
        }
    }
    let Some(path) = path else {
        eprintln!("usage: ori agent changed [--prev <prev.json>] [--json] <path>");
        return 2;
    };
    let target = PathBuf::from(&path);
    let files = collect_ori_files(&target, &["src"]);
    if files.is_empty() {
        eprintln!("no .ori files found under {path}");
        return 2;
    }

    let mut modules: Vec<ori_compiler::ast::Module> = Vec::with_capacity(files.len());
    for (file_path, text) in &files {
        let parse =
            ori_compiler::parser::parse_source(&SourceFile::new(file_path.clone(), text.clone()));
        modules.push(parse.module);
    }
    let resolution = ori_compiler::resolver::resolve(&modules);

    let mut notes: Vec<String> = Vec::new();
    let prev: BTreeMap<String, u64> = match prev_path.as_deref() {
        None => {
            // No explicit --prev was passed: try the default cache location
            // under <path>/.ori/fingerprints.json. A missing file simply means
            // this is the first run, which is not a failure.
            let default = default_fingerprint_path(&target);
            match fs::read_to_string(&default) {
                Ok(text) => match serde_json::from_str::<BTreeMap<String, u64>>(&text) {
                    Ok(map) => map,
                    Err(err) => {
                        notes.push(format!(
                            "previous fingerprints file `{}` was unreadable ({err}); treating all symbols as new",
                            default.display()
                        ));
                        BTreeMap::new()
                    }
                },
                Err(_) => BTreeMap::new(),
            }
        }
        Some(prev_file) => match fs::read_to_string(prev_file) {
            Ok(text) => match serde_json::from_str::<BTreeMap<String, u64>>(&text) {
                Ok(map) => map,
                Err(err) => {
                    notes.push(format!(
                        "previous fingerprints file `{prev_file}` was unreadable ({err}); treating all symbols as new"
                    ));
                    BTreeMap::new()
                }
            },
            Err(err) => {
                notes.push(format!(
                    "previous fingerprints file `{prev_file}` was unreadable ({err}); treating all symbols as new"
                ));
                BTreeMap::new()
            }
        },
    };

    let report =
        ori_compiler::query::build_agent_changed_report(&prev, &modules, &resolution.graph, notes);
    println!("{}", report.to_json());

    // Persist the current fingerprint table for the next run. Best-effort: a
    // read-only filesystem must not break the command.
    let current = ori_compiler::query::combined_fingerprints(&modules);
    let target_cache = default_fingerprint_path(&target);
    persist_fingerprints(&target_cache, &current);

    0
}

fn default_fingerprint_path(target: &Path) -> PathBuf {
    let base = if target.is_file() {
        target
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    } else {
        target.to_path_buf()
    };
    base.join(".ori").join("fingerprints.json")
}

fn persist_fingerprints(path: &Path, table: &BTreeMap<String, u64>) {
    if let Some(parent) = path.parent() {
        if let Err(err) = fs::create_dir_all(parent) {
            eprintln!(
                "warning: could not create fingerprint cache directory {}: {err}",
                parent.display()
            );
            return;
        }
    }
    let serialized = match serde_json::to_string(table) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("warning: could not serialize fingerprint cache: {err}");
            return;
        }
    };
    if let Err(err) = fs::write(path, serialized) {
        eprintln!(
            "warning: could not write fingerprint cache {}: {err}",
            path.display()
        );
    }
}

fn cmd_agent_telemetry(args: &[String]) -> i32 {
    let mut input: Option<String> = None;
    let mut idx = 0usize;
    while idx < args.len() {
        match args[idx].as_str() {
            "--in" => {
                let Some(value) = args.get(idx + 1) else {
                    eprintln!("--in requires a value (path or `-` for stdin)");
                    return 2;
                };
                input = Some(value.clone());
                idx += 2;
            }
            "--json" => idx += 1,
            other => {
                eprintln!("unknown flag `{other}`");
                return 2;
            }
        }
    }
    let Some(input) = input else {
        eprintln!("usage: ori agent telemetry --in <session.json | ->");
        return 2;
    };

    let text = if input == "-" {
        let mut buf = String::new();
        use std::io::Read;
        match std::io::stdin().read_to_string(&mut buf) {
            Ok(_) => buf,
            Err(err) => {
                eprintln!("failed to read stdin: {err}");
                return 2;
            }
        }
    } else {
        match fs::read_to_string(&input) {
            Ok(text) => text,
            Err(err) => {
                eprintln!("failed to read {input}: {err}");
                return 2;
            }
        }
    };

    match parse_telemetry_json(&text) {
        Ok(envelope) => {
            // Re-serialise so the emitted envelope is canonical (totals
            // recomputed, fields in declaration order).
            println!("{}", telemetry_json(&envelope));
            0
        }
        Err(err) => {
            eprintln!("invalid model-loop telemetry: {err}");
            1
        }
    }
}

fn cmd_agent_symbols(args: &[String]) -> i32 {
    let changed = args.iter().any(|arg| arg == "--changed");
    let Some(file) = positional_file(args) else {
        eprintln!("usage: ori agent symbols [--changed] [--json] <file.ori>");
        return 2;
    };
    match Compiler::check_file(&file) {
        Ok(result) => {
            println!("{}", agent_symbol_list_json(&result, changed));
            0
        }
        Err(err) => {
            eprintln!("failed to read {file}: {err}");
            2
        }
    }
}

fn cmd_agent_diagnose(args: &[String]) -> i32 {
    let Some(file) = positional_file(args) else {
        eprintln!("usage: ori agent diagnose [--json] <file.ori>");
        return 2;
    };
    match Compiler::check_file(&file) {
        Ok(result) => {
            println!("{}", agent_diagnose_json(&result));
            0
        }
        Err(err) => {
            eprintln!("failed to read {file}: {err}");
            2
        }
    }
}

fn cmd_agent_tests(args: &[String]) -> i32 {
    if !args.iter().any(|arg| arg == "--affected") {
        eprintln!("usage: ori agent tests --affected [--changed-name <name>] [--json] <root>");
        return 2;
    }
    let mut changed_names: Vec<String> = Vec::new();
    let mut iter = args.iter();
    let mut root: Option<String> = None;
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--affected" | "--json" => {}
            "--changed-name" => {
                if let Some(name) = iter.next() {
                    changed_names.push(name.clone());
                }
            }
            value if !value.starts_with("--") => root = Some(value.to_string()),
            _ => {}
        }
    }
    let root = root.unwrap_or_else(|| ".".to_string());
    let test_files = collect_ori_files(Path::new(&root), &["tests"]);
    let report = select_affected_tests(&test_files, &changed_names);
    println!("{}", report.to_json());
    0
}

fn cmd_run(args: &[String]) -> i32 {
    let mut entry: Option<String> = None;
    let mut json = false;
    let mut file: Option<String> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--entry" => {
                if let Some(value) = iter.next() {
                    entry = Some(value.clone());
                }
            }
            "--json" => json = true,
            value if !value.starts_with("--") => file = Some(value.to_string()),
            value => {
                eprintln!("unknown flag `{value}`");
                return 2;
            }
        }
    }
    let Some(file) = file else {
        eprintln!("usage: ori run [--entry name] [--json] <file.ori>");
        return 2;
    };
    match Compiler::check_file(&file) {
        Ok(result) => {
            // Resolve which entrypoint we'd hand to the executor first; we
            // mirror the legacy `run_module` priority order so behaviour is
            // stable across the two backends.
            let entry_name = pick_entry_name(&result.module, entry.as_deref());
            let bodies = match fs::read_to_string(&file) {
                Ok(text) => {
                    let source = SourceFile::new(file.clone(), text);
                    Some(parse_module_bodies(&source))
                }
                Err(_) => None,
            };
            let has_executable_body = bodies
                .as_ref()
                .and_then(|b| {
                    result
                        .module
                        .symbols
                        .iter()
                        .find(|s| s.name == entry_name)
                        .and_then(|sym| b.get(&sym.id))
                })
                .map(|expr| {
                    !matches!(
                        expr,
                        ori_compiler::expr::Expr::Lit(ori_compiler::expr::Literal::Unit)
                    )
                })
                .unwrap_or(false);

            if let (true, Some(parsed_bodies)) = (has_executable_body, bodies.as_ref()) {
                match exec_program(&result.module, parsed_bodies, &entry_name, Vec::new()) {
                    Ok(value) => {
                        if json {
                            let body = serde_json::json!({
                                "schema": "ori.run.v1",
                                "module": result.module.name,
                                "entry": entry_name,
                                "status": "ok",
                                "value": format_value(&value),
                            });
                            println!("{body}");
                        } else {
                            println!("status: ok");
                            println!("entry:  {entry_name}");
                            println!("value:  {}", format_value(&value));
                        }
                        return 0;
                    }
                    Err(err) => {
                        if json {
                            let body = serde_json::json!({
                                "schema": "ori.run.v1",
                                "module": result.module.name,
                                "entry": entry_name,
                                "status": "error",
                                "error_code": err.code,
                                "error_message": err.message,
                                "effects_observed": err.observed_effects,
                            });
                            println!("{body}");
                        } else {
                            println!("status: error");
                            println!("entry:  {entry_name}");
                            println!("error:  {} {}", err.code, err.message);
                            if !err.observed_effects.is_empty() {
                                println!("effects: {}", err.observed_effects.join(", "));
                            }
                        }
                        return 1;
                    }
                }
            }

            // No executable body available — fall back to the legacy
            // effect-only run report.
            let report = run_module(&result.module, entry.as_deref());
            if json {
                println!("{}", report.to_json());
            } else {
                println!("status: {}", report.status);
                println!("entry:  {}", report.entry);
                if !report.effects_observed.is_empty() {
                    println!("effects: {}", report.effects_observed.join(", "));
                }
                if !report.stdout.is_empty() {
                    print!("{}", report.stdout);
                }
            }
            if report.status == "missing_entry" {
                1
            } else {
                0
            }
        }
        Err(err) => {
            eprintln!("failed to read {file}: {err}");
            2
        }
    }
}

/// Render a [`Value`] as a short, human-readable string for `ori run`'s
/// non-JSON output. The canonical machine-readable form lives in the JSON
/// envelope; this helper exists only to keep the human view legible.
fn format_value(value: &Value) -> String {
    match value {
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Str(s) => format!("\"{s}\""),
        Value::Unit => "Unit".to_string(),
        Value::None_ => "None".to_string(),
        Value::Some_(v) => format!("Some({})", format_value(v)),
        Value::Ok_(v) => format!("Ok({})", format_value(v)),
        Value::Err_(v) => format!("Err({})", format_value(v)),
        Value::List(items) => {
            let parts: Vec<String> = items.iter().map(format_value).collect();
            format!("[{}]", parts.join(", "))
        }
        Value::Record(fields) => {
            let parts: Vec<String> = fields
                .iter()
                .map(|(k, v)| format!("{k}: {}", format_value(v)))
                .collect();
            format!("{{ {} }}", parts.join(", "))
        }
    }
}

/// Resolve the entrypoint name we'd target for `ori run`, mirroring the
/// fallback chain used by the legacy `run_module` reporter so both backends
/// agree on which symbol is "the entry".
fn pick_entry_name(module: &Module, requested: Option<&str>) -> String {
    let candidates: Vec<&str> = match requested {
        Some(name) => vec![name],
        None => vec!["main", "boot", "run", "start"],
    };
    for name in &candidates {
        if module
            .symbols
            .iter()
            .any(|s| s.name == *name && matches!(s.kind, ori_compiler::ast::SymbolKind::Function))
        {
            return (*name).to_string();
        }
    }
    candidates.first().copied().unwrap_or("main").to_string()
}

fn cmd_build(args: &[String]) -> i32 {
    let mut target = "dev".to_string();
    let mut json = false;
    let mut file: Option<String> = None;
    let mut app_id: Option<String> = None;
    let mut platforms: Vec<String> = Vec::new();
    let mut ui_kind: Option<NativeUiKind> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--target" => {
                if let Some(value) = iter.next() {
                    target = value.clone();
                }
            }
            "--app-id" => {
                if let Some(value) = iter.next() {
                    app_id = Some(value.clone());
                }
            }
            "--platforms" => {
                if let Some(value) = iter.next() {
                    platforms = value
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
            }
            "--ui-kind" => {
                if let Some(value) = iter.next() {
                    match NativeUiKind::from_cli_str(value.as_str()) {
                        Some(kind) => ui_kind = Some(kind),
                        None => {
                            eprintln!(
                                "unknown --ui-kind `{value}` (expected ios-uikit|ios-swiftui|android-compose|android-view)"
                            );
                            return 2;
                        }
                    }
                }
            }
            "--json" => json = true,
            value if !value.starts_with("--") => file = Some(value.to_string()),
            value => {
                eprintln!("unknown flag `{value}`");
                return 2;
            }
        }
    }
    let Some(file) = file else {
        eprintln!(
            "usage: ori build [--target dev|release|wasm-component|llvm-text|mobile] [--app-id <id>] [--platforms ios,android] [--ui-kind ios-uikit|ios-swiftui|android-compose|android-view] [--json] <file.ori>"
        );
        return 2;
    };
    let start = std::time::Instant::now();
    match Compiler::check_file(&file) {
        Ok(result) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            let errors = result.diagnostics.iter().filter(|d| d.is_error()).count();
            let warnings = result.diagnostics.len() - errors;
            let mut outputs: Vec<serde_json::Value> = Vec::new();
            let mut emit_warnings: Vec<String> = Vec::new();
            match target.as_str() {
                "mobile" => {
                    let app_id_value = app_id
                        .clone()
                        .unwrap_or_else(|| format!("app.{}", result.module.name));
                    let platforms_default: Vec<String> = if platforms.is_empty() {
                        SUPPORTED_PLATFORMS
                            .iter()
                            .map(|s| (*s).to_string())
                            .collect()
                    } else {
                        platforms.clone()
                    };
                    let platforms_ref: Vec<&str> =
                        platforms_default.iter().map(|s| s.as_str()).collect();
                    let manifest = build_mobile_manifest_with_ui(
                        &result.module,
                        &app_id_value,
                        &platforms_ref,
                        ui_kind,
                    );
                    let mobile_diags = validate_manifest(&manifest, &file);
                    let output_path = format!("{file}.mobile.json");
                    let manifest_json = manifest.to_json();
                    let byte_count = manifest_json.len();
                    let manifest_value: serde_json::Value =
                        serde_json::from_str(&manifest_json).unwrap_or(serde_json::Value::Null);
                    match fs::write(&output_path, &manifest_json) {
                        Ok(()) => {
                            outputs.push(serde_json::json!({
                                "kind": "mobile",
                                "path": output_path,
                                "byte_count": byte_count,
                                "manifest": manifest_value,
                                "diagnostics": mobile_diags,
                            }));
                        }
                        Err(err) => {
                            emit_warnings.push(format!("failed to write {output_path}: {err}"));
                        }
                    }
                    for diag in &mobile_diags {
                        if diag.is_error() {
                            emit_warnings.push(format!("mobile {}: {}", diag.id, diag.message));
                        }
                    }
                }
                "wasm-component" => {
                    let output_path = format!("{file}.wasm");
                    let bytes = build_wasm_bytes(&result.module);
                    match fs::write(&output_path, &bytes) {
                        Ok(()) => {
                            outputs.push(serde_json::json!({
                                "kind": "wasm-component",
                                "path": output_path,
                                "byte_count": bytes.len(),
                            }));
                        }
                        Err(err) => {
                            emit_warnings.push(format!("failed to write {output_path}: {err}"));
                        }
                    }
                }
                "llvm-text" => {
                    let output_path = format!("{file}.ll");
                    let hir = lower_hir(&result.module);
                    let mir = lower_mir(&hir);
                    let text = emit_textual_ir(&mir);
                    let byte_count = text.len();
                    match fs::write(&output_path, &text) {
                        Ok(()) => {
                            outputs.push(serde_json::json!({
                                "kind": "llvm-text",
                                "path": output_path,
                                "byte_count": byte_count,
                            }));
                        }
                        Err(err) => {
                            emit_warnings.push(format!("failed to write {output_path}: {err}"));
                        }
                    }
                }
                _ => {}
            }
            let report = serde_json::json!({
                "schema": "ori.build_report.v1",
                "package": result.module.name,
                "target": target,
                "duration_ms": duration_ms,
                "units_compiled": 1,
                "cached_units": 0,
                "errors": errors,
                "warnings": warnings,
                "outputs": outputs,
                "emit_warnings": emit_warnings,
            });
            if json {
                println!("{report}");
            } else {
                println!(
                    "build {} [target={}] errors={} warnings={} in {}ms",
                    result.module.name, target, errors, warnings, duration_ms
                );
                for output in &outputs {
                    if let (Some(path), Some(bytes)) = (
                        output.get("path").and_then(|v| v.as_str()),
                        output.get("byte_count").and_then(|v| v.as_u64()),
                    ) {
                        println!("  wrote {path} ({bytes} bytes)");
                    }
                }
                for warning in &emit_warnings {
                    eprintln!("  warning: {warning}");
                }
            }
            if errors > 0 {
                1
            } else {
                0
            }
        }
        Err(err) => {
            eprintln!("failed to read {file}: {err}");
            2
        }
    }
}

/// Lower the parsed module to MIR and try the MIR encoder; fall back to the
/// canned hello module if MIR is empty or the encoder cannot handle the
/// shape yet. The bootstrap MIR currently produces shapes the encoder
/// rejects (placeholder return types like `Unit`/`View`), so a deterministic
/// fallback keeps the artefact path alive.
fn build_wasm_bytes(module: &Module) -> Vec<u8> {
    let hir = lower_hir(module);
    let mir = lower_mir(&hir);
    match encode_from_mir(&mir) {
        Ok(bytes) if !bytes.is_empty() && bytes.len() > 8 => bytes,
        _ => encode_hello_module(),
    }
}

fn cmd_bench(args: &[String]) -> i32 {
    let mut samples = 5usize;
    let mut json = true;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--samples" => {
                if let Some(value) = iter.next() {
                    if let Ok(n) = value.parse::<usize>() {
                        samples = n.max(3);
                    }
                }
            }
            "--json" => json = true,
            "--no-json" => json = false,
            other => {
                eprintln!("unknown flag `{other}`");
                return 2;
            }
        }
    }
    let report = run_default_suite(samples);
    if json {
        println!("{}", report.to_json());
    } else {
        println!("orison bench: {} suites", report.suites.len());
        for suite in &report.suites {
            for metric in &suite.metrics {
                println!(
                    "  {} {} mean={:.0}{} p50={:.0}{} p95={:.0}{} (n={})",
                    suite.name,
                    metric.key,
                    metric.mean,
                    metric.unit,
                    metric.p50,
                    metric.unit,
                    metric.p95,
                    metric.unit,
                    metric.samples
                );
            }
        }
    }
    0
}

fn cmd_openapi(args: &[String]) -> i32 {
    let Some(file) = positional_file(args) else {
        eprintln!("usage: ori openapi [--json] <file.ori>");
        return 2;
    };
    match Compiler::check_file(&file) {
        Ok(result) => {
            println!("{}", extract_openapi(&result.module).to_json());
            0
        }
        Err(err) => {
            eprintln!("failed to read {file}: {err}");
            2
        }
    }
}

fn cmd_ui(args: &[String]) -> i32 {
    // `ori ui [--json] <file.ori>`   (legacy)
    // `ori ui render --dry-run --module <path> --view <symbol>`   (M29)
    //
    // Dispatch on the first positional: when it matches a known
    // subcommand we delegate; otherwise we fall back to the legacy
    // manifest emit so existing scripts keep working.
    if let Some(first) = args.first() {
        if first == "render" {
            return cmd_ui_render(&args[1..]);
        }
    }
    let Some(file) = positional_file(args) else {
        eprintln!("usage: ori ui [--json] <file.ori>");
        eprintln!("       ori ui render --dry-run --module <path> --view <symbol>");
        return 2;
    };
    match Compiler::check_file(&file) {
        Ok(result) => {
            println!("{}", build_ui_manifest(&result.module).to_json());
            0
        }
        Err(err) => {
            eprintln!("failed to read {file}: {err}");
            2
        }
    }
}

fn cmd_ui_render(args: &[String]) -> i32 {
    let mut dry_run = false;
    let mut module_path: Option<String> = None;
    let mut view_query: Option<String> = None;
    let mut idx = 0usize;
    while idx < args.len() {
        match args[idx].as_str() {
            "--dry-run" => {
                dry_run = true;
                idx += 1;
            }
            "--module" => {
                let Some(value) = args.get(idx + 1) else {
                    eprintln!("--module requires a value");
                    return 2;
                };
                module_path = Some(value.clone());
                idx += 2;
            }
            "--view" => {
                let Some(value) = args.get(idx + 1) else {
                    eprintln!("--view requires a value");
                    return 2;
                };
                view_query = Some(value.clone());
                idx += 2;
            }
            other => {
                eprintln!("unknown flag `{other}`");
                return 2;
            }
        }
    }
    if !dry_run {
        eprintln!("usage: ori ui render --dry-run --module <path> --view <symbol>");
        return 2;
    }
    let Some(module_path) = module_path else {
        eprintln!("--module is required");
        return 2;
    };
    let Some(view_query) = view_query else {
        eprintln!("--view is required");
        return 2;
    };

    let result = match Compiler::check_file(&module_path) {
        Ok(r) => r,
        Err(err) => {
            eprintln!("failed to read {module_path}: {err}");
            return 2;
        }
    };

    let manifest = build_ui_manifest(&result.module);
    let entry = manifest
        .views
        .iter()
        .find(|v| v.symbol == view_query || v.name == view_query);
    let Some(entry) = entry else {
        eprintln!(
            "view `{view_query}` not found in module `{}`",
            result.module.name
        );
        return 2;
    };

    // Lower the manifest prop list into PropSlots. The bootstrap manifest
    // does not yet carry kind information beyond the printed type string,
    // so every slot is treated as an optional `str` for dry-run purposes.
    // This keeps the contract honest: dry-runs always succeed even when
    // the source-level types are richer than the runtime can represent.
    let slots: Vec<ori_compiler::ui_render::PropSlot> = entry
        .props
        .iter()
        .map(|p| ori_compiler::ui_render::PropSlot::optional(p.name.clone(), "str"))
        .collect();

    let view = ori_compiler::ui_render::ViewDecl::placeholder(entry.name.clone(), slots)
        .with_symbol(entry.symbol.clone());

    match ori_compiler::ui_render::render_view(&view, &std::collections::BTreeMap::new()) {
        Ok(tree) => {
            println!("{}", ori_compiler::ui_render::render_report_json(&tree));
            0
        }
        Err(err) => {
            eprintln!("render failed: {err}");
            1
        }
    }
}

fn cmd_design(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("usage: ori design check [--tokens <file>] [--json] <module.ori>");
        return 2;
    }
    match args[0].as_str() {
        "check" => cmd_design_check(&args[1..]),
        other => {
            eprintln!("unknown design subcommand `{other}`");
            2
        }
    }
}

fn cmd_design_check(args: &[String]) -> i32 {
    let mut tokens_path: Option<String> = None;
    let mut json = false;
    let mut file: Option<String> = None;
    let mut idx = 0usize;
    while idx < args.len() {
        match args[idx].as_str() {
            "--tokens" => {
                let Some(value) = args.get(idx + 1) else {
                    eprintln!("--tokens requires a value");
                    return 2;
                };
                tokens_path = Some(value.clone());
                idx += 2;
            }
            "--json" => {
                json = true;
                idx += 1;
            }
            value if value.starts_with("--") => {
                eprintln!("unknown flag `{value}`");
                return 2;
            }
            value => {
                file = Some(value.to_string());
                idx += 1;
            }
        }
    }
    let Some(file) = file else {
        eprintln!("usage: ori design check [--tokens <file>] [--json] <module.ori>");
        return 2;
    };
    let token_set = match tokens_path.as_deref() {
        Some(path) => match fs::read_to_string(path) {
            Ok(text) => match TokenSet::from_toml_subset(&text) {
                Ok(set) => set,
                Err(err) => {
                    eprintln!("failed to parse tokens file {path}: {err}");
                    return 2;
                }
            },
            Err(err) => {
                eprintln!("failed to read tokens file {path}: {err}");
                return 2;
            }
        },
        None => TokenSet::default(),
    };
    match Compiler::check_file(&file) {
        Ok(result) => {
            let report = check_design_tokens(&result.module, &token_set);
            let diagnostics = design_tokens_diagnostics(&result.module, &report);
            if json {
                let body = serde_json::json!({
                    "schema": "ori.design_check.v1",
                    "module": result.module.name,
                    "diagnostics": diagnostics,
                    "report": report,
                });
                println!("{}", to_json(&body));
            } else {
                println!("module: {}", result.module.name);
                for diag in &diagnostics {
                    println!("  {} {}: {}", diag.level.as_str(), diag.id, diag.message);
                }
                println!("{}", report.to_json());
            }
            if diagnostics.iter().any(|d| d.is_error()) {
                1
            } else {
                0
            }
        }
        Err(err) => {
            eprintln!("failed to read {file}: {err}");
            2
        }
    }
}

fn cmd_schema(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("usage: ori schema import grpc <file.proto> --module <name> [--json]");
        return 2;
    }
    match args[0].as_str() {
        "import" => cmd_schema_import(&args[1..]),
        other => {
            eprintln!("unknown schema subcommand `{other}`");
            2
        }
    }
}

fn cmd_schema_import(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("usage: ori schema import <grpc|graphql> <file> --module <name> [--json]");
        return 2;
    }
    match args[0].as_str() {
        "grpc" => cmd_schema_import_grpc(&args[1..]),
        "graphql" => cmd_schema_import_graphql(&args[1..]),
        other => {
            eprintln!("unknown schema import format `{other}`");
            2
        }
    }
}

fn cmd_schema_import_grpc(args: &[String]) -> i32 {
    let mut json = false;
    let mut module: Option<String> = None;
    let mut file: Option<String> = None;
    let mut idx = 0usize;
    while idx < args.len() {
        match args[idx].as_str() {
            "--json" => {
                json = true;
                idx += 1;
            }
            "--module" => {
                let Some(value) = args.get(idx + 1) else {
                    eprintln!("--module requires a value");
                    return 2;
                };
                module = Some(value.clone());
                idx += 2;
            }
            value if value.starts_with("--") => {
                eprintln!("unknown flag `{value}`");
                return 2;
            }
            value => {
                file = Some(value.to_string());
                idx += 1;
            }
        }
    }
    let Some(file) = file else {
        eprintln!("usage: ori schema import grpc <file.proto> --module <name> [--json]");
        return 2;
    };
    let Some(module) = module else {
        eprintln!("--module <name> is required");
        return 2;
    };
    let text = match fs::read_to_string(&file) {
        Ok(t) => t,
        Err(err) => {
            eprintln!("failed to read {file}: {err}");
            return 2;
        }
    };
    let proto = match parse_proto(&text) {
        Ok(p) => p,
        Err(err) => {
            if json {
                let body = serde_json::json!({
                    "schema": "ori.rpc_import.v1",
                    "ok": false,
                    "file": file,
                    "error_code": err.code,
                    "error_message": err.message,
                    "line": err.line,
                });
                println!("{body}");
            } else {
                eprintln!("proto import failed: {err}");
            }
            return 1;
        }
    };
    let report = RpcImportReport::from_proto(&proto);
    let module_text = to_orison_module(&proto, &module);
    if json {
        let body = serde_json::json!({
            "schema": report.schema,
            "ok": true,
            "file": file,
            "module": module,
            "package": report.package,
            "messages": report.messages,
            "services": report.services,
            "rpcs": report.rpcs,
            "orison": module_text,
        });
        println!("{body}");
    } else {
        print!("{module_text}");
        if !module_text.ends_with('\n') {
            println!();
        }
        eprintln!(
            "imported {} message(s), {} service(s), {} rpc(s) from {} (package: {})",
            report.messages,
            report.services,
            report.rpcs,
            file,
            if report.package.is_empty() {
                "<none>"
            } else {
                report.package.as_str()
            }
        );
    }
    0
}

fn cmd_schema_import_graphql(args: &[String]) -> i32 {
    let mut json = false;
    let mut module: Option<String> = None;
    let mut file: Option<String> = None;
    let mut idx = 0usize;
    while idx < args.len() {
        match args[idx].as_str() {
            "--json" => {
                json = true;
                idx += 1;
            }
            "--module" => {
                let Some(value) = args.get(idx + 1) else {
                    eprintln!("--module requires a value");
                    return 2;
                };
                module = Some(value.clone());
                idx += 2;
            }
            value if value.starts_with("--") => {
                eprintln!("unknown flag `{value}`");
                return 2;
            }
            value => {
                file = Some(value.to_string());
                idx += 1;
            }
        }
    }
    let Some(file) = file else {
        eprintln!("usage: ori schema import graphql <file.graphql> --module <name> [--json]");
        return 2;
    };
    let Some(module) = module else {
        eprintln!("--module <name> is required");
        return 2;
    };
    let text = match fs::read_to_string(&file) {
        Ok(t) => t,
        Err(err) => {
            eprintln!("failed to read {file}: {err}");
            return 2;
        }
    };
    let schema = match parse_sdl(&text) {
        Ok(s) => s,
        Err(err) => {
            if json {
                let body = serde_json::json!({
                    "schema": "ori.graphql_import.v1",
                    "ok": false,
                    "file": file,
                    "module": module,
                    "error_message": err.message,
                    "line": err.line,
                });
                println!("{body}");
            } else {
                eprintln!("graphql import failed: {err}");
            }
            return 1;
        }
    };
    let module_text = graphql_to_orison_module(&schema, &module);
    let report = GraphqlImportReport::build(&schema, &module, &module_text);
    if json {
        let body = serde_json::json!({
            "schema": report.schema,
            "ok": true,
            "file": file,
            "module": report.module,
            "types": report.types,
            "queries": report.queries,
            "mutations": report.mutations,
            "generated_lines": report.generated_lines,
            "unsupported": report.unsupported,
            "orison": module_text,
        });
        println!("{body}");
    } else {
        print!("{module_text}");
        if !module_text.ends_with('\n') {
            println!();
        }
        eprintln!(
            "imported {} type(s), {} query/queries, {} mutation(s) from {} (module: {})",
            report.types, report.queries, report.mutations, file, report.module
        );
        if !report.unsupported.is_empty() {
            eprintln!(
                "warning: {} unsupported declaration(s) ignored: {}",
                report.unsupported.len(),
                report.unsupported.join(", ")
            );
        }
    }
    0
}

fn cmd_wasm(args: &[String]) -> i32 {
    let Some(file) = positional_file(args) else {
        eprintln!("usage: ori wasm [--json] <file.ori>");
        return 2;
    };
    match Compiler::check_file(&file) {
        Ok(result) => {
            println!(
                "{}",
                build_wasm_component_manifest(&result.module).to_json()
            );
            0
        }
        Err(err) => {
            eprintln!("failed to read {file}: {err}");
            2
        }
    }
}

fn cmd_capability(args: &[String]) -> i32 {
    // `ori capability check ...` dispatches to the M35 runtime guard
    // dry-run. Every other invocation keeps the legacy
    // `ori capability [--policy ...] <file.ori>` shape so existing
    // tooling keeps working.
    if let Some(first) = args.first() {
        if first == "check" {
            return cmd_capability_check(&args[1..]);
        }
    }
    let mut policy: Vec<String> = Vec::new();
    let mut file: Option<String> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--policy" => {
                if let Some(value) = iter.next() {
                    policy = value
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
            }
            "--json" => {}
            value if !value.starts_with("--") => file = Some(value.to_string()),
            value => {
                eprintln!("unknown flag `{value}`");
                return 2;
            }
        }
    }
    let Some(file) = file else {
        eprintln!("usage: ori capability [--policy a,b,c] [--json] <file.ori>");
        return 2;
    };
    match Compiler::check_file(&file) {
        Ok(result) => {
            let manifest = build_capability_manifest(&result.module, &policy);
            println!("{}", manifest.to_json());
            0
        }
        Err(err) => {
            eprintln!("failed to read {file}: {err}");
            2
        }
    }
}

/// `ori capability check --dry-run --module <path> --principal <id> --has <effect>,<effect>`
///
/// Walks every Service- and Route-style symbol in the module, builds a
/// [`CallContext`] per route using the principal id and the effects the
/// principal currently `--has`, runs [`guard_call_at`] with `now=0` so
/// expired tokens still produce stable output, and prints a single
/// `ori.capability_runtime.v1` JSON envelope to stdout.
fn cmd_capability_check(args: &[String]) -> i32 {
    let mut module_path: Option<String> = None;
    let mut principal: Option<String> = None;
    let mut has: Vec<String> = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--dry-run" => {}
            "--json" => {}
            "--module" => {
                if let Some(value) = iter.next() {
                    module_path = Some(value.clone());
                }
            }
            "--principal" => {
                if let Some(value) = iter.next() {
                    principal = Some(value.clone());
                }
            }
            "--has" => {
                if let Some(value) = iter.next() {
                    has = value
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
            }
            value if !value.starts_with("--") && module_path.is_none() => {
                module_path = Some(value.to_string());
            }
            value => {
                eprintln!("unknown flag `{value}`");
                return 2;
            }
        }
    }
    let Some(module_path) = module_path else {
        eprintln!(
            "usage: ori capability check --dry-run --module <path> --principal <id> --has <a,b>"
        );
        return 2;
    };
    let principal_id = principal.unwrap_or_else(|| "anonymous".to_string());
    let result = match Compiler::check_file(&module_path) {
        Ok(r) => r,
        Err(err) => {
            eprintln!("failed to read {module_path}: {err}");
            return 2;
        }
    };

    // Build the capability set from the comma-separated --has list. Each
    // effect gets a token issued to the same principal with no expiry,
    // so guard_call_at with now=0 will report Allowed iff every required
    // effect is in the held set.
    let mut tokens: BTreeMap<String, CapabilityToken> = BTreeMap::new();
    for effect in &has {
        tokens.insert(
            effect.clone(),
            CapabilityToken {
                effect: effect.clone(),
                issued_to: principal_id.clone(),
                expires_at: None,
            },
        );
    }
    let cap_set = CapabilitySet {
        tokens,
        denials: std::collections::BTreeSet::new(),
        audit_required: std::collections::BTreeSet::new(),
    };

    // Each route/service symbol with declared effects yields one
    // CallContext; symbols with no effects are skipped because there is
    // nothing to guard.
    let mut outcomes = Vec::new();
    for symbol in &result.module.symbols {
        let is_routeish = matches!(
            symbol.kind,
            SymbolKind::Function | SymbolKind::Service | SymbolKind::Query
        );
        if !is_routeish {
            continue;
        }
        if symbol.effects.is_empty() {
            continue;
        }
        let required: std::collections::BTreeSet<String> = symbol.effects.iter().cloned().collect();
        let ctx = CallContext {
            caller_symbol: symbol.id.clone(),
            required_effects: required,
            principal_id: principal_id.clone(),
            capabilities: cap_set.clone(),
        };
        outcomes.push(guard_call_at(&ctx, 0));
    }
    println!("{}", guard_report_json(&outcomes));
    0
}

fn cmd_test(args: &[String]) -> i32 {
    let root = positional_file(args).unwrap_or_else(|| ".".to_string());
    let test_files = collect_ori_files(Path::new(&root), &["tests", "examples"]);
    // Bootstrap: when --changed is requested but no oracle is wired in, we
    // run all tests we can find.
    let report = select_affected_tests(&test_files, &[]);
    println!("{}", report.to_json());
    0
}

fn cmd_coverage(args: &[String]) -> i32 {
    let mut json = false;
    let mut path: Option<String> = None;
    for arg in args {
        match arg.as_str() {
            "--json" => json = true,
            value if value.starts_with("--") => {
                eprintln!("unknown flag `{value}`");
                return 2;
            }
            value => path = Some(value.to_string()),
        }
    }
    let Some(path) = path else {
        eprintln!("usage: ori coverage [--json] <path>");
        return 2;
    };
    let root = PathBuf::from(&path);
    let impl_files = collect_ori_files(&root.join("src"), &[]);
    let test_files = collect_ori_files(&root.join("tests"), &[]);
    let mut report = coverage_for_files(&impl_files, &test_files);
    report.root = path.clone();
    if json {
        println!("{}", report.to_json());
    } else {
        println!(
            "coverage: {}/{} functions covered ({:.1}%)",
            report.covered.len(),
            report.total_functions,
            report.percent
        );
        if !report.uncovered.is_empty() {
            println!("uncovered:");
            for entry in &report.uncovered {
                println!("  - {} ({})", entry.id, entry.kind);
            }
        }
    }
    0
}

fn positional_file(args: &[String]) -> Option<String> {
    args.iter()
        .rfind(|arg| !arg.starts_with("--"))
        .map(|s| s.to_string())
}

fn collect_ori_files(root: &Path, prefer_dirs: &[&str]) -> BTreeMap<String, String> {
    fn walk(dir: &Path, acc: &mut BTreeMap<String, String>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, acc);
            } else if path.extension().and_then(|s| s.to_str()) == Some("ori") {
                if let Ok(text) = fs::read_to_string(&path) {
                    acc.insert(path.to_string_lossy().into_owned(), text);
                }
            }
        }
    }
    let mut acc = BTreeMap::new();
    // If the user pointed at a single file, just read it.
    if root.is_file() {
        if let Ok(text) = fs::read_to_string(root) {
            acc.insert(root.to_string_lossy().into_owned(), text);
        }
        return acc;
    }
    // Prefer subdirectories first (more conventional layouts).
    for sub in prefer_dirs {
        let p = root.join(sub);
        if p.is_dir() {
            walk(&p, &mut acc);
        }
    }
    if acc.is_empty() {
        walk(root, &mut acc);
    }
    acc
}

fn cmd_agent_map(args: &[String]) -> i32 {
    let mut budget = 4000usize;
    let mut file: Option<String> = None;
    let mut idx = 0usize;
    while idx < args.len() {
        match args[idx].as_str() {
            "--budget" => {
                let Some(value) = args.get(idx + 1) else {
                    eprintln!("--budget requires a value");
                    return 2;
                };
                match value.parse::<usize>() {
                    Ok(parsed) if parsed > 0 => budget = parsed,
                    _ => {
                        eprintln!("--budget must be a positive integer");
                        return 2;
                    }
                }
                idx += 2;
            }
            "--json" => idx += 1,
            value if value.starts_with('-') => {
                eprintln!("unknown flag `{value}`");
                return 2;
            }
            value => {
                file = Some(value.to_string());
                idx += 1;
            }
        }
    }
    let Some(file) = file else {
        eprintln!("missing file");
        return 2;
    };
    match Compiler::check_file(file.as_str()) {
        Ok(result) => {
            println!("{}", agent_map_json(&result, AgentMapOptions { budget }));
            0
        }
        Err(err) => {
            eprintln!("failed to read {file}: {err}");
            2
        }
    }
}

fn cmd_agent_explain(args: &[String]) -> i32 {
    if args.len() < 2 {
        eprintln!("usage: ori agent explain <symbol-id-or-name> [--json] <file.ori>");
        return 2;
    }
    let symbol = &args[0];
    let file = args
        .iter()
        .skip(1)
        .rev()
        .find(|arg| arg.as_str() != "--json")
        .cloned();
    let Some(file) = file else {
        eprintln!("missing file");
        return 2;
    };
    match Compiler::check_file(file.as_str()) {
        Ok(result) => {
            println!("{}", explain_symbol_json(&result, symbol));
            0
        }
        Err(err) => {
            eprintln!("failed to read {file}: {err}");
            2
        }
    }
}

fn cmd_patch(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("usage: ori patch <check|apply|dry-run|explain> ...");
        return 2;
    }
    match args[0].as_str() {
        "check" => cmd_patch_check(&args[1..]),
        "apply" => cmd_patch_apply(&args[1..], false),
        "dry-run" => cmd_patch_apply(&args[1..], true),
        "explain" => cmd_patch_explain(&args[1..]),
        other => {
            eprintln!("unknown patch subcommand `{other}`");
            2
        }
    }
}

fn cmd_patch_check(args: &[String]) -> i32 {
    let file = args.iter().rfind(|arg| arg.as_str() != "--json");
    let Some(path) = file else {
        eprintln!("missing patch file");
        return 2;
    };
    match fs::read_to_string(path.as_str()) {
        Ok(text) => {
            let result = check_patch_json(path.as_str(), &text);
            println!("{}", result.to_json());
            if result.valid {
                0
            } else {
                1
            }
        }
        Err(err) => {
            eprintln!("failed to read {path}: {err}");
            2
        }
    }
}

fn cmd_patch_apply(args: &[String], force_dry_run: bool) -> i32 {
    let mut dry_run = force_dry_run;
    let mut positional: Vec<String> = Vec::new();
    for arg in args {
        match arg.as_str() {
            "--dry-run" => dry_run = true,
            "--json" => {}
            value if !value.starts_with("--") => positional.push(value.to_string()),
            value => {
                eprintln!("unknown flag `{value}`");
                return 2;
            }
        }
    }
    if positional.len() < 2 {
        eprintln!("usage: ori patch apply [--dry-run] [--json] <patch.json> <source.ori>");
        return 2;
    }
    let patch_path = &positional[0];
    let source_path = &positional[1];
    let Ok(patch_text) = fs::read_to_string(patch_path) else {
        eprintln!("failed to read patch {patch_path}");
        return 2;
    };
    let Ok(source_text) = fs::read_to_string(source_path) else {
        eprintln!("failed to read source {source_path}");
        return 2;
    };
    let source = SourceFile::new(source_path.clone(), source_text);
    let report = apply_patch(&source, &patch_text, dry_run);
    println!("{}", report.to_json());
    if report.applied {
        0
    } else {
        1
    }
}

fn cmd_patch_explain(args: &[String]) -> i32 {
    let Some(path) = args.iter().rfind(|a| !a.starts_with("--")) else {
        eprintln!("usage: ori patch explain [--json] <patch.json>");
        return 2;
    };
    let Ok(text) = fs::read_to_string(path) else {
        eprintln!("failed to read patch {path}");
        return 2;
    };
    let value: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("patch is not valid JSON: {err}");
            return 2;
        }
    };
    let ops = value
        .get("operations")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let intent = value
        .get("intent")
        .and_then(|v| v.as_str())
        .unwrap_or("(no intent declared)");
    let explanation = serde_json::json!({
        "schema": "ori.patch_explain.v1",
        "intent": intent,
        "operation_count": ops,
        "advice": "Run `ori patch dry-run` to preview the resulting source before applying.",
    });
    println!("{explanation}");
    0
}

fn cmd_doctor() -> i32 {
    println!("{}", doctor_report_json());
    0
}

fn cmd_serve(args: &[String]) -> i32 {
    // `ori serve --dry-run --module <path>` — load the module, build
    // the runtime dispatch table, and print the `ori.backend_dispatch.v1`
    // envelope. No HTTP server is started; `--dry-run` is the only
    // supported mode for the M28 milestone.
    let mut dry_run = false;
    let mut module_path: Option<String> = None;
    let mut idx = 0usize;
    while idx < args.len() {
        match args[idx].as_str() {
            "--dry-run" => {
                dry_run = true;
                idx += 1;
            }
            "--module" => {
                let Some(value) = args.get(idx + 1) else {
                    eprintln!("--module requires a value");
                    return 2;
                };
                module_path = Some(value.clone());
                idx += 2;
            }
            "--json" => {
                // Accept and ignore: the dispatch report is always JSON.
                idx += 1;
            }
            other if other.starts_with("--") => {
                eprintln!("unknown flag `{other}`");
                return 2;
            }
            other => {
                module_path = Some(other.to_string());
                idx += 1;
            }
        }
    }
    if !dry_run {
        eprintln!("usage: ori serve --dry-run --module <file.ori>");
        eprintln!("note: only --dry-run is supported in the M28 bootstrap");
        return 2;
    }
    let Some(path) = module_path else {
        eprintln!("usage: ori serve --dry-run --module <file.ori>");
        return 2;
    };
    match Compiler::check_file(&path) {
        Ok(result) => {
            let table = build_dispatch_table(&result.module);
            println!("{}", dispatch_report_json(&table));
            0
        }
        Err(err) => {
            eprintln!("failed to read {path}: {err}");
            2
        }
    }
}

fn cmd_preprocess(args: &[String]) -> i32 {
    use ori_compiler::preproc::{preprocess_file, PreprocessConfig};
    let mut config = PreprocessConfig::new();
    let mut json = false;
    let mut file: Option<String> = None;
    let mut idx = 0usize;
    while idx < args.len() {
        match args[idx].as_str() {
            "--const" => {
                let Some(value) = args.get(idx + 1) else {
                    eprintln!("--const requires a value of the form key=value");
                    return 2;
                };
                let Some((k, v)) = value.split_once('=') else {
                    eprintln!("--const value must be of the form key=value (got `{value}`)");
                    return 2;
                };
                config = config.with_const(k.to_string(), v.to_string());
                idx += 2;
            }
            "--allow-env" => {
                let Some(value) = args.get(idx + 1) else {
                    eprintln!("--allow-env requires a comma-separated value");
                    return 2;
                };
                for name in value.split(',') {
                    let trimmed = name.trim();
                    if !trimmed.is_empty() {
                        config = config.allow_env_var(trimmed.to_string());
                    }
                }
                idx += 2;
            }
            "--json" => {
                json = true;
                idx += 1;
            }
            value if value.starts_with("--") => {
                eprintln!("unknown flag `{value}`");
                return 2;
            }
            value => {
                file = Some(value.to_string());
                idx += 1;
            }
        }
    }
    let Some(file) = file else {
        eprintln!("usage: ori preprocess [--const k=v ...] [--allow-env X,Y] [--json] <file.ori>");
        return 2;
    };
    match preprocess_file(Path::new(&file), &config) {
        Ok((text, diagnostics)) => {
            if json {
                let body = serde_json::json!({
                    "schema": "ori.preprocess.v1",
                    "expanded": true,
                    "diagnostics": diagnostics,
                    "text": text,
                    "path": file,
                });
                println!("{}", to_json(&body));
            } else if diagnostics.is_empty() {
                print!("{text}");
                if !text.ends_with('\n') {
                    println!();
                }
            } else {
                for diag in &diagnostics {
                    eprintln!("{} {}: {}", diag.level.as_str(), diag.id, diag.message);
                }
                print!("{text}");
                if !text.ends_with('\n') {
                    println!();
                }
            }
            if diagnostics.iter().any(|d| d.is_error()) {
                1
            } else {
                0
            }
        }
        Err(err) => {
            if json {
                let body = serde_json::json!({
                    "schema": "ori.preprocess.v1",
                    "expanded": false,
                    "diagnostics": [],
                    "text": "",
                    "path": file,
                });
                println!("{}", to_json(&body));
                eprintln!("preprocess failed: {err}");
            } else {
                eprintln!("failed to read {file}: {err}");
            }
            2
        }
    }
}

fn cmd_db(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("usage: ori db check [--json] <file.ori>");
        return 2;
    }
    match args[0].as_str() {
        "check" => cmd_db_check(&args[1..]),
        other => {
            eprintln!("unknown db subcommand `{other}`");
            2
        }
    }
}

fn cmd_db_check(args: &[String]) -> i32 {
    let json = args.iter().any(|arg| arg == "--json");
    let Some(file) = args.iter().find(|arg| !arg.starts_with("--")) else {
        eprintln!("usage: ori db check [--json] <file.ori>");
        return 2;
    };
    match Compiler::check_file(file.as_str()) {
        Ok(result) => {
            let query_diagnostics = check_module_queries(&result.module);
            let migration = build_migration_report(&result.module);
            let has_errors =
                query_diagnostics.iter().any(|d| d.is_error()) || !migration.cycles.is_empty();
            if json {
                let body = serde_json::json!({
                    "schema": "ori.db_check.v1",
                    "module": result.module.name,
                    "queries": {
                        "diagnostics": query_diagnostics,
                    },
                    "migrations": migration,
                });
                println!("{}", to_json(&body));
            } else {
                println!("module: {}", result.module.name);
                println!("query diagnostics: {}", query_diagnostics.len());
                for diag in &query_diagnostics {
                    println!("  {} {}: {}", diag.level.as_str(), diag.id, diag.message);
                }
                println!("migration order ({}):", migration.ordered.len());
                for id in &migration.ordered {
                    println!("  - {id}");
                }
                if !migration.cycles.is_empty() {
                    println!("migration cycles:");
                    for cycle in &migration.cycles {
                        println!("  - [{}]", cycle.join(", "));
                    }
                }
            }
            if has_errors {
                1
            } else {
                0
            }
        }
        Err(err) => {
            eprintln!("failed to read {file}: {err}");
            2
        }
    }
}

fn cmd_docs(args: &[String]) -> i32 {
    let mut format = "human".to_string();
    let mut budget: usize = 4000;
    let mut json = false;
    let mut path: Option<String> = None;
    let mut idx = 0usize;
    while idx < args.len() {
        match args[idx].as_str() {
            "--format" => {
                let Some(value) = args.get(idx + 1) else {
                    eprintln!("--format requires a value");
                    return 2;
                };
                if value != "human" && value != "agent" {
                    eprintln!("--format must be `human` or `agent`");
                    return 2;
                }
                format = value.clone();
                idx += 2;
            }
            "--budget" => {
                let Some(value) = args.get(idx + 1) else {
                    eprintln!("--budget requires a value");
                    return 2;
                };
                match value.parse::<usize>() {
                    Ok(parsed) => budget = parsed,
                    Err(_) => {
                        eprintln!("--budget must be a non-negative integer");
                        return 2;
                    }
                }
                idx += 2;
            }
            "--json" => {
                json = true;
                idx += 1;
            }
            value if value.starts_with("--") => {
                eprintln!("unknown flag `{value}`");
                return 2;
            }
            value => {
                path = Some(value.to_string());
                idx += 1;
            }
        }
    }
    let target = path.unwrap_or_else(|| ".".to_string());
    let modules = match collect_modules(Path::new(&target)) {
        Ok(modules) => modules,
        Err(err) => {
            eprintln!("docs failed: {err}");
            return 2;
        }
    };
    let (content, budget_used) = match format.as_str() {
        "human" => (generate_human_docs(&modules), None),
        "agent" => (generate_agent_docs(&modules, budget), Some(budget)),
        _ => {
            eprintln!("invalid format `{format}`");
            return 2;
        }
    };
    if json {
        let envelope = match budget_used {
            Some(b) => serde_json::json!({
                "schema": "ori.docs.v1",
                "format": format,
                "budget": b,
                "content": content,
            }),
            None => serde_json::json!({
                "schema": "ori.docs.v1",
                "format": format,
                "budget": serde_json::Value::Null,
                "content": content,
            }),
        };
        println!("{}", to_json(&envelope));
    } else {
        print!("{content}");
        if !content.ends_with('\n') {
            println!();
        }
    }
    0
}

fn cmd_migrate(args: &[String]) -> i32 {
    let mut from_edition: Option<String> = None;
    let mut to_edition: Option<String> = None;
    let mut dry_run = true;
    let mut explicit_apply = false;
    let mut json = false;
    let mut path: Option<String> = None;
    let mut idx = 0usize;
    while idx < args.len() {
        match args[idx].as_str() {
            "--from" => {
                let Some(value) = args.get(idx + 1) else {
                    eprintln!("--from requires a value");
                    return 2;
                };
                from_edition = Some(value.clone());
                idx += 2;
            }
            "--to" => {
                let Some(value) = args.get(idx + 1) else {
                    eprintln!("--to requires a value");
                    return 2;
                };
                to_edition = Some(value.clone());
                idx += 2;
            }
            "--dry-run" => {
                dry_run = true;
                idx += 1;
            }
            "--apply" => {
                explicit_apply = true;
                dry_run = false;
                idx += 1;
            }
            "--json" => {
                json = true;
                idx += 1;
            }
            value if value.starts_with("--") => {
                eprintln!("unknown flag `{value}`");
                return 2;
            }
            value => {
                path = Some(value.to_string());
                idx += 1;
            }
        }
    }
    let Some(from_edition) = from_edition else {
        eprintln!("usage: ori migrate --from <X> --to <Y> [--dry-run] [--json] [path]");
        return 2;
    };
    let Some(to_edition) = to_edition else {
        eprintln!("usage: ori migrate --from <X> --to <Y> [--dry-run] [--json] [path]");
        return 2;
    };
    if explicit_apply && !dry_run {
        let err = serde_json::json!({
            "schema": "ori.migration_report.v1",
            "level": "error",
            "from": from_edition,
            "to": to_edition,
            "message": "non-dry-run migration is not yet implemented; rerun with --dry-run",
        });
        if json {
            println!("{err}");
        } else {
            eprintln!(
                "migrate: non-dry-run migration is not yet implemented; rerun with --dry-run"
            );
        }
        return 1;
    }
    let target = path.unwrap_or_else(|| ".".to_string());
    let modules = match collect_modules(Path::new(&target)) {
        Ok(modules) => modules,
        Err(err) => {
            eprintln!("migrate failed: {err}");
            return 2;
        }
    };
    let report = plan_migration(&modules, &from_edition, &to_edition);
    let has_unsupported = report
        .candidates
        .iter()
        .any(|candidate| candidate.kind == "unsupported_edition");
    if has_unsupported {
        let err = unsupported_edition_error(&from_edition, &to_edition);
        if json {
            println!("{}", err.to_json());
        } else {
            eprintln!("migrate: {}", err.message);
        }
        return 1;
    }
    if json {
        println!("{}", report.to_json());
    } else {
        println!(
            "migration plan {} -> {} ({} candidate(s))",
            report.from,
            report.to,
            report.candidates.len()
        );
        for candidate in &report.candidates {
            println!(
                "  [{}] {} :: `{}` -> `{}`",
                candidate.kind, candidate.target, candidate.from_form, candidate.to_form
            );
        }
        if report.candidates.is_empty() {
            println!("  (no changes required)");
        }
    }
    0
}

/// Walk `.ori` files starting at `root` (file or directory) and parse each
/// into a [`Module`]. Errors are returned as a single string suitable for
/// printing to stderr.
fn collect_modules(root: &Path) -> Result<Vec<Module>, String> {
    let files = collect_ori_files(root, &["src", "tests", "examples"]);
    if files.is_empty() {
        return Err(format!("no .ori files found under `{}`", root.display()));
    }
    let mut modules: Vec<Module> = Vec::with_capacity(files.len());
    for (path, text) in &files {
        let source = SourceFile::new(path.clone(), text.clone());
        let result = Compiler::check_source(source);
        modules.push(result.module);
    }
    modules.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.path.cmp(&b.path)));
    Ok(modules)
}

/// Resolve the manifest path: caller may pass a directory or an explicit file.
fn resolve_manifest_path(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.join("ori.toml")
    } else {
        path.to_path_buf()
    }
}

/// Manifest root directory for a given manifest path.
fn manifest_root(path: &Path) -> PathBuf {
    path.parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Shared arg parser: returns `(json_flag, positional_args)`.
fn split_flags(args: &[String]) -> (bool, Vec<String>) {
    let mut json = false;
    let mut positional: Vec<String> = Vec::new();
    for arg in args {
        if arg == "--json" {
            json = true;
        } else {
            positional.push(arg.clone());
        }
    }
    (json, positional)
}

fn cmd_package(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("usage: ori package check [--json] [path]");
        return 2;
    }
    match args[0].as_str() {
        "check" => cmd_package_check(&args[1..]),
        other => {
            eprintln!("unknown package subcommand `{other}`");
            2
        }
    }
}

fn cmd_package_check(args: &[String]) -> i32 {
    let (json, positional) = split_flags(args);
    let target = positional
        .into_iter()
        .find(|arg| !arg.starts_with("--"))
        .unwrap_or_else(|| ".".to_string());
    let manifest_path = resolve_manifest_path(Path::new(&target));
    let manifest = match Manifest::from_path(&manifest_path) {
        Ok(m) => m,
        Err(err) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "schema": "ori.package_check.v1",
                        "ok": false,
                        "manifest_path": manifest_path.display().to_string(),
                        "error": err.to_string(),
                    })
                );
            } else {
                eprintln!("package check failed: {err}");
            }
            return 1;
        }
    };
    let root = manifest_root(&manifest_path);
    let lockfile = match build_lockfile(&manifest, &root) {
        Ok(lock) => lock,
        Err(err) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "schema": "ori.package_check.v1",
                        "ok": false,
                        "manifest_path": manifest_path.display().to_string(),
                        "error": err.to_string(),
                    })
                );
            } else {
                eprintln!("package check failed: {err}");
            }
            return 1;
        }
    };
    if json {
        let body = serde_json::json!({
            "schema": "ori.package_check.v1",
            "ok": true,
            "manifest_path": manifest_path.display().to_string(),
            "manifest": manifest,
            "lockfile": lockfile,
        });
        println!("{body}");
    } else {
        println!(
            "ok: {} v{}",
            manifest.package.name, manifest.package.version
        );
        println!("packages in lockfile: {}", lockfile.packages.len());
    }
    0
}

fn cmd_audit(args: &[String]) -> i32 {
    let (json, positional) = split_flags(args);
    let target = positional
        .into_iter()
        .find(|arg| !arg.starts_with("--"))
        .unwrap_or_else(|| ".".to_string());
    let manifest_path = resolve_manifest_path(Path::new(&target));
    let manifest = match Manifest::from_path(&manifest_path) {
        Ok(m) => m,
        Err(err) => {
            eprintln!("audit failed: {err}");
            return 2;
        }
    };
    let root = manifest_root(&manifest_path);
    let graph = match resolve(&manifest, &root) {
        Ok(g) => g,
        Err(err) => {
            eprintln!("audit failed: {err}");
            return 2;
        }
    };
    let report = run_audit(&manifest, &graph);
    if json {
        match serde_json::to_string(&report) {
            Ok(s) => println!("{s}"),
            Err(err) => {
                eprintln!("audit serialise failed: {err}");
                return 2;
            }
        }
    } else {
        println!(
            "audit: pass={} warn={} fail={}",
            report.summary.pass, report.summary.warn, report.summary.fail
        );
        for finding in &report.findings {
            println!(
                "  [{:?}] {} ({}): {}",
                finding.severity, finding.id, finding.target, finding.message
            );
        }
    }
    if report.summary.fail > 0 {
        1
    } else {
        0
    }
}

fn cmd_sbom(args: &[String]) -> i32 {
    let mut json = false;
    let mut format = SbomFormat::OriNative;
    let mut positional: Vec<String> = Vec::new();
    let mut idx = 0usize;
    while idx < args.len() {
        match args[idx].as_str() {
            "--json" => {
                json = true;
                idx += 1;
            }
            "--format" => {
                let Some(value) = args.get(idx + 1) else {
                    eprintln!("--format requires a value");
                    return 2;
                };
                let Some(parsed) = SbomFormat::from_cli(value.as_str()) else {
                    eprintln!("unknown sbom format `{value}` (expected ori-native|spdx|cyclonedx)");
                    return 2;
                };
                format = parsed;
                idx += 2;
            }
            other => {
                positional.push(other.to_string());
                idx += 1;
            }
        }
    }
    let target = positional
        .into_iter()
        .find(|arg| !arg.starts_with("--"))
        .unwrap_or_else(|| ".".to_string());
    let manifest_path = resolve_manifest_path(Path::new(&target));
    let manifest = match Manifest::from_path(&manifest_path) {
        Ok(m) => m,
        Err(err) => {
            eprintln!("sbom failed: {err}");
            return 2;
        }
    };
    let root = manifest_root(&manifest_path);
    let graph = match resolve(&manifest, &root) {
        Ok(g) => g,
        Err(err) => {
            eprintln!("sbom failed: {err}");
            return 2;
        }
    };
    let sbom = build_sbom(&graph, format);
    let rendered = if json {
        serde_json::to_string(&sbom)
    } else {
        serde_json::to_string_pretty(&sbom)
    };
    match rendered {
        Ok(s) => {
            println!("{s}");
            0
        }
        Err(err) => {
            eprintln!("sbom serialise failed: {err}");
            2
        }
    }
}

fn cmd_provenance(args: &[String]) -> i32 {
    if args.is_empty() || args[0] != "verify" {
        eprintln!("usage: ori provenance verify [--json] <file.json>");
        return 2;
    }
    let (json, positional) = split_flags(&args[1..]);
    let Some(file) = positional.into_iter().find(|arg| !arg.starts_with("--")) else {
        eprintln!("missing provenance file");
        return 2;
    };
    let text = match fs::read_to_string(file.as_str()) {
        Ok(t) => t,
        Err(err) => {
            eprintln!("failed to read {file}: {err}");
            return 2;
        }
    };
    let result = verify_provenance(&text);
    let rendered = if json {
        serde_json::to_string(&result)
    } else {
        serde_json::to_string_pretty(&result)
    };
    match rendered {
        Ok(s) => {
            println!("{s}");
            if result.verified {
                0
            } else {
                1
            }
        }
        Err(err) => {
            eprintln!("provenance serialise failed: {err}");
            2
        }
    }
}

/// Parse `name@version`. Returns `None` if the separator is missing or either
/// side is blank.
fn parse_name_version(spec: &str) -> Option<(String, String)> {
    let (name, version) = spec.split_once('@')?;
    let name = name.trim();
    let version = version.trim();
    if name.is_empty() || version.is_empty() {
        return None;
    }
    Some((name.to_string(), version.to_string()))
}

fn cmd_publish(args: &[String]) -> i32 {
    let mut registry: Option<String> = None;
    let mut tarball: Option<String> = None;
    let mut manifest_target: Option<String> = None;
    let mut json = false;
    let mut idx = 0usize;
    while idx < args.len() {
        match args[idx].as_str() {
            "--registry" => {
                let Some(value) = args.get(idx + 1) else {
                    eprintln!("--registry requires a value");
                    return 2;
                };
                registry = Some(value.clone());
                idx += 2;
            }
            "--tarball" => {
                let Some(value) = args.get(idx + 1) else {
                    eprintln!("--tarball requires a value");
                    return 2;
                };
                tarball = Some(value.clone());
                idx += 2;
            }
            "--json" => {
                json = true;
                idx += 1;
            }
            value if value.starts_with("--") => {
                eprintln!("unknown flag `{value}`");
                return 2;
            }
            value => {
                manifest_target = Some(value.to_string());
                idx += 1;
            }
        }
    }
    let Some(registry) = registry else {
        eprintln!("usage: ori publish --registry <path> --tarball <file> [--json] [path]");
        return 2;
    };
    let Some(tarball) = tarball else {
        eprintln!("usage: ori publish --registry <path> --tarball <file> [--json] [path]");
        return 2;
    };
    let manifest_path = resolve_manifest_path(Path::new(
        &manifest_target.unwrap_or_else(|| ".".to_string()),
    ));
    let manifest = match Manifest::from_path(&manifest_path) {
        Ok(m) => m,
        Err(err) => {
            eprintln!("publish failed: manifest error: {err}");
            return 2;
        }
    };
    let tarball_bytes = match fs::read(&tarball) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("publish failed: could not read tarball {tarball}: {err}");
            return 2;
        }
    };
    let reg = LocalRegistry::new(&registry);
    if let Err(err) = reg.init() {
        eprintln!("publish failed: could not initialise registry: {err}");
        return 2;
    }
    match reg.publish(&manifest, &tarball_bytes) {
        Ok(receipt) => match render_receipt(&receipt, json) {
            Ok(text) => {
                println!("{text}");
                0
            }
            Err(err) => {
                eprintln!("publish failed: could not render receipt: {err}");
                2
            }
        },
        Err(err) => {
            eprintln!("publish failed: {err}");
            registry_exit_code(&err)
        }
    }
}

fn render_receipt(receipt: &PublishReceipt, json: bool) -> Result<String, serde_json::Error> {
    if json {
        serde_json::to_string(receipt)
    } else {
        serde_json::to_string_pretty(receipt)
    }
}

fn cmd_fetch(args: &[String]) -> i32 {
    let mut registry: Option<String> = None;
    let mut out: Option<String> = None;
    let mut spec: Option<String> = None;
    let mut json = false;
    let mut idx = 0usize;
    while idx < args.len() {
        match args[idx].as_str() {
            "--registry" => {
                let Some(value) = args.get(idx + 1) else {
                    eprintln!("--registry requires a value");
                    return 2;
                };
                registry = Some(value.clone());
                idx += 2;
            }
            "--out" => {
                let Some(value) = args.get(idx + 1) else {
                    eprintln!("--out requires a value");
                    return 2;
                };
                out = Some(value.clone());
                idx += 2;
            }
            "--json" => {
                json = true;
                idx += 1;
            }
            value if value.starts_with("--") => {
                eprintln!("unknown flag `{value}`");
                return 2;
            }
            value => {
                spec = Some(value.to_string());
                idx += 1;
            }
        }
    }
    let Some(registry) = registry else {
        eprintln!("usage: ori fetch --registry <path> <name>@<version> [--out <file>] [--json]");
        return 2;
    };
    let Some(spec) = spec else {
        eprintln!("usage: ori fetch --registry <path> <name>@<version> [--out <file>] [--json]");
        return 2;
    };
    let Some((name, version)) = parse_name_version(&spec) else {
        eprintln!("invalid <name>@<version>: `{spec}`");
        return 2;
    };
    let reg = LocalRegistry::new(&registry);
    match reg.fetch(&name, &version) {
        Ok(bytes) => {
            if let Some(out_path) = out.as_deref() {
                if let Err(err) = fs::write(out_path, &bytes) {
                    eprintln!("fetch failed: could not write {out_path}: {err}");
                    return 2;
                }
            }
            if json {
                let body = serde_json::json!({
                    "schema": "ori.fetch_receipt.v1",
                    "name": name,
                    "version": version,
                    "bytes": bytes.len(),
                    "checksum": ori_pkg::fnv1a_hex(&bytes),
                    "written_to": out,
                });
                println!("{body}");
            } else if let Some(out_path) = out.as_deref() {
                println!(
                    "fetched {name}@{version} ({} bytes) -> {out_path}",
                    bytes.len()
                );
            } else {
                println!("fetched {name}@{version} ({} bytes)", bytes.len());
            }
            0
        }
        Err(err) => {
            eprintln!("fetch failed: {err}");
            registry_exit_code(&err)
        }
    }
}

fn cmd_registry(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("usage: ori registry <list|yank> --registry <path> ...");
        return 2;
    }
    match args[0].as_str() {
        "list" => cmd_registry_list(&args[1..]),
        "yank" => cmd_registry_yank(&args[1..]),
        other => {
            eprintln!("unknown registry subcommand `{other}`");
            2
        }
    }
}

fn cmd_registry_list(args: &[String]) -> i32 {
    let mut registry: Option<String> = None;
    let mut json = false;
    let mut idx = 0usize;
    while idx < args.len() {
        match args[idx].as_str() {
            "--registry" => {
                let Some(value) = args.get(idx + 1) else {
                    eprintln!("--registry requires a value");
                    return 2;
                };
                registry = Some(value.clone());
                idx += 2;
            }
            "--json" => {
                json = true;
                idx += 1;
            }
            value => {
                eprintln!("unknown argument `{value}`");
                return 2;
            }
        }
    }
    let Some(registry) = registry else {
        eprintln!("usage: ori registry list --registry <path> [--json]");
        return 2;
    };
    let reg = LocalRegistry::new(&registry);
    let entries: Vec<PackageEntry> = match reg.list() {
        Ok(v) => v,
        Err(err) => {
            eprintln!("registry list failed: {err}");
            return registry_exit_code(&err);
        }
    };
    if json {
        let body = serde_json::json!({
            "schema": REGISTRY_LIST_SCHEMA,
            "registry": registry,
            "packages": entries,
        });
        println!("{body}");
    } else {
        println!("registry: {registry}");
        println!("packages: {}", entries.len());
        for entry in &entries {
            let yank_mark = if entry.yanked { " (yanked)" } else { "" };
            println!(
                "  {}@{} {} bytes checksum={}{}",
                entry.name, entry.version, entry.bytes, entry.checksum, yank_mark
            );
        }
    }
    0
}

fn cmd_registry_yank(args: &[String]) -> i32 {
    let mut registry: Option<String> = None;
    let mut reason: Option<String> = None;
    let mut spec: Option<String> = None;
    let mut json = false;
    let mut idx = 0usize;
    while idx < args.len() {
        match args[idx].as_str() {
            "--registry" => {
                let Some(value) = args.get(idx + 1) else {
                    eprintln!("--registry requires a value");
                    return 2;
                };
                registry = Some(value.clone());
                idx += 2;
            }
            "--reason" => {
                let Some(value) = args.get(idx + 1) else {
                    eprintln!("--reason requires a value");
                    return 2;
                };
                reason = Some(value.clone());
                idx += 2;
            }
            "--json" => {
                json = true;
                idx += 1;
            }
            value if value.starts_with("--") => {
                eprintln!("unknown flag `{value}`");
                return 2;
            }
            value => {
                spec = Some(value.to_string());
                idx += 1;
            }
        }
    }
    let Some(registry) = registry else {
        eprintln!(
            "usage: ori registry yank --registry <path> <name>@<version> --reason <r> [--json]"
        );
        return 2;
    };
    let Some(spec) = spec else {
        eprintln!(
            "usage: ori registry yank --registry <path> <name>@<version> --reason <r> [--json]"
        );
        return 2;
    };
    let Some(reason) = reason else {
        eprintln!(
            "usage: ori registry yank --registry <path> <name>@<version> --reason <r> [--json]"
        );
        return 2;
    };
    let Some((name, version)) = parse_name_version(&spec) else {
        eprintln!("invalid <name>@<version>: `{spec}`");
        return 2;
    };
    let reg = LocalRegistry::new(&registry);
    match reg.yank(&name, &version, &reason) {
        Ok(()) => {
            if json {
                let body = serde_json::json!({
                    "schema": "ori.yank_receipt.v1",
                    "name": name,
                    "version": version,
                    "reason": reason,
                });
                println!("{body}");
            } else {
                println!("yanked {name}@{version}: {reason}");
            }
            0
        }
        Err(err) => {
            eprintln!("yank failed: {err}");
            registry_exit_code(&err)
        }
    }
}

fn registry_exit_code(err: &RegistryError) -> i32 {
    match err {
        RegistryError::Invalid(_) => 2,
        RegistryError::NotFound => 1,
        RegistryError::AlreadyExists(_, _) => 1,
        RegistryError::Yanked(_) => 1,
        RegistryError::Io(_) => 2,
    }
}
