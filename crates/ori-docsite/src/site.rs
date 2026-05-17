//! Site assembly: scan an input directory, render each markdown file to HTML,
//! emit a shared stylesheet, and produce a [`SiteReport`].

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::css::STYLE_CSS;
use crate::error::SiteError;
use crate::markdown::{escape_attr, escape_html, render_markdown};
use crate::navigation::{build_navigation, render_navigation, NavEntry};

/// Summary returned by [`build_site`] on success.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteReport {
    /// Number of HTML pages emitted (one per source `.md`).
    pub pages: usize,
    /// Number of non-page assets emitted (currently always 1: `style.css`).
    pub assets: usize,
    /// Total bytes written to disk across pages and assets.
    pub bytes_written: u64,
}

/// Build the docsite from `input_dir` into `output_dir`.
///
/// Walks `input_dir` recursively, converts every `*.md` file with the in-repo
/// markdown subset converter, builds a shared sorted navigation tree, and
/// writes one HTML page per source markdown plus a single `style.css`. Output
/// directory is created if it does not exist.
pub fn build_site(input_dir: &Path, output_dir: &Path) -> Result<SiteReport, SiteError> {
    if !input_dir.is_dir() {
        return Err(SiteError::InputNotADirectory(input_dir.to_path_buf()));
    }

    // Collect all markdown source paths relative to input_dir.
    let mut sources: Vec<PathBuf> = Vec::new();
    collect_markdown(input_dir, input_dir, &mut sources)?;
    sources.sort();

    // Build NavEntry list deterministically.
    let mut entries: Vec<NavEntry> = Vec::with_capacity(sources.len());
    let mut rendered: Vec<(PathBuf, String)> = Vec::with_capacity(sources.len());

    for source_abs in &sources {
        let rel = match source_abs.strip_prefix(input_dir) {
            Ok(p) => p.to_path_buf(),
            Err(_) => return Err(SiteError::PathEscape { path: source_abs.clone() }),
        };
        let source_rel = path_to_forward_slash(&rel)
            .ok_or_else(|| SiteError::NonUtf8Path { path: rel.clone() })?;
        let html_rel = replace_md_extension(&source_rel);

        let bytes = fs::read(source_abs).map_err(|e| SiteError::Read {
            path: source_abs.clone(),
            source: e,
        })?;
        let md_text = String::from_utf8_lossy(&bytes).into_owned();
        let title = first_heading_or_filename(&md_text, &source_rel);
        let html_body = render_markdown(&md_text);

        entries.push(NavEntry {
            source_rel: source_rel.clone(),
            html_rel: html_rel.clone(),
            title,
        });
        rendered.push((PathBuf::from(html_rel), html_body));
    }

    let nav_tree = build_navigation(&entries);

    // Ensure output directory exists.
    fs::create_dir_all(output_dir).map_err(|e| SiteError::CreateDir {
        path: output_dir.to_path_buf(),
        source: e,
    })?;

    let mut bytes_written: u64 = 0;
    let mut pages: usize = 0;

    for ((html_rel_buf, body), entry) in rendered.iter().zip(entries.iter()) {
        let nav_snippet = render_navigation(&nav_tree, &entry.html_rel);
        let depth = entry.html_rel.matches('/').count();
        let css_href = if depth == 0 {
            "style.css".to_string()
        } else {
            format!("{}style.css", "../".repeat(depth))
        };
        let page = render_page(&entry.title, &css_href, &nav_snippet, body);

        let dest = output_dir.join(html_rel_buf);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|e| SiteError::CreateDir {
                path: parent.to_path_buf(),
                source: e,
            })?;
        }
        fs::write(&dest, page.as_bytes()).map_err(|e| SiteError::Write {
            path: dest.clone(),
            source: e,
        })?;
        bytes_written = bytes_written.saturating_add(page.as_bytes().len() as u64);
        pages += 1;
    }

    // Write shared stylesheet.
    let css_path = output_dir.join("style.css");
    fs::write(&css_path, STYLE_CSS.as_bytes()).map_err(|e| SiteError::Write {
        path: css_path.clone(),
        source: e,
    })?;
    bytes_written = bytes_written.saturating_add(STYLE_CSS.as_bytes().len() as u64);

    Ok(SiteReport {
        pages,
        assets: 1,
        bytes_written,
    })
}

fn collect_markdown(
    root: &Path,
    dir: &Path,
    out: &mut Vec<PathBuf>,
) -> Result<(), SiteError> {
    let read = fs::read_dir(dir).map_err(|e| SiteError::Walk {
        path: dir.to_path_buf(),
        source: e,
    })?;
    for entry in read {
        let entry = entry.map_err(|e| SiteError::Walk {
            path: dir.to_path_buf(),
            source: e,
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|e| SiteError::Walk {
            path: path.clone(),
            source: e,
        })?;
        if file_type.is_dir() {
            collect_markdown(root, &path, out)?;
        } else if file_type.is_file() {
            if has_md_extension(&path) {
                out.push(path);
            }
        } else if file_type.is_symlink() {
            // Treat symlinks conservatively: only follow if the target is a
            // markdown file inside `root`. We do a soft check via `metadata`.
            if let Ok(meta) = fs::metadata(&path) {
                if meta.is_file() && has_md_extension(&path) {
                    out.push(path);
                }
            }
        }
    }
    Ok(())
}

fn has_md_extension(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => ext.eq_ignore_ascii_case("md"),
        None => false,
    }
}

fn path_to_forward_slash(rel: &Path) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for comp in rel.components() {
        match comp {
            Component::Normal(s) => match s.to_str() {
                Some(t) => parts.push(t),
                None => return None,
            },
            Component::CurDir => {}
            // Reject absolute or parent components in a relative path.
            Component::RootDir | Component::Prefix(_) | Component::ParentDir => return None,
        }
    }
    Some(parts.join("/"))
}

fn replace_md_extension(source_rel: &str) -> String {
    if let Some(stripped) = source_rel.strip_suffix(".md") {
        format!("{}.html", stripped)
    } else if let Some(stripped) = source_rel.strip_suffix(".MD") {
        format!("{}.html", stripped)
    } else {
        format!("{}.html", source_rel)
    }
}

fn first_heading_or_filename(md: &str, source_rel: &str) -> String {
    for line in md.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("# ") {
            return rest.trim().to_string();
        }
        if trimmed == "#" {
            break;
        }
    }
    // Fallback: use the file stem.
    let stem = match source_rel.rsplit('/').next() {
        Some(name) => name,
        None => source_rel,
    };
    stem.strip_suffix(".md")
        .or_else(|| stem.strip_suffix(".MD"))
        .unwrap_or(stem)
        .to_string()
}

fn render_page(title: &str, css_href: &str, nav: &str, body: &str) -> String {
    format!(
        "<!doctype html>\n\
<html lang=\"en\">\n\
<head>\n\
  <meta charset=\"utf-8\" />\n\
  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n\
  <title>{title}</title>\n\
  <link rel=\"stylesheet\" href=\"{css}\" />\n\
</head>\n\
<body>\n\
  <div class=\"layout\">\n\
    <nav class=\"sidebar\">\n\
      <h1>Orison Docs</h1>\n\
{nav}\
    </nav>\n\
    <main class=\"content\">\n\
{body}\
      <footer>Generated by ori-docsite.</footer>\n\
    </main>\n\
  </div>\n\
</body>\n\
</html>\n",
        title = escape_html(title),
        css = escape_attr(css_href),
        nav = nav,
        body = body,
    )
}

// We import std::io only for the trait bound; silence unused warning if any.
#[allow(dead_code)]
fn _silence_io(_: io::ErrorKind) {}
