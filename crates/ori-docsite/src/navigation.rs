//! Navigation tree builder.
//!
//! Given a sorted slice of source markdown paths (relative to the input
//! directory), produces a hierarchical tree grouped by directory and renders
//! it as a left-nav HTML snippet. Ordering is strictly lexicographic over the
//! original path components so the output is byte-deterministic across runs
//! and platforms.

use std::collections::BTreeMap;

use crate::markdown::{escape_attr, escape_html};

/// A single leaf entry in the navigation tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavEntry {
    /// Source relative path (e.g. `tutorial/01-install.md`).
    pub source_rel: String,
    /// Output relative HTML path (e.g. `tutorial/01-install.html`).
    pub html_rel: String,
    /// Human-readable label, derived from the first heading or filename.
    pub title: String,
}

/// A node in the navigation tree: either a directory or a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavNode {
    /// A directory groups child nodes; child order matches sorted file paths.
    Directory {
        /// Directory display name (last path segment).
        name: String,
        /// Children sorted by source path (directories interleave with pages
        /// at their proper position in the lexicographic order).
        children: Vec<NavNode>,
    },
    /// A leaf page entry.
    Page(NavEntry),
}

/// Build the navigation tree from a sorted list of entries.
///
/// The caller is responsible for sorting `entries` by `source_rel` before
/// invocation; [`crate::build_site`] does this. Within each directory level,
/// pages and subdirectories appear in lexicographic order of their first
/// path component, which is consistent with how the sorted list is produced.
pub fn build_navigation(entries: &[NavEntry]) -> Vec<NavNode> {
    // Group by first path component (a directory) vs no slash (a top-level
    // page). Use BTreeMap so directory order is deterministic.
    let mut dirs: BTreeMap<String, Vec<NavEntry>> = BTreeMap::new();
    let mut pages: Vec<NavEntry> = Vec::new();

    for entry in entries {
        match entry.source_rel.find('/') {
            Some(idx) => {
                let head = entry.source_rel[..idx].to_string();
                let tail = entry.source_rel[idx + 1..].to_string();
                let html_tail = match entry.html_rel.find('/') {
                    Some(j) => entry.html_rel[j + 1..].to_string(),
                    None => entry.html_rel.clone(),
                };
                dirs.entry(head).or_default().push(NavEntry {
                    source_rel: tail,
                    html_rel: html_tail,
                    title: entry.title.clone(),
                });
            }
            None => pages.push(entry.clone()),
        }
    }

    // Merge directories + pages into a single ordered list by the first path
    // component string. Pages use their full source name as the sort key.
    let mut combined: Vec<(String, NavNode)> = Vec::new();
    for (name, children) in dirs {
        let key = name.clone();
        let dir_node = NavNode::Directory {
            name: name.clone(),
            children: build_navigation(&children)
                .into_iter()
                .map(|n| n)
                .collect(),
        };
        combined.push((key, dir_node));
    }
    for page in pages {
        let key = page.source_rel.clone();
        combined.push((key, NavNode::Page(page)));
    }
    combined.sort_by(|a, b| a.0.cmp(&b.0));
    combined.into_iter().map(|(_, n)| n).collect()
}

/// Render the navigation tree to an HTML `<ul>` snippet rooted at the site.
///
/// `current_html_rel` is the relative HTML path of the page being rendered;
/// it is used to mark the current entry with `class="current"`. Links use
/// relative URLs computed from `current_html_rel`'s depth.
pub fn render_navigation(nodes: &[NavNode], current_html_rel: &str) -> String {
    let depth = current_html_rel.matches('/').count();
    let prefix = if depth == 0 {
        String::new()
    } else {
        "../".repeat(depth)
    };
    let mut out = String::new();
    render_nav_inner(nodes, current_html_rel, &prefix, 0, &mut out);
    out
}

fn render_nav_inner(
    nodes: &[NavNode],
    current_html_rel: &str,
    prefix: &str,
    indent: usize,
    out: &mut String,
) {
    let pad = "  ".repeat(indent);
    out.push_str(&pad);
    out.push_str("<ul>\n");
    for node in nodes {
        match node {
            NavNode::Directory { name, children } => {
                out.push_str(&pad);
                out.push_str(&format!(
                    "  <li class=\"nav-dir\"><span class=\"nav-dir-name\">{}</span>\n",
                    escape_html(name)
                ));
                render_nav_inner(children, current_html_rel, prefix, indent + 2, out);
                out.push_str(&pad);
                out.push_str("  </li>\n");
            }
            NavNode::Page(entry) => {
                let is_current = entry.html_rel == current_html_rel;
                let href = format!("{}{}", prefix, entry.html_rel);
                let cls = if is_current {
                    " class=\"current\""
                } else {
                    ""
                };
                out.push_str(&pad);
                out.push_str(&format!(
                    "  <li><a{cls} href=\"{href}\">{label}</a></li>\n",
                    cls = cls,
                    href = escape_attr(&href),
                    label = escape_html(&entry.title)
                ));
            }
        }
    }
    out.push_str(&pad);
    out.push_str("</ul>\n");
}
