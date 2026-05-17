//! Tests for the deterministic navigation tree builder.

#![allow(clippy::assertions_on_constants)]

use ori_docsite::{build_navigation, NavEntry, NavNode};

fn entry(path: &str, title: &str) -> NavEntry {
    NavEntry {
        source_rel: path.to_string(),
        html_rel: path.replace(".md", ".html"),
        title: title.to_string(),
    }
}

#[test]
fn top_level_pages_sorted_lexicographically() {
    let mut entries = vec![
        entry("zeta.md", "Zeta"),
        entry("alpha.md", "Alpha"),
        entry("mike.md", "Mike"),
    ];
    entries.sort_by(|a, b| a.source_rel.cmp(&b.source_rel));
    let tree = build_navigation(&entries);
    let names: Vec<&str> = tree
        .iter()
        .map(|n| match n {
            NavNode::Page(p) => p.title.as_str(),
            NavNode::Directory { name, .. } => name.as_str(),
        })
        .collect();
    assert_eq!(names, vec!["Alpha", "Mike", "Zeta"], "tree: {tree:?}");
}

#[test]
fn directories_group_their_children() {
    let mut entries = vec![
        entry("tutorial/02-hello.md", "Hello"),
        entry("tutorial/01-install.md", "Install"),
        entry("language/SPEC.md", "Spec"),
        entry("README.md", "Readme"),
    ];
    entries.sort_by(|a, b| a.source_rel.cmp(&b.source_rel));
    let tree = build_navigation(&entries);
    // Sorted keys are: "README.md", "language", "tutorial"
    assert_eq!(tree.len(), 3, "tree: {tree:?}");
    match &tree[0] {
        NavNode::Page(p) => assert_eq!(p.title, "Readme"),
        other => panic_assert("expected Readme page first", other),
    }
    match &tree[1] {
        NavNode::Directory { name, children } => {
            assert_eq!(name, "language");
            assert_eq!(children.len(), 1);
        }
        other => panic_assert("expected language dir", other),
    }
    match &tree[2] {
        NavNode::Directory { name, children } => {
            assert_eq!(name, "tutorial");
            assert_eq!(children.len(), 2);
            // Children must be sorted: 01-install before 02-hello.
            match &children[0] {
                NavNode::Page(p) => assert_eq!(p.title, "Install"),
                other => panic_assert("expected install first", other),
            }
        }
        other => panic_assert("expected tutorial dir", other),
    }
}

fn panic_assert(msg: &str, node: &NavNode) {
    assert!(false, "{msg}: got {node:?}");
}

#[test]
fn nested_directories_recurse() {
    let mut entries = vec![
        entry("a/b/c/leaf.md", "Leaf"),
        entry("a/b/other.md", "Other"),
    ];
    entries.sort_by(|a, b| a.source_rel.cmp(&b.source_rel));
    let tree = build_navigation(&entries);
    assert_eq!(tree.len(), 1);
    match &tree[0] {
        NavNode::Directory { name, children } => {
            assert_eq!(name, "a");
            assert_eq!(children.len(), 1);
            match &children[0] {
                NavNode::Directory { name, children } => {
                    assert_eq!(name, "b");
                    // b/c (dir) sorts before b/other.md alphabetically
                    // since "c" < "other".
                    assert_eq!(children.len(), 2);
                }
                other => panic_assert("expected b dir", other),
            }
        }
        other => panic_assert("expected a dir", other),
    }
}
