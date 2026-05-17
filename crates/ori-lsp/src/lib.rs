//! Minimal Language Server Protocol implementation for the Orison toolchain.
//!
//! This crate intentionally avoids any third-party LSP dependencies. Per
//! `MEMORY.md` decision D002 the bootstrap implementation may only depend on
//! `serde` and `serde_json`. JSON-RPC 2.0 framing, message parsing, and the
//! subset of LSP messages required for `ori lsp --stdio` are implemented by
//! hand against the standard library.
//!
//! The public API exposes a [`Server`] that can be driven with arbitrary
//! [`std::io::Read`] / [`std::io::Write`] pairs. The `ori-cli` crate uses
//! stdin/stdout, while the integration tests use byte pipes.

#![deny(missing_debug_implementations)]

pub mod codec;
pub mod diagnostics;
pub mod protocol;
pub mod server;
pub mod state;

pub use server::Server;
pub use state::{DocumentState, WorkspaceState};
