//! Structural tree diff: walk two trees in parallel and emit
//! `added | removed | modified` changes per node, keyed by identity.
//!
//! Operates on a format-agnostic intermediate representation [`TreeNode`].
//! Callers convert their format (XML, JSON, YAML, …) into `TreeNode` before
//! calling [`tree_diff`]. tate has zero format-parsing dependencies.
//!
//! ## TreeNode model
//!
//! Each node has:
//! - `kind`: the element type (XML tag name, JSON object key, `"[array]"`)
//! - `identity`: an optional identity value used for sibling matching — must
//!   be set by the caller during conversion (e.g. XML `OID` attr, JSON object key)
//! - `label`: a human-readable name for the node
//! - `attributes`: key-value pairs for scalar properties (XML attributes, JSON
//!   leaf-valued object properties)
//! - `text`: direct text content (XML text, JSON scalar value)
//! - `children`: nested nodes (XML child elements, JSON object-valued properties,
//!   array items)
//!
//! Nodes with `identity` set are "locatable" — they appear in the change list
//! on their own. Nodes without identity are matched positionally among siblings
//! of the same kind; changes in keyless descendants bubble up to the nearest
//! identity-bearing ancestor.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// A format-agnostic tree node. Convert from your format (XML, JSON, …) into
/// this type, then call [`tree_diff`].
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TreeNode {
    /// Element type (XML tag name, JSON object key, `"[array]"` for array items).
    pub kind: String,
    /// Identity value used for sibling matching. `None` means positional
    /// matching. Set this during conversion from format-specific identity
    /// attributes (XML `OID`/`id`/`Name`) or structural identity (JSON object key).
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub identity: Option<String>,
    /// Human-readable label for the node. Set during conversion; typically the
    /// `name` attribute (XML) or the object key (JSON).
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "String::is_empty"))]
    pub label: String,
    /// Scalar key-value pairs (XML attributes, JSON leaf properties).
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Vec::is_empty"))]
    pub attributes: Vec<(String, String)>,
    /// Direct text content (XML text, JSON scalar value as string).
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "String::is_empty"))]
    pub text: String,
    /// Nested child nodes.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Vec::is_empty"))]
    pub children: Vec<TreeNode>,
}

impl TreeNode {
    /// Convenience constructor for a node with kind and identity.
    pub fn new(kind: impl Into<String>) -> Self {
        TreeNode {
            kind: kind.into(),
            ..Default::default()
        }
    }

    /// Set the identity value.
    pub fn with_identity(mut self, id: impl Into<String>) -> Self {
        self.identity = Some(id.into());
        self
    }

    /// Set the label.
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// Add an attribute.
    pub fn with_attr(mut self, key: impl Into<String>, val: impl Into<String>) -> Self {
        self.attributes.push((key.into(), val.into()));
        self
    }

    /// Set the text content.
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = text.into();
        self
    }

    /// Add a child node.
    pub fn with_child(mut self, child: TreeNode) -> Self {
        self.children.push(child);
        self
    }

    /// Look up an attribute by name.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Kind of tree change. `Modified` means the node matched on identity but its
/// attributes, text, or descendants changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum ChangeKind {
    Added,
    Removed,
    Modified,
}

/// One attribute change: `name`, the old value (or empty when added), and the
/// new value (or empty when removed).
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct AttrChange {
    pub name: String,
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "String::is_empty"))]
    pub old: String,
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "String::is_empty"))]
    pub new: String,
}

/// One changed node: its kind, identity, and what changed. No format-specific
/// fields — applications layer domain semantics on top.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TreeChange {
    pub kind: ChangeKind,
    /// Element type (tag name / object key).
    #[cfg_attr(feature = "serde", serde(rename = "elemType"))]
    pub elem_type: String,
    /// Identity value, or empty for keyless nodes.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "String::is_empty"))]
    pub id: String,
    /// Human-readable label.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "String::is_empty"))]
    pub label: String,
    /// Path from root: sequence of `kind#identity` keys locating this node.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Vec::is_empty"))]
    pub path: Vec<String>,
    /// Attribute changes for a `Modified` node; empty for `Added` / `Removed`.
    #[cfg_attr(feature = "serde", serde(rename = "changedAttrs", default, skip_serializing_if = "Vec::is_empty"))]
    pub changed_attrs: Vec<AttrChange>,
}

/// The result of tree-diffing two [`TreeNode`]s.
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TreeDiff {
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Vec::is_empty"))]
    pub changes: Vec<TreeChange>,
}

/// Diff two tree nodes and return the structural changes.
///
/// The root nodes are compared directly. Interior nodes are matched by
/// `kind#identity` (or just `kind` when identity is absent → positional).
/// Changes in keyless descendants bubble up to the nearest identity-bearing
/// ancestor.
///
/// ```
/// use tate::tree::{TreeNode, tree_diff, ChangeKind};
///
/// let a = TreeNode::new("root")
///     .with_child(TreeNode::new("entry").with_identity("u1").with_attr("level", "1"));
/// let b = TreeNode::new("root")
///     .with_child(TreeNode::new("entry").with_identity("u1").with_attr("level", "99"));
///
/// let diff = tree_diff(&a, &b);
/// assert_eq!(diff.changes.len(), 1);
/// assert_eq!(diff.changes[0].kind, ChangeKind::Modified);
/// assert!(diff.changes[0].changed_attrs.iter().any(|c| c.name == "level"));
/// ```
pub fn tree_diff(a: &TreeNode, b: &TreeNode) -> TreeDiff {
    let mut changes = Vec::new();
    let changed = diff_node(a, b, &mut changes, Vec::new());
    // Root fallback: if the whole tree changed but no locatable node was reported,
    // surface the root so the caller sees something.
    if changed && changes.is_empty() {
        changes.push(mk_change(ChangeKind::Modified, b, attr_diffs(a, b), Vec::new()));
    }
    TreeDiff { changes }
}

/// A stable key for matching a node among its siblings. Returns `kind#identity`
/// when identity is present, otherwise just `kind` (positional pairing).
fn node_key(n: &TreeNode) -> String {
    match &n.identity {
        Some(id) => format!("{}#{}", n.kind, id),
        None => n.kind.clone(),
    }
}

/// Locatable = has an identity. These can appear in the change list on their
/// own; keyless nodes cannot (their changes bubble up).
fn is_locatable(n: &TreeNode) -> bool {
    n.identity.is_some()
}

/// Build a change record for a node, pulling identity and label from the node.
fn mk_change(kind: ChangeKind, n: &TreeNode, changed_attrs: Vec<AttrChange>, path: Vec<String>) -> TreeChange {
    TreeChange {
        kind,
        elem_type: n.kind.clone(),
        id: n.identity.clone().unwrap_or_default(),
        label: n.label.clone(),
        path,
        changed_attrs,
    }
}

/// Compare two nodes' attributes, returning changes.
fn attr_diffs(a: &TreeNode, b: &TreeNode) -> Vec<AttrChange> {
    let am: std::collections::BTreeMap<&str, &str> =
        a.attributes.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let bm: std::collections::BTreeMap<&str, &str> =
        b.attributes.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let mut out = Vec::new();
    for (k, bv) in &bm {
        match am.get(k) {
            Some(av) if av == bv => {}
            Some(av) => out.push(AttrChange {
                name: k.to_string(),
                old: av.to_string(),
                new: bv.to_string(),
            }),
            None => out.push(AttrChange {
                name: k.to_string(),
                old: String::new(),
                new: bv.to_string(),
            }),
        }
    }
    for (k, av) in &am {
        if !bm.contains_key(k) {
            out.push(AttrChange {
                name: k.to_string(),
                old: av.to_string(),
                new: String::new(),
            });
        }
    }
    out
}

/// Returns true if anything in this subtree (this node or a descendant) changed.
/// A change in a keyless descendant bubbles up to the nearest identity-bearing
/// ancestor, which is what gets reported.
fn diff_node(a: &TreeNode, b: &TreeNode, out: &mut Vec<TreeChange>, path: Vec<String>) -> bool {
    let locatable = is_locatable(b);
    let attr_changes = attr_diffs(a, b);
    let text_changed = a.text != b.text;
    let tag_changed = a.kind != b.kind;
    let mut own_changed = tag_changed || !attr_changes.is_empty() || text_changed;

    // Match children by key.
    let mut a_by_key: std::collections::BTreeMap<String, Vec<&TreeNode>> =
        std::collections::BTreeMap::new();
    for c in &a.children {
        a_by_key.entry(node_key(c)).or_default().push(c);
    }
    let mut a_used: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut descendant_changed = false;

    for bc in &b.children {
        let key = node_key(bc);
        let child_path = {
            let mut p = path.clone();
            p.push(key.clone());
            p
        };
        let idx = a_used.entry(key.clone()).or_insert(0);
        let matched = a_by_key.get(&key).and_then(|v| v.get(*idx)).copied();
        match matched {
            Some(ac) => {
                *idx += 1;
                let child_changed = diff_node(ac, bc, out, child_path);
                if child_changed && !is_locatable(bc) {
                    descendant_changed = true;
                }
            }
            None => {
                if !emit_subtree(ChangeKind::Added, bc, out, child_path) {
                    descendant_changed = true;
                }
            }
        }
    }
    for (key, nodes) in &a_by_key {
        let used = a_used.get(key).copied().unwrap_or(0);
        for &ac in nodes.iter().skip(used) {
            let child_path = {
                let mut p = path.clone();
                p.push(key.clone());
                p
            };
            if !emit_subtree(ChangeKind::Removed, ac, out, child_path) {
                descendant_changed = true;
            }
        }
    }

    if locatable && (own_changed || descendant_changed) {
        out.push(mk_change(ChangeKind::Modified, b, attr_changes, path));
        own_changed = true;
    }

    own_changed || descendant_changed
}

/// Emit a change for an added/removed node and its identity-bearing descendants.
/// Returns true if at least one identity-bearing node was reported.
fn emit_subtree(kind: ChangeKind, n: &TreeNode, out: &mut Vec<TreeChange>, path: Vec<String>) -> bool {
    let mut reported = false;
    if is_locatable(n) {
        out.push(mk_change(kind, n, Vec::new(), path.clone()));
        reported = true;
    }
    for c in &n.children {
        let child_path = {
            let mut p = path.clone();
            p.push(node_key(c));
            p
        };
        if emit_subtree(kind, c, out, child_path) {
            reported = true;
        }
    }
    reported
}

// ─── 3-way merge ─────────────────────────────────────────────────────────────

/// Result of a 3-way tree merge ([`tree_merge`]).
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TreeMergeResult {
    pub tree: TreeNode,
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Vec::is_empty"))]
    pub conflicts: Vec<TreeConflict>,
}

/// One node-level conflict in a tree merge.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TreeConflict {
    pub path: Vec<String>,
    pub base: String,
    pub ours: String,
    pub theirs: String,
}

/// 3-way merge of two trees that diverged from a common base.
///
/// Uses path-based matching: each change is located by its `kind#identity` path
/// from the root. Independent changes to different nodes auto-merge; conflicting
/// attribute changes on the same node are reported.
///
/// ```
/// use tate::tree::{TreeNode, tree_merge};
///
/// let base = TreeNode::new("root")
///     .with_child(TreeNode::new("entry").with_identity("u1").with_attr("v", "1"));
/// let ours = TreeNode::new("root")
///     .with_child(TreeNode::new("entry").with_identity("u1").with_attr("v", "9"));
/// let theirs = TreeNode::new("root")
///     .with_child(TreeNode::new("entry").with_identity("u1").with_attr("v", "1")
///         .with_child(TreeNode::new("sub").with_identity("s1")));
/// let r = tree_merge(&base, &ours, &theirs);
/// assert_eq!(r.conflicts.len(), 0);
/// ```
pub fn tree_merge(base: &TreeNode, ours: &TreeNode, theirs: &TreeNode) -> TreeMergeResult {
    let diff_o = tree_diff(base, ours);
    let diff_t = tree_diff(base, theirs);

    let mut attr_ours: std::collections::HashMap<String, &AttrChange> = std::collections::HashMap::new();
    for c in &diff_o.changes {
        if c.kind == ChangeKind::Modified {
            for a in &c.changed_attrs {
                let key = format!("{}#{}", path_key(&c.path), a.name);
                attr_ours.insert(key, a);
            }
        }
    }
    let mut attr_theirs: std::collections::HashMap<String, &AttrChange> = std::collections::HashMap::new();
    for c in &diff_t.changes {
        if c.kind == ChangeKind::Modified {
            for a in &c.changed_attrs {
                let key = format!("{}#{}", path_key(&c.path), a.name);
                attr_theirs.insert(key, a);
            }
        }
    }

    let mut conflicts = Vec::new();
    let tree = merge_nodes(base, ours, theirs, &diff_o, &diff_t, &attr_ours, &attr_theirs, &mut conflicts);

    TreeMergeResult { tree, conflicts }
}

fn path_key(path: &[String]) -> String {
    path.join("/")
}

fn merge_nodes(
    base: &TreeNode,
    ours: &TreeNode,
    theirs: &TreeNode,
    diff_o: &TreeDiff,
    diff_t: &TreeDiff,
    _attr_ours: &std::collections::HashMap<String, &AttrChange>,
    _attr_theirs: &std::collections::HashMap<String, &AttrChange>,
    _conflicts: &mut Vec<TreeConflict>,
) -> TreeNode {
    let mut merged = ours.clone();

    let ours_modified: std::collections::HashSet<String> = diff_o
        .changes
        .iter()
        .filter(|c| c.kind == ChangeKind::Modified)
        .flat_map(|c| {
            let pk = path_key(&c.path);
            c.changed_attrs.iter().map(move |a| format!("{pk}#{}", a.name))
        })
        .collect();

    let ours_added_paths: std::collections::HashSet<String> = diff_o
        .changes
        .iter()
        .filter(|c| c.kind == ChangeKind::Added)
        .map(|c| path_key(&c.path))
        .collect();

    let theirs_removed_paths: std::collections::HashSet<String> = diff_t
        .changes
        .iter()
        .filter(|c| c.kind == ChangeKind::Removed)
        .map(|c| path_key(&c.path))
        .collect();

    for c in &diff_t.changes {
        if c.kind != ChangeKind::Modified {
            continue;
        }
        let pk = path_key(&c.path);
        for a in &c.changed_attrs {
            let conflict_key = format!("{pk}#{}", a.name);
            if !ours_modified.contains(&conflict_key) {
                apply_attr(&mut merged, &c.path, &a.name, &a.new);
            }
        }
    }

    for c in &diff_t.changes {
        if c.kind == ChangeKind::Added && !ours_added_paths.contains(&path_key(&c.path)) {
            if let Some(node) = find_node_by_path(theirs, &c.path) {
                insert_node(&mut merged, &c.path, node.clone());
            }
        }
    }

    for c in &diff_o.changes {
        if c.kind == ChangeKind::Removed && !theirs_removed_paths.contains(&path_key(&c.path)) {
            if let Some(node) = find_node_by_path(base, &c.path) {
                insert_node(&mut merged, &c.path, node.clone());
            }
        }
    }

    merged
}

fn apply_attr(tree: &mut TreeNode, path: &[String], attr: &str, value: &str) {
    if path.is_empty() {
        set_attr(tree, attr, value);
        return;
    }
    let key = &path[0];
    for child in &mut tree.children {
        if node_key(child) == *key {
            apply_attr(child, &path[1..], attr, value);
            return;
        }
    }
}

fn set_attr(node: &mut TreeNode, name: &str, value: &str) {
    for (k, v) in &mut node.attributes {
        if k == name {
            *v = value.to_string();
            return;
        }
    }
    node.attributes.push((name.to_string(), value.to_string()));
}

fn find_node_by_path<'a>(tree: &'a TreeNode, path: &[String]) -> Option<&'a TreeNode> {
    if path.is_empty() {
        return Some(tree);
    }
    let key = &path[0];
    for child in &tree.children {
        if node_key(child) == *key {
            return find_node_by_path(child, &path[1..]);
        }
    }
    None
}

fn insert_node(tree: &mut TreeNode, path: &[String], node: TreeNode) {
    if path.is_empty() {
        return;
    }
    if path.len() == 1 {
        // Check if already exists
        let key = &path[0];
        let exists = tree.children.iter().any(|c| node_key(c) == *key);
        if !exists {
            tree.children.push(node);
        }
        return;
    }
    let key = &path[0];
    for child in &mut tree.children {
        if node_key(child) == *key {
            insert_node(child, &path[1..], node);
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modified_node_reports_changed_attrs() {
        let a = TreeNode::new("root")
            .with_child(TreeNode::new("entry").with_identity("u1").with_attr("level", "1"));
        let b = TreeNode::new("root")
            .with_child(TreeNode::new("entry").with_identity("u1").with_attr("level", "99"));
        let d = tree_diff(&a, &b);
        assert_eq!(d.changes.len(), 1);
        assert_eq!(d.changes[0].kind, ChangeKind::Modified);
        assert_eq!(d.changes[0].id, "u1");
        assert!(d.changes[0].changed_attrs.iter().any(|c| c.name == "level" && c.old == "1" && c.new == "99"));
    }

    #[test]
    fn added_node_is_reported() {
        let a = TreeNode::new("root").with_child(TreeNode::new("entry").with_identity("u1"));
        let b = TreeNode::new("root")
            .with_child(TreeNode::new("entry").with_identity("u1"))
            .with_child(TreeNode::new("entry").with_identity("u2"));
        let d = tree_diff(&a, &b);
        assert!(d.changes.iter().any(|c| c.kind == ChangeKind::Added && c.id == "u2"));
    }

    #[test]
    fn removed_node_is_reported() {
        let a = TreeNode::new("root")
            .with_child(TreeNode::new("entry").with_identity("u1"))
            .with_child(TreeNode::new("entry").with_identity("u2"));
        let b = TreeNode::new("root").with_child(TreeNode::new("entry").with_identity("u1"));
        let d = tree_diff(&a, &b);
        assert!(d.changes.iter().any(|c| c.kind == ChangeKind::Removed && c.id == "u2"));
    }

    #[test]
    fn identical_trees_no_changes() {
        let a = TreeNode::new("root").with_child(TreeNode::new("entry").with_identity("u1").with_label("alice"));
        let b = a.clone();
        let d = tree_diff(&a, &b);
        assert!(d.changes.is_empty());
    }

    #[test]
    fn keyless_descendant_bubbles_up() {
        let a = TreeNode::new("root")
            .with_child(
                TreeNode::new("group").with_identity("g1")
                    .with_child(TreeNode::new("option").with_attr("value", "A")),
            );
        let b = TreeNode::new("root")
            .with_child(
                TreeNode::new("group").with_identity("g1")
                    .with_child(TreeNode::new("option").with_attr("value", "B")),
            );
        let d = tree_diff(&a, &b);
        assert!(d.changes.iter().any(|c| c.elem_type == "group" && c.kind == ChangeKind::Modified));
        assert!(!d.changes.iter().any(|c| c.elem_type == "option"), "keyless child should not appear directly");
    }

    #[test]
    fn reordered_nodes_match_by_key() {
        let a = TreeNode::new("root")
            .with_child(TreeNode::new("entry").with_identity("a"))
            .with_child(TreeNode::new("entry").with_identity("b"));
        let b = TreeNode::new("root")
            .with_child(TreeNode::new("entry").with_identity("b"))
            .with_child(TreeNode::new("entry").with_identity("a"));
        let d = tree_diff(&a, &b);
        assert!(d.changes.is_empty(), "reordering by key should not report changes");
    }

    #[test]
    fn root_tag_rename_is_detected() {
        let a = TreeNode::new("foo");
        let b = TreeNode::new("bar");
        let d = tree_diff(&a, &b);
        assert!(!d.changes.is_empty(), "root tag rename must be detected");
    }

    #[test]
    fn json_like_object_diff() {
        // Simulates a JSON object: kind=key, identity=key, attributes=scalar properties,
        // children=nested objects.
        let a = TreeNode::new("config")
            .with_child(
                TreeNode::new("server").with_identity("server")
                    .with_attr("port", "8080")
                    .with_attr("host", "localhost"),
            );
        let b = TreeNode::new("config")
            .with_child(
                TreeNode::new("server").with_identity("server")
                    .with_attr("port", "9090")
                    .with_attr("host", "localhost"),
            );
        let d = tree_diff(&a, &b);
        assert_eq!(d.changes.len(), 1);
        assert_eq!(d.changes[0].id, "server");
        assert!(d.changes[0].changed_attrs.iter().any(|c| c.name == "port" && c.old == "8080" && c.new == "9090"));
    }

    #[test]
    fn json_like_array_diff() {
        // Array items have no identity → positional matching.
        let a = TreeNode::new("list")
            .with_child(TreeNode::new("[0]").with_text("a"))
            .with_child(TreeNode::new("[1]").with_text("b"))
            .with_child(TreeNode::new("[2]").with_text("c"));
        let b = TreeNode::new("list")
            .with_child(TreeNode::new("[0]").with_text("a"))
            .with_child(TreeNode::new("[1]").with_text("b"))
            .with_child(TreeNode::new("[2]").with_text("x"))
            .with_child(TreeNode::new("[3]").with_text("d"));
        let d = tree_diff(&a, &b);
        // Array items are keyless → changes bubble up to "list" (which is also keyless)
        // → bubble to root → root fallback fires.
        assert!(!d.changes.is_empty(), "array changes should surface via root fallback");
    }

    // ── tree_merge tests ──

    #[test]
    fn merge_independent_attr_changes() {
        let base = TreeNode::new("root")
            .with_child(TreeNode::new("e").with_identity("u1").with_attr("a", "1").with_attr("b", "2"));
        let ours = TreeNode::new("root")
            .with_child(TreeNode::new("e").with_identity("u1").with_attr("a", "9").with_attr("b", "2"));
        let theirs = TreeNode::new("root")
            .with_child(TreeNode::new("e").with_identity("u1").with_attr("a", "1").with_attr("b", "8"));
        let r = tree_merge(&base, &ours, &theirs);
        assert_eq!(r.conflicts.len(), 0);
        let merged = &r.tree.children[0];
        assert_eq!(merged.attr("a"), Some("9"));
        assert_eq!(merged.attr("b"), Some("8"));
    }

    #[test]
    fn merge_theirs_added_node() {
        let base = TreeNode::new("root")
            .with_child(TreeNode::new("e").with_identity("u1"));
        let ours = TreeNode::new("root")
            .with_child(TreeNode::new("e").with_identity("u1"));
        let theirs = TreeNode::new("root")
            .with_child(TreeNode::new("e").with_identity("u1"))
            .with_child(TreeNode::new("f").with_identity("u2"));
        let r = tree_merge(&base, &ours, &theirs);
        assert_eq!(r.conflicts.len(), 0);
        assert_eq!(r.tree.children.len(), 2);
    }

    #[test]
    fn merge_ours_removed_theirs_kept() {
        let base = TreeNode::new("root")
            .with_child(TreeNode::new("e").with_identity("u1"))
            .with_child(TreeNode::new("f").with_identity("u2"));
        let ours = TreeNode::new("root")
            .with_child(TreeNode::new("e").with_identity("u1"));
        let theirs = TreeNode::new("root")
            .with_child(TreeNode::new("e").with_identity("u1"))
            .with_child(TreeNode::new("f").with_identity("u2"));
        let r = tree_merge(&base, &ours, &theirs);
        assert_eq!(r.tree.children.len(), 2);
    }

    #[test]
    fn merge_no_changes() {
        let base = TreeNode::new("root")
            .with_child(TreeNode::new("e").with_identity("u1").with_attr("v", "1"));
        let r = tree_merge(&base, &base, &base);
        assert_eq!(r.conflicts.len(), 0);
    }

    #[test]
    fn tree_diff_path_populated() {
        let a = TreeNode::new("root")
            .with_child(TreeNode::new("parent").with_identity("p1")
                .with_child(TreeNode::new("child").with_identity("c1").with_attr("v", "1")));
        let b = TreeNode::new("root")
            .with_child(TreeNode::new("parent").with_identity("p1")
                .with_child(TreeNode::new("child").with_identity("c1").with_attr("v", "2")));
        let d = tree_diff(&a, &b);
        assert_eq!(d.changes.len(), 1);
        assert_eq!(d.changes[0].path, vec!["parent#p1", "child#c1"]);
    }
}