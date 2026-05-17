//! Orison bootstrap compiler library.
//!
//! This crate hosts every front-end and analysis pass driven by the `ori`
//! CLI and `ori-lsp` server. The bootstrap deliberately depends only on
//! `serde` and `serde_json` (MEMORY.md decision D002), so every module is
//! implemented by hand against the standard library. Keeping the public
//! surface here pinned to a small re-export set (see below) lets downstream
//! crates avoid leaking implementation details into their own APIs.
//!
//! ## Stability invariant
//!
//! Anything the compiler knows must be available to tools and agents through
//! stable structured output. The structured-JSON envelopes produced by
//! [`compiler::Compiler`], [`capsule`], [`docs`], and similar modules are
//! part of the public contract and must not regress without a matching
//! schema bump.

pub mod ast;
pub mod async_runtime;
pub mod backend_dispatch;
pub mod bench;
pub mod body;
pub mod borrow;
pub mod borrow_regions;
pub mod bundler;
pub mod capability_runtime;
pub mod capsule;
pub mod codegen_text;
pub mod compiler;
pub mod const_fold;
pub mod coverage;
pub mod cst;
pub mod design_tokens;
pub mod desktop;
pub mod diagnostic;
pub mod docs;
pub mod effect_check;
pub mod effect_propagate;
pub mod effects;
pub mod exhaustive;
pub mod expr;
pub mod expr_ops;
pub mod formatter;
pub mod generics;
pub mod graphql_import;
pub mod hir;
pub mod incremental;
pub mod interp;
pub mod interp_exec;
pub mod json;
pub mod lexer;
pub mod migrate;
pub mod migration_graph;
pub mod mir;
pub mod mobile;
pub mod mobile_permissions;
pub mod mobile_ui_ir;
pub mod node_id;
pub mod numeric_lit;
pub mod openapi;
pub mod parser;
pub mod patch;
pub mod patch_apply;
pub mod preproc;
pub mod query;
pub mod resolver;
pub mod rpc_import;
pub mod source;
pub mod sql_check;
pub mod string_lits;
pub mod symbols;
pub mod type_check;
pub mod type_infer;
pub mod types;
pub mod ui_check;
pub mod ui_render;
pub mod wasm_component;
pub mod wasm_encoder;

pub use compiler::{CompileMode, CompileResult, Compiler};
pub use diagnostic::{Diagnostic, DiagnosticLevel, Fix};
pub use node_id::NodeId;
pub use source::{Position, SourceFile, Span};
