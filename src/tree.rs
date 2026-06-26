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
    let changed = diff_node(a, b, &mut changes);
    // Root fallback: if the whole tree changed but no locatable node was reported,
    // surface the root so the caller sees something.
    if changed && changes.is_empty() {
        changes.push(mk_change(ChangeKind::Modified, b, attr_diffs(a, b)));
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
fn mk_change(kind: ChangeKind, n: &TreeNode, changed_attrs: Vec<AttrChange>) -> TreeChange {
    TreeChange {
        kind,
        elem_type: n.kind.clone(),
        id: n.identity.clone().unwrap_or_default(),
        label: n.label.clone(),
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
fn diff_node(a: &TreeNode, b: &TreeNode, out: &mut Vec<TreeChange>) -> bool {
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
        let idx = a_used.entry(key.clone()).or_insert(0);
        let matched = a_by_key.get(&key).and_then(|v| v.get(*idx)).copied();
        match matched {
            Some(ac) => {
                *idx += 1;
                let child_changed = diff_node(ac, bc, out);
                if child_changed && !is_locatable(bc) {
                    descendant_changed = true;
                }
            }
            None => {
                if !emit_subtree(ChangeKind::Added, bc, out) {
                    descendant_changed = true;
                }
            }
        }
    }
    for (key, nodes) in &a_by_key {
        let used = a_used.get(key).copied().unwrap_or(0);
        for &ac in nodes.iter().skip(used) {
            if !emit_subtree(ChangeKind::Removed, ac, out) {
                descendant_changed = true;
            }
        }
    }

    if locatable && (own_changed || descendant_changed) {
        out.push(mk_change(ChangeKind::Modified, b, attr_changes));
        own_changed = true;
    }

    own_changed || descendant_changed
}

/// Emit a change for an added/removed node and its identity-bearing descendants.
/// Returns true if at least one identity-bearing node was reported.
fn emit_subtree(kind: ChangeKind, n: &TreeNode, out: &mut Vec<TreeChange>) -> bool {
    let mut reported = false;
    if is_locatable(n) {
        out.push(mk_change(kind, n, Vec::new()));
        reported = true;
    }
    for c in &n.children {
        if emit_subtree(kind, c, out) {
            reported = true;
        }
    }
    reported
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
}