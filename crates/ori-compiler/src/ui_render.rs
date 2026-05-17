//! Render pipeline that turns a [`ViewDecl`] into a deterministic
//! [`RenderTree`] plus the diff operations needed to drive a host UI.
//!
//! This is the runtime counterpart to [`crate::ui_check`] (static typing +
//! accessibility heuristics) and [`crate::mobile_ui_ir`] (native binding
//! IR). Where `ui_check` describes a `view` symbol *at compile time*,
//! `ui_render` materialises a concrete tree from prop values supplied at
//! call time and produces a byte-stable JSON envelope that downstream
//! adapters (web, mobile, snapshot tests, agents) can consume.
//!
//! ## Determinism contract
//!
//! Every public output is deterministic:
//!
//! * [`RenderTree`] uses [`BTreeMap`] for prop maps; the same inputs always
//!   produce the same serialised bytes.
//! * [`diff_trees`] sorts emitted operations by `(path, op-kind)` so two
//!   identical input pairs always yield byte-identical `Vec<RenderOp>`.
//! * The JSON envelope produced by [`render_report_json`] is therefore
//!   safe to diff in CI without spurious churn.
//!
//! ## Diagnostics
//!
//! Render-time errors are reported as a runtime [`RenderError`] tagged
//! with the `RND00xx` id family:
//!
//! | Id      | Variant                          | Meaning                                                       |
//! |---------|----------------------------------|---------------------------------------------------------------|
//! | RND0001 | [`RenderError::UnknownViewKind`] | Tree references a kind not declared in the active registry.   |
//! | RND0002 | [`RenderError::PropTypeMismatch`]| A prop value's `PropValue` variant disagrees with the slot.   |
//! | RND0003 | [`RenderError::MissingRequiredProp`] | A required prop slot was not supplied.                    |
//! | RND0004 | [`RenderError::DuplicateKey`]    | Two children of the same parent share a non-empty `key`.      |
//! | RND0005 | [`RenderError::ExcessiveDepth`]  | The rendered tree's depth exceeds the safety bound.           |
//!
//! The safety bound is intentionally low (`MAX_RENDER_DEPTH = 64`) so a
//! runaway recursion in a malformed view template is surfaced as a clean
//! diagnostic rather than allowed to exhaust the host stack.

use crate::json::to_json;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// Stable schema id for the [`RenderTree`] envelope.
pub const UI_RENDER_SCHEMA: &str = "ori.ui_render.v1";

/// Maximum permitted depth of a rendered [`ViewNode`] tree. Anything past
/// this bound is rejected with [`RenderError::ExcessiveDepth`] so a
/// malformed template can never exhaust the host stack.
pub const MAX_RENDER_DEPTH: usize = 64;

/// A runtime prop value attached to a [`ViewNode`].
///
/// The variant set is intentionally tight: the bootstrap render pipeline
/// understands strings, integers, booleans, and JSON-style nulls. Richer
/// payloads (records, lists, callbacks) land alongside the view-tree IR
/// in a later milestone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum PropValue {
    /// UTF-8 string.
    Str(String),
    /// 64-bit signed integer.
    Int(i64),
    /// Boolean.
    Bool(bool),
    /// Explicit null. Distinct from "absent": absent props are not present
    /// in the [`ViewNode::props`] map at all.
    Null,
}

impl PropValue {
    /// Canonical, stable name of the variant. Used for type mismatch
    /// diagnostics and the JSON envelope.
    pub fn kind_str(&self) -> &'static str {
        match self {
            PropValue::Str(_) => "str",
            PropValue::Int(_) => "int",
            PropValue::Bool(_) => "bool",
            PropValue::Null => "null",
        }
    }
}

/// Declared expectations for one prop slot in a [`ViewDecl`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PropSlot {
    /// Slot name (matches the source-level prop binding).
    pub name: String,
    /// Required [`PropValue`] kind for the slot, encoded as the same
    /// canonical string [`PropValue::kind_str`] uses.
    pub kind: String,
    /// `true` when omitting the prop must produce
    /// [`RenderError::MissingRequiredProp`].
    pub required: bool,
}

impl PropSlot {
    /// Convenience constructor for a required string slot.
    pub fn required_str(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: "str".to_string(),
            required: true,
        }
    }

    /// Convenience constructor for a required integer slot.
    pub fn required_int(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: "int".to_string(),
            required: true,
        }
    }

    /// Convenience constructor for a required boolean slot.
    pub fn required_bool(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: "bool".to_string(),
            required: true,
        }
    }

    /// Convenience constructor for an optional slot of the given kind.
    pub fn optional(name: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: kind.into(),
            required: false,
        }
    }
}

/// Template body element used inside a [`ViewDecl`].
///
/// Bodies are described as a small tree of templates so the bootstrap
/// can lower a declared `view` to a [`RenderTree`] without yet having a
/// general expression evaluator. A future milestone replaces this with
/// the view-tree IR produced by the compiler proper.
#[derive(Debug, Clone)]
pub struct ViewTemplate {
    /// Kind tag emitted into the resulting [`ViewNode::kind`].
    pub kind: String,
    /// Optional reconciliation key, kept verbatim on the rendered node.
    pub key: Option<String>,
    /// Statically-known props attached to this template element.
    pub props: BTreeMap<String, PropValue>,
    /// Names of prop slots from the enclosing [`ViewDecl`] that are
    /// forwarded onto this element verbatim.
    pub prop_bindings: Vec<String>,
    /// Child template elements, evaluated in declaration order.
    pub children: Vec<ViewTemplate>,
}

impl ViewTemplate {
    /// Construct a leaf template element.
    pub fn leaf(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            key: None,
            props: BTreeMap::new(),
            prop_bindings: Vec::new(),
            children: Vec::new(),
        }
    }

    /// Attach a reconciliation key.
    pub fn with_key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Attach a literal prop.
    pub fn with_prop(mut self, name: impl Into<String>, value: PropValue) -> Self {
        self.props.insert(name.into(), value);
        self
    }

    /// Bind a slot from the enclosing [`ViewDecl`].
    pub fn with_prop_binding(mut self, slot_name: impl Into<String>) -> Self {
        self.prop_bindings.push(slot_name.into());
        self
    }

    /// Append a child template.
    pub fn with_child(mut self, child: ViewTemplate) -> Self {
        self.children.push(child);
        self
    }
}

/// Declared `view` ready for [`render_view`].
///
/// A `ViewDecl` is the runtime sibling of a `view` source declaration.
/// It owns the slot contract (which props are required and what their
/// kinds are) and the body template that will be lowered into a
/// [`ViewNode`] tree.
#[derive(Debug, Clone)]
pub struct ViewDecl {
    /// User-facing view name (matches the source identifier).
    pub name: String,
    /// Symbol id of the originating Orison view, when known.
    pub symbol: Option<String>,
    /// Declared prop slots in declaration order.
    pub props: Vec<PropSlot>,
    /// Body template tree.
    pub body: ViewTemplate,
    /// Other view kinds this decl references (used to enforce
    /// [`RenderError::UnknownViewKind`]). The view's own `name` is
    /// always considered known.
    pub allowed_kinds: BTreeSet<String>,
}

impl ViewDecl {
    /// Build a `ViewDecl` with an empty body template that re-emits the
    /// view's own name as the root kind. Useful for CLI dry-run rendering
    /// where the source-level body is not yet lowered.
    pub fn placeholder(name: impl Into<String>, props: Vec<PropSlot>) -> Self {
        let name = name.into();
        let mut allowed: BTreeSet<String> = BTreeSet::new();
        allowed.insert(name.clone());
        Self {
            name: name.clone(),
            symbol: None,
            props,
            body: ViewTemplate::leaf(name),
            allowed_kinds: allowed,
        }
    }

    /// Replace the body template.
    pub fn with_body(mut self, body: ViewTemplate) -> Self {
        self.body = body;
        self.refresh_allowed_kinds();
        self
    }

    /// Attach an originating symbol id.
    pub fn with_symbol(mut self, symbol: impl Into<String>) -> Self {
        self.symbol = Some(symbol.into());
        self
    }

    /// Allow a non-self kind (e.g. a child component referenced by the
    /// body) to appear in the rendered tree without triggering
    /// [`RenderError::UnknownViewKind`].
    pub fn allow_kind(mut self, kind: impl Into<String>) -> Self {
        self.allowed_kinds.insert(kind.into());
        self
    }

    /// Recompute [`Self::allowed_kinds`] from the current body template.
    /// Always includes the view's own name.
    fn refresh_allowed_kinds(&mut self) {
        let mut allowed: BTreeSet<String> = BTreeSet::new();
        allowed.insert(self.name.clone());
        collect_template_kinds(&self.body, &mut allowed);
        // Preserve any kinds the caller registered through
        // [`Self::allow_kind`] *before* `with_body` was invoked.
        for prior in self.allowed_kinds.iter() {
            allowed.insert(prior.clone());
        }
        self.allowed_kinds = allowed;
    }
}

fn collect_template_kinds(template: &ViewTemplate, out: &mut BTreeSet<String>) {
    out.insert(template.kind.clone());
    for child in &template.children {
        collect_template_kinds(child, out);
    }
}

/// One node of a rendered view tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ViewNode {
    /// Kind tag, identical to the originating [`ViewTemplate::kind`].
    pub kind: String,
    /// Optional reconciliation key inherited from the template.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Stable, ascending-key prop map.
    pub props: BTreeMap<String, PropValue>,
    /// Children in source order.
    pub children: Vec<ViewNode>,
}

impl ViewNode {
    /// Construct a leaf node with no props or children.
    pub fn leaf(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            key: None,
            props: BTreeMap::new(),
            children: Vec::new(),
        }
    }
}

/// Aggregate counters describing a rendered [`RenderTree`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RenderStats {
    /// Total node count including the root.
    pub node_count: usize,
    /// Depth of the deepest leaf (root counted as depth 1).
    pub depth: usize,
    /// Number of nodes whose `key` field is `Some`.
    pub keyed_nodes: usize,
    /// Number of nodes whose `key` field is `None`.
    pub unkeyed_nodes: usize,
}

/// Top-level envelope returned by [`render_view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RenderTree {
    /// Stable schema id (`ori.ui_render.v1`).
    pub schema: &'static str,
    /// Rendered root node.
    pub root: ViewNode,
    /// Aggregate counters for the tree.
    pub stats: RenderStats,
}

/// Runtime render diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "id", rename_all = "snake_case")]
pub enum RenderError {
    /// `RND0001`: tree references a kind the registry does not know.
    #[serde(rename = "RND0001")]
    UnknownViewKind {
        /// Path in the tree (root path is empty).
        path: Vec<usize>,
        /// Unknown kind tag.
        kind: String,
    },
    /// `RND0002`: a prop value's variant disagrees with the slot kind.
    #[serde(rename = "RND0002")]
    PropTypeMismatch {
        /// Path in the tree.
        path: Vec<usize>,
        /// Prop name.
        prop: String,
        /// Expected slot kind (canonical [`PropValue::kind_str`] string).
        expected: String,
        /// Provided variant's canonical string.
        found: String,
    },
    /// `RND0003`: a required prop slot was not supplied.
    #[serde(rename = "RND0003")]
    MissingRequiredProp {
        /// View name carrying the slot.
        view: String,
        /// Slot name.
        prop: String,
    },
    /// `RND0004`: two children of the same parent share a non-empty key.
    #[serde(rename = "RND0004")]
    DuplicateKey {
        /// Path of the parent in the tree.
        path: Vec<usize>,
        /// Shared key.
        key: String,
    },
    /// `RND0005`: tree depth exceeds [`MAX_RENDER_DEPTH`].
    #[serde(rename = "RND0005")]
    ExcessiveDepth {
        /// Path of the offending node.
        path: Vec<usize>,
        /// Observed depth.
        depth: usize,
        /// Permitted maximum.
        max: usize,
    },
}

impl RenderError {
    /// Stable diagnostic id (`RND0001`..`RND0005`).
    pub fn id(&self) -> &'static str {
        match self {
            RenderError::UnknownViewKind { .. } => "RND0001",
            RenderError::PropTypeMismatch { .. } => "RND0002",
            RenderError::MissingRequiredProp { .. } => "RND0003",
            RenderError::DuplicateKey { .. } => "RND0004",
            RenderError::ExcessiveDepth { .. } => "RND0005",
        }
    }
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenderError::UnknownViewKind { path, kind } => {
                write!(f, "{}: unknown view kind `{kind}` at {path:?}", self.id())
            }
            RenderError::PropTypeMismatch {
                path,
                prop,
                expected,
                found,
            } => write!(
                f,
                "{}: prop `{prop}` expected `{expected}` got `{found}` at {path:?}",
                self.id()
            ),
            RenderError::MissingRequiredProp { view, prop } => write!(
                f,
                "{}: view `{view}` is missing required prop `{prop}`",
                self.id()
            ),
            RenderError::DuplicateKey { path, key } => {
                write!(f, "{}: duplicate key `{key}` at {path:?}", self.id())
            }
            RenderError::ExcessiveDepth { path, depth, max } => write!(
                f,
                "{}: render depth {depth} exceeds maximum {max} at {path:?}",
                self.id()
            ),
        }
    }
}

impl std::error::Error for RenderError {}

/// Render `view` with the given `props` into a deterministic [`RenderTree`].
///
/// Slot validation runs first (required-ness + type checking); then the
/// body template is lowered into a [`ViewNode`] tree with sibling key
/// uniqueness and a hard depth bound enforced as it is constructed.
pub fn render_view(
    view: &ViewDecl,
    props: &BTreeMap<String, PropValue>,
) -> Result<RenderTree, RenderError> {
    validate_props(view, props)?;
    let mut path: Vec<usize> = Vec::new();
    let root = build_node(view, &view.body, props, &mut path, 1)?;
    let stats = compute_stats(&root);
    Ok(RenderTree {
        schema: UI_RENDER_SCHEMA,
        root,
        stats,
    })
}

fn validate_props(view: &ViewDecl, props: &BTreeMap<String, PropValue>) -> Result<(), RenderError> {
    // Slot iteration order is the declared one, but missing-required
    // detection itself is deterministic because slots are stored in a
    // `Vec` populated by the caller in source order.
    for slot in &view.props {
        match props.get(&slot.name) {
            None => {
                if slot.required {
                    return Err(RenderError::MissingRequiredProp {
                        view: view.name.clone(),
                        prop: slot.name.clone(),
                    });
                }
            }
            Some(value) => {
                if value.kind_str() != slot.kind {
                    // Slot type mismatch surfaces at the root path; the
                    // tree itself has not yet been constructed.
                    return Err(RenderError::PropTypeMismatch {
                        path: Vec::new(),
                        prop: slot.name.clone(),
                        expected: slot.kind.clone(),
                        found: value.kind_str().to_string(),
                    });
                }
            }
        }
    }
    Ok(())
}

fn build_node(
    view: &ViewDecl,
    template: &ViewTemplate,
    props: &BTreeMap<String, PropValue>,
    path: &mut Vec<usize>,
    depth: usize,
) -> Result<ViewNode, RenderError> {
    if depth > MAX_RENDER_DEPTH {
        return Err(RenderError::ExcessiveDepth {
            path: path.clone(),
            depth,
            max: MAX_RENDER_DEPTH,
        });
    }
    if !view.allowed_kinds.contains(&template.kind) {
        return Err(RenderError::UnknownViewKind {
            path: path.clone(),
            kind: template.kind.clone(),
        });
    }

    // Materialise props: literal first, then forwarded slot bindings so
    // bindings can intentionally override literal defaults.
    let mut materialised: BTreeMap<String, PropValue> = template.props.clone();
    for binding in &template.prop_bindings {
        if let Some(value) = props.get(binding) {
            materialised.insert(binding.clone(), value.clone());
        }
    }

    // Build children, enforcing sibling-key uniqueness.
    let mut children: Vec<ViewNode> = Vec::with_capacity(template.children.len());
    let mut seen_keys: BTreeSet<String> = BTreeSet::new();
    for (idx, child_template) in template.children.iter().enumerate() {
        if let Some(key) = &child_template.key {
            if !seen_keys.insert(key.clone()) {
                return Err(RenderError::DuplicateKey {
                    path: path.clone(),
                    key: key.clone(),
                });
            }
        }
        path.push(idx);
        let child = build_node(view, child_template, props, path, depth + 1)?;
        path.pop();
        children.push(child);
    }

    Ok(ViewNode {
        kind: template.kind.clone(),
        key: template.key.clone(),
        props: materialised,
        children,
    })
}

fn compute_stats(root: &ViewNode) -> RenderStats {
    let mut stats = RenderStats {
        node_count: 0,
        depth: 0,
        keyed_nodes: 0,
        unkeyed_nodes: 0,
    };
    walk_stats(root, 1, &mut stats);
    stats
}

fn walk_stats(node: &ViewNode, depth: usize, stats: &mut RenderStats) {
    stats.node_count += 1;
    if depth > stats.depth {
        stats.depth = depth;
    }
    if node.key.is_some() {
        stats.keyed_nodes += 1;
    } else {
        stats.unkeyed_nodes += 1;
    }
    for child in &node.children {
        walk_stats(child, depth + 1, stats);
    }
}

/// One incremental update emitted by [`diff_trees`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum RenderOp {
    /// Insert `node` at `path`.
    Insert {
        /// Insertion path.
        path: Vec<usize>,
        /// New node.
        node: ViewNode,
    },
    /// Remove the node at `path`.
    Remove {
        /// Path of the node to remove.
        path: Vec<usize>,
    },
    /// Update a single prop at `path`.
    UpdateProp {
        /// Path of the node whose prop changes.
        path: Vec<usize>,
        /// Prop name.
        key: String,
        /// New value.
        value: PropValue,
    },
    /// Reorder a child of the node at `path` from `from` to `to`.
    Reorder {
        /// Path of the parent.
        path: Vec<usize>,
        /// Original child index.
        from: usize,
        /// New child index.
        to: usize,
    },
}

impl RenderOp {
    fn path(&self) -> &Vec<usize> {
        match self {
            RenderOp::Insert { path, .. } => path,
            RenderOp::Remove { path } => path,
            RenderOp::UpdateProp { path, .. } => path,
            RenderOp::Reorder { path, .. } => path,
        }
    }

    /// Stable ordinal for sorting. Order matters when two ops share a
    /// path: removes precede inserts so reconciliation never observes a
    /// transient duplicate sibling.
    fn order_tag(&self) -> u8 {
        match self {
            RenderOp::Remove { .. } => 0,
            RenderOp::UpdateProp { .. } => 1,
            RenderOp::Reorder { .. } => 2,
            RenderOp::Insert { .. } => 3,
        }
    }

    fn secondary_key(&self) -> (String, usize, usize) {
        // Disambiguator for ops sharing path + tag (e.g. two UpdateProp
        // at the same node). Pulling fields out keeps the comparison
        // total without bringing in float ordering.
        match self {
            RenderOp::UpdateProp { key, .. } => (key.clone(), 0, 0),
            RenderOp::Reorder { from, to, .. } => (String::new(), *from, *to),
            RenderOp::Insert { node, .. } => (node.kind.clone(), 0, 0),
            RenderOp::Remove { .. } => (String::new(), 0, 0),
        }
    }
}

/// Produce a deterministic op vector that mutates `old` into `new`.
///
/// Walks both trees in parallel: same kind at the same path → recurse;
/// otherwise emit Remove + Insert. Sibling key matches in different
/// positions are emitted as [`RenderOp::Reorder`]. Output is sorted by
/// `(path, op-kind, secondary)` so two equal input pairs always produce
/// byte-identical results.
pub fn diff_trees(old: &ViewNode, new: &ViewNode) -> Vec<RenderOp> {
    let mut ops: Vec<RenderOp> = Vec::new();
    let mut path: Vec<usize> = Vec::new();
    diff_at(old, new, &mut path, &mut ops);
    ops.sort_by(|a, b| {
        a.path()
            .cmp(b.path())
            .then(a.order_tag().cmp(&b.order_tag()))
            .then(a.secondary_key().cmp(&b.secondary_key()))
    });
    ops
}

fn diff_at(old: &ViewNode, new: &ViewNode, path: &mut Vec<usize>, ops: &mut Vec<RenderOp>) {
    if old.kind != new.kind || old.key != new.key {
        // Replacement: remove the old subtree, insert the new one in
        // its place. The caller's path is already the location of the
        // node, so we attach it directly.
        ops.push(RenderOp::Remove { path: path.clone() });
        ops.push(RenderOp::Insert {
            path: path.clone(),
            node: new.clone(),
        });
        return;
    }

    // Same kind + key → diff props.
    diff_props(&old.props, &new.props, path, ops);

    // Diff children with keyed-reorder detection.
    diff_children(&old.children, &new.children, path, ops);
}

fn diff_props(
    old: &BTreeMap<String, PropValue>,
    new: &BTreeMap<String, PropValue>,
    path: &mut [usize],
    ops: &mut Vec<RenderOp>,
) {
    // Union of keys, walked in sorted order so emitted ops are stable.
    let mut keys: BTreeSet<&String> = BTreeSet::new();
    for k in old.keys() {
        keys.insert(k);
    }
    for k in new.keys() {
        keys.insert(k);
    }
    for key in keys {
        match (old.get(key), new.get(key)) {
            (Some(a), Some(b)) if a == b => {
                // No change.
            }
            (_, Some(b)) => {
                ops.push(RenderOp::UpdateProp {
                    path: path.to_vec(),
                    key: key.clone(),
                    value: b.clone(),
                });
            }
            (Some(_), None) => {
                // Removing a prop is modelled as setting it to Null so
                // the host adapter has a single op kind to dispatch on.
                ops.push(RenderOp::UpdateProp {
                    path: path.to_vec(),
                    key: key.clone(),
                    value: PropValue::Null,
                });
            }
            (None, None) => {}
        }
    }
}

fn diff_children(
    old_children: &[ViewNode],
    new_children: &[ViewNode],
    path: &mut Vec<usize>,
    ops: &mut Vec<RenderOp>,
) {
    // First, detect keyed reorders: every old keyed child whose key still
    // exists in `new` at a different index becomes a Reorder op. Only
    // emit a Reorder when the moved child's *content* matches structurally
    // (same kind and key); content updates are handled by the standard
    // recursion below.
    let mut old_keys: BTreeMap<&str, usize> = BTreeMap::new();
    for (idx, child) in old_children.iter().enumerate() {
        if let Some(k) = &child.key {
            old_keys.insert(k.as_str(), idx);
        }
    }
    for (new_idx, child) in new_children.iter().enumerate() {
        if let Some(k) = &child.key {
            if let Some(old_idx) = old_keys.get(k.as_str()).copied() {
                if old_idx != new_idx {
                    ops.push(RenderOp::Reorder {
                        path: path.to_vec(),
                        from: old_idx,
                        to: new_idx,
                    });
                }
            }
        }
    }

    // Then walk positionally to emit insert/remove/update.
    let max = old_children.len().max(new_children.len());
    for idx in 0..max {
        path.push(idx);
        match (old_children.get(idx), new_children.get(idx)) {
            (Some(o), Some(n)) => diff_at(o, n, path, ops),
            (None, Some(n)) => ops.push(RenderOp::Insert {
                path: path.clone(),
                node: n.clone(),
            }),
            (Some(_), None) => ops.push(RenderOp::Remove { path: path.clone() }),
            (None, None) => {}
        }
        path.pop();
    }
}

/// Render `tree` as canonical JSON.
pub fn render_report_json(tree: &RenderTree) -> String {
    to_json(tree)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(pairs: &[(&str, PropValue)]) -> BTreeMap<String, PropValue> {
        let mut map: BTreeMap<String, PropValue> = BTreeMap::new();
        for (k, v) in pairs {
            map.insert((*k).to_string(), v.clone());
        }
        map
    }

    fn simple_view() -> ViewDecl {
        ViewDecl::placeholder("Hello", vec![PropSlot::required_str("name")])
    }

    #[test]
    fn renders_simple_view_into_single_node() {
        let view = simple_view();
        let provided = props(&[("name", PropValue::Str("ada".into()))]);
        let tree = match render_view(&view, &provided) {
            Ok(t) => t,
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected ok, got {err}");
                }
                return;
            }
        };
        assert_eq!(tree.root.kind, "Hello");
        assert_eq!(tree.stats.node_count, 1);
        assert_eq!(tree.stats.depth, 1);
        assert_eq!(tree.schema, UI_RENDER_SCHEMA);
    }

    #[test]
    fn renders_nested_view_with_correct_depth_and_node_count() {
        let body = ViewTemplate::leaf("Container")
            .with_child(ViewTemplate::leaf("Header"))
            .with_child(ViewTemplate::leaf("Body").with_child(ViewTemplate::leaf("Paragraph")));
        let view = ViewDecl::placeholder("App", vec![]).with_body(body);
        let tree = match render_view(&view, &BTreeMap::new()) {
            Ok(t) => t,
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected ok, got {err}");
                }
                return;
            }
        };
        assert_eq!(tree.stats.node_count, 4);
        assert_eq!(tree.stats.depth, 3);
        assert_eq!(tree.stats.keyed_nodes, 0);
        assert_eq!(tree.stats.unkeyed_nodes, 4);
    }

    #[test]
    fn missing_required_prop_emits_rnd0003() {
        let view = simple_view();
        let err = match render_view(&view, &BTreeMap::new()) {
            Err(e) => e,
            Ok(_) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected error, got ok");
                }
                return;
            }
        };
        assert_eq!(err.id(), "RND0003");
        match err {
            RenderError::MissingRequiredProp { view, prop } => {
                assert_eq!(view, "Hello");
                assert_eq!(prop, "name");
            }
            other => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "wrong variant: {other:?}");
                }
            }
        }
    }

    #[test]
    fn prop_type_mismatch_emits_rnd0002() {
        let view = ViewDecl::placeholder("Counter", vec![PropSlot::required_int("value")]);
        let bad = props(&[("value", PropValue::Str("not-an-int".into()))]);
        let err = match render_view(&view, &bad) {
            Err(e) => e,
            Ok(_) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected error");
                }
                return;
            }
        };
        assert_eq!(err.id(), "RND0002");
    }

    #[test]
    fn duplicate_sibling_key_emits_rnd0004() {
        let body = ViewTemplate::leaf("List")
            .with_child(ViewTemplate::leaf("Item").with_key("a"))
            .with_child(ViewTemplate::leaf("Item").with_key("a"));
        let view = ViewDecl::placeholder("KeyedList", vec![]).with_body(body);
        let err = match render_view(&view, &BTreeMap::new()) {
            Err(e) => e,
            Ok(_) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected error");
                }
                return;
            }
        };
        assert_eq!(err.id(), "RND0004");
    }

    #[test]
    fn excessive_depth_emits_rnd0005() {
        // Build a chain of 80 nested templates.
        let mut body = ViewTemplate::leaf("Deep");
        for _ in 0..80 {
            body = ViewTemplate::leaf("Deep").with_child(body);
        }
        let view = ViewDecl::placeholder("Deep", vec![]).with_body(body);
        let err = match render_view(&view, &BTreeMap::new()) {
            Err(e) => e,
            Ok(_) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected error");
                }
                return;
            }
        };
        assert_eq!(err.id(), "RND0005");
    }

    #[test]
    fn unknown_view_kind_emits_rnd0001() {
        // Force an unknown kind by editing allowed_kinds after the fact.
        let body = ViewTemplate::leaf("Container").with_child(ViewTemplate::leaf("MysteryBox"));
        let mut view = ViewDecl::placeholder("App", vec![]).with_body(body);
        // Remove the auto-collected `MysteryBox` so it becomes unknown.
        view.allowed_kinds.remove("MysteryBox");
        let err = match render_view(&view, &BTreeMap::new()) {
            Err(e) => e,
            Ok(_) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected error");
                }
                return;
            }
        };
        assert_eq!(err.id(), "RND0001");
    }

    #[test]
    fn diff_equal_trees_is_empty() {
        let view = ViewDecl::placeholder("App", vec![])
            .with_body(ViewTemplate::leaf("Root").with_child(ViewTemplate::leaf("Child")));
        let a = match render_view(&view, &BTreeMap::new()) {
            Ok(t) => t,
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "render failed: {err}");
                }
                return;
            }
        };
        let b = a.clone();
        let ops = diff_trees(&a.root, &b.root);
        assert!(ops.is_empty(), "expected no ops, got {ops:?}");
    }

    #[test]
    fn diff_emits_insert_for_appended_child() {
        let old = ViewNode {
            kind: "List".to_string(),
            key: None,
            props: BTreeMap::new(),
            children: vec![ViewNode::leaf("A")],
        };
        let new = ViewNode {
            kind: "List".to_string(),
            key: None,
            props: BTreeMap::new(),
            children: vec![ViewNode::leaf("A"), ViewNode::leaf("B")],
        };
        let ops = diff_trees(&old, &new);
        assert_eq!(ops.len(), 1);
        match &ops[0] {
            RenderOp::Insert { path, node } => {
                assert_eq!(path, &vec![1usize]);
                assert_eq!(node.kind, "B");
            }
            other => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "unexpected op {other:?}");
                }
            }
        }
    }

    #[test]
    fn diff_emits_remove_for_dropped_child() {
        let old = ViewNode {
            kind: "List".to_string(),
            key: None,
            props: BTreeMap::new(),
            children: vec![ViewNode::leaf("A"), ViewNode::leaf("B")],
        };
        let new = ViewNode {
            kind: "List".to_string(),
            key: None,
            props: BTreeMap::new(),
            children: vec![ViewNode::leaf("A")],
        };
        let ops = diff_trees(&old, &new);
        assert_eq!(ops.len(), 1);
        match &ops[0] {
            RenderOp::Remove { path } => assert_eq!(path, &vec![1usize]),
            other => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "unexpected op {other:?}");
                }
            }
        }
    }

    #[test]
    fn diff_emits_update_prop_for_value_change() {
        let mut p_old: BTreeMap<String, PropValue> = BTreeMap::new();
        p_old.insert("color".to_string(), PropValue::Str("red".into()));
        let mut p_new: BTreeMap<String, PropValue> = BTreeMap::new();
        p_new.insert("color".to_string(), PropValue::Str("blue".into()));
        let old = ViewNode {
            kind: "Box".to_string(),
            key: None,
            props: p_old,
            children: Vec::new(),
        };
        let new = ViewNode {
            kind: "Box".to_string(),
            key: None,
            props: p_new,
            children: Vec::new(),
        };
        let ops = diff_trees(&old, &new);
        assert_eq!(ops.len(), 1);
        match &ops[0] {
            RenderOp::UpdateProp { path, key, value } => {
                assert!(path.is_empty());
                assert_eq!(key, "color");
                assert_eq!(value, &PropValue::Str("blue".into()));
            }
            other => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "unexpected op {other:?}");
                }
            }
        }
    }

    #[test]
    fn diff_detects_keyed_reorder() {
        let mk = |key: &str| ViewNode {
            kind: "Item".to_string(),
            key: Some(key.to_string()),
            props: BTreeMap::new(),
            children: Vec::new(),
        };
        let old = ViewNode {
            kind: "List".to_string(),
            key: None,
            props: BTreeMap::new(),
            children: vec![mk("a"), mk("b"), mk("c")],
        };
        let new = ViewNode {
            kind: "List".to_string(),
            key: None,
            props: BTreeMap::new(),
            children: vec![mk("c"), mk("b"), mk("a")],
        };
        let ops = diff_trees(&old, &new);
        let reorders: Vec<&RenderOp> = ops
            .iter()
            .filter(|op| matches!(op, RenderOp::Reorder { .. }))
            .collect();
        assert_eq!(reorders.len(), 2, "expected exactly two reorder ops");
        // Reorders precede the positional diff: middle child `b` stays
        // put so no reorder for it, but `a` (0→2) and `c` (2→0) move.
        for op in &reorders {
            if let RenderOp::Reorder { from, to, .. } = op {
                assert_ne!(from, to);
            }
        }
    }

    #[test]
    fn render_envelope_is_byte_deterministic() {
        let view = ViewDecl::placeholder("App", vec![PropSlot::required_str("title")]).with_body(
            ViewTemplate::leaf("App")
                .with_prop_binding("title")
                .with_child(ViewTemplate::leaf("Footer")),
        );
        let provided = props(&[("title", PropValue::Str("Welcome".into()))]);
        let first = match render_view(&view, &provided) {
            Ok(t) => render_report_json(&t),
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "render failed: {err}");
                }
                return;
            }
        };
        let second = match render_view(&view, &provided) {
            Ok(t) => render_report_json(&t),
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "render failed: {err}");
                }
                return;
            }
        };
        assert_eq!(first, second, "renders must be byte-identical");
        assert!(first.contains("\"schema\":\"ori.ui_render.v1\""));
    }

    #[test]
    fn malformed_view_with_two_required_unset_reports_first_missing() {
        let view = ViewDecl::placeholder(
            "Form",
            vec![
                PropSlot::required_str("title"),
                PropSlot::required_bool("submitted"),
            ],
        );
        let err = match render_view(&view, &BTreeMap::new()) {
            Err(e) => e,
            Ok(_) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected error");
                }
                return;
            }
        };
        // Either missing prop is acceptable; the contract is that *some*
        // RND0003 is reported.
        assert_eq!(err.id(), "RND0003");
    }

    #[test]
    fn malformed_view_with_unknown_child_kind_in_deeper_tree() {
        let body = ViewTemplate::leaf("Root")
            .with_child(ViewTemplate::leaf("Container").with_child(ViewTemplate::leaf("Ghost")));
        let mut view = ViewDecl::placeholder("App", vec![]).with_body(body);
        view.allowed_kinds.remove("Ghost");
        let err = match render_view(&view, &BTreeMap::new()) {
            Err(e) => e,
            Ok(_) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "expected error");
                }
                return;
            }
        };
        match err {
            RenderError::UnknownViewKind { path, kind } => {
                assert_eq!(kind, "Ghost");
                // Ghost lives at path [0, 0] under the root.
                assert_eq!(path, vec![0, 0]);
            }
            other => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "wrong variant: {other:?}");
                }
            }
        }
    }

    #[test]
    fn prop_bindings_are_forwarded_into_node_props() {
        let view = ViewDecl::placeholder("Hello", vec![PropSlot::required_str("name")])
            .with_body(ViewTemplate::leaf("Hello").with_prop_binding("name"));
        let provided = props(&[("name", PropValue::Str("ada".into()))]);
        let tree = match render_view(&view, &provided) {
            Ok(t) => t,
            Err(err) => {
                #[allow(clippy::assertions_on_constants)]
                {
                    assert!(false, "render failed: {err}");
                }
                return;
            }
        };
        let actual = tree.root.props.get("name");
        assert_eq!(actual, Some(&PropValue::Str("ada".into())));
    }

    #[test]
    fn diff_ops_are_sorted_deterministically() {
        // Build trees that exercise insert + remove + update at multiple
        // paths, then assert the output is sorted by path then op-kind.
        let old = ViewNode {
            kind: "Root".to_string(),
            key: None,
            props: {
                let mut m: BTreeMap<String, PropValue> = BTreeMap::new();
                m.insert("z".to_string(), PropValue::Int(1));
                m
            },
            children: vec![ViewNode::leaf("A"), ViewNode::leaf("B")],
        };
        let new = ViewNode {
            kind: "Root".to_string(),
            key: None,
            props: {
                let mut m: BTreeMap<String, PropValue> = BTreeMap::new();
                m.insert("z".to_string(), PropValue::Int(2));
                m
            },
            children: vec![ViewNode::leaf("A"), ViewNode::leaf("C")],
        };
        let ops = diff_trees(&old, &new);
        // Verify ascending path order.
        for window in ops.windows(2) {
            assert!(
                window[0].path() <= window[1].path()
                    || window[0].order_tag() <= window[1].order_tag(),
                "ops not sorted: {ops:?}"
            );
        }
        // Re-run and assert identical byte serialisation.
        let again = diff_trees(&old, &new);
        let a = to_json(&ops);
        let b = to_json(&again);
        assert_eq!(a, b, "diff must be byte-deterministic");
    }
}
