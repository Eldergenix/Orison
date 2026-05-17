//! Bundled stylesheet for the docsite.
//!
//! The CSS is intentionally vanilla (no preprocessors, no JS) so the output
//! site can be served from any static host without a build step. The exported
//! [`STYLE_CSS`] constant is byte-stable across builds.

/// The single bundled stylesheet emitted at `style.css` in the output dir.
pub const STYLE_CSS: &str = r#"/* ori-docsite default stylesheet (no JS, no preprocessor) */
:root {
  --bg: #ffffff;
  --fg: #1a1a1a;
  --muted: #6b7280;
  --link: #2563eb;
  --link-hover: #1d4ed8;
  --code-bg: #f4f4f5;
  --code-fg: #18181b;
  --border: #e5e7eb;
  --nav-bg: #fafafa;
  --nav-current: #eef2ff;
  --table-stripe: #fafafa;
  --max-content: 760px;
  --nav-width: 280px;
}

@media (prefers-color-scheme: dark) {
  :root {
    --bg: #0f1115;
    --fg: #e5e7eb;
    --muted: #9ca3af;
    --link: #60a5fa;
    --link-hover: #93c5fd;
    --code-bg: #1f2329;
    --code-fg: #e5e7eb;
    --border: #2a2f37;
    --nav-bg: #14171c;
    --nav-current: #1e293b;
    --table-stripe: #14171c;
  }
}

* { box-sizing: border-box; }

html, body {
  margin: 0;
  padding: 0;
  background: var(--bg);
  color: var(--fg);
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto,
               "Helvetica Neue", Arial, sans-serif;
  line-height: 1.6;
  font-size: 16px;
}

.layout {
  display: flex;
  min-height: 100vh;
}

.sidebar {
  width: var(--nav-width);
  flex: 0 0 var(--nav-width);
  background: var(--nav-bg);
  border-right: 1px solid var(--border);
  padding: 1.5rem 1rem;
  overflow-y: auto;
}

.sidebar h1 {
  font-size: 1.1rem;
  margin: 0 0 1rem 0;
}

.sidebar ul {
  list-style: none;
  padding-left: 0.75rem;
  margin: 0;
}

.sidebar > ul { padding-left: 0; }

.sidebar li { margin: 0.15rem 0; }

.sidebar a {
  color: var(--fg);
  text-decoration: none;
  display: block;
  padding: 0.2rem 0.4rem;
  border-radius: 4px;
  font-size: 0.92rem;
}

.sidebar a:hover { background: var(--code-bg); }
.sidebar a.current { background: var(--nav-current); color: var(--link); }

.nav-dir-name {
  display: block;
  font-weight: 600;
  font-size: 0.85rem;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  color: var(--muted);
  margin: 0.6rem 0 0.2rem;
}

.content {
  flex: 1;
  padding: 2rem 2.5rem;
  max-width: calc(var(--max-content) + 5rem);
}

h1, h2, h3, h4, h5, h6 {
  line-height: 1.25;
  margin: 1.6em 0 0.6em;
}

h1 { font-size: 2rem; }
h2 { font-size: 1.5rem; border-bottom: 1px solid var(--border); padding-bottom: 0.2em; }
h3 { font-size: 1.2rem; }
h4 { font-size: 1.05rem; }
h5, h6 { font-size: 1rem; color: var(--muted); }

p { margin: 0.8em 0; }

a { color: var(--link); text-decoration: none; }
a:hover { color: var(--link-hover); text-decoration: underline; }

code {
  background: var(--code-bg);
  color: var(--code-fg);
  border-radius: 3px;
  padding: 0.12em 0.32em;
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 0.9em;
}

pre {
  background: var(--code-bg);
  color: var(--code-fg);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 0.85rem 1rem;
  overflow-x: auto;
  font-size: 0.88rem;
  line-height: 1.5;
}

pre code {
  background: transparent;
  padding: 0;
  font-size: inherit;
}

ul, ol { padding-left: 1.4em; }
li { margin: 0.2em 0; }

table {
  border-collapse: collapse;
  margin: 1em 0;
  width: 100%;
  font-size: 0.92rem;
}

th, td {
  border: 1px solid var(--border);
  padding: 0.45em 0.7em;
  text-align: left;
  vertical-align: top;
}

th { background: var(--code-bg); font-weight: 600; }
tbody tr:nth-child(even) { background: var(--table-stripe); }

hr {
  border: none;
  border-top: 1px solid var(--border);
  margin: 2em 0;
}

footer {
  margin-top: 3rem;
  padding-top: 1rem;
  border-top: 1px solid var(--border);
  color: var(--muted);
  font-size: 0.85rem;
}

@media (max-width: 720px) {
  .layout { flex-direction: column; }
  .sidebar {
    width: 100%;
    flex: 0 0 auto;
    border-right: none;
    border-bottom: 1px solid var(--border);
    max-height: 40vh;
  }
  .content { padding: 1.25rem; }
}
"#;
