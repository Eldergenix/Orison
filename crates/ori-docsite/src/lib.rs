//! `ori-docsite` is the bootstrap static documentation site generator for the
//! Orison language kit.
//!
//! Per `MEMORY.md` decision D002 (bootstrap dep policy), this crate depends only
//! on `serde` and `serde_json`. The markdown subset supported by the in-repo
//! converter is documented in [`markdown`]; the converter is hand-written to
//! avoid pulling in `pulldown-cmark` or `comrak`.
//!
//! All output is deterministic: navigation entries are sorted by source file
//! path, table-of-contents order matches input directory traversal sorted
//! lexicographically, and no system clocks or hash maps are used in the
//! rendered HTML.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod css;
pub mod error;
pub mod markdown;
pub mod navigation;
pub mod site;

pub use error::SiteError;
pub use markdown::render_markdown;
pub use navigation::{build_navigation, NavEntry, NavNode};
pub use site::{build_site, SiteReport};
