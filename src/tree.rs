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
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
    /// Text-content change for a `Modified` node, if its `text` differed:
    /// `(old, new)`. `None` when the text was unchanged. Text is the scalar
    /// payload of JSON values, XML text nodes, and grid cells, so a merge must
    /// treat it on equal footing with attributes.
    #[cfg_attr(feature = "serde", serde(rename = "changedText", default, skip_serializing_if = "Option::is_none"))]
    pub changed_text: Option<(String, String)>,
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
        changes.push(mk_change(ChangeKind::Modified, b, attr_diffs(a, b), text_diff(a, b), Vec::new()));
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
fn mk_change(
    kind: ChangeKind,
    n: &TreeNode,
    changed_attrs: Vec<AttrChange>,
    changed_text: Option<(String, String)>,
    path: Vec<String>,
) -> TreeChange {
    TreeChange {
        kind,
        elem_type: n.kind.clone(),
        id: n.identity.clone().unwrap_or_default(),
        label: n.label.clone(),
        path,
        changed_attrs,
        changed_text,
    }
}

/// The text change between two nodes, or `None` if their text is equal.
fn text_diff(a: &TreeNode, b: &TreeNode) -> Option<(String, String)> {
    if a.text != b.text {
        Some((a.text.clone(), b.text.clone()))
    } else {
        None
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
        let changed_text = if text_changed {
            Some((a.text.clone(), b.text.clone()))
        } else {
            None
        };
        out.push(mk_change(ChangeKind::Modified, b, attr_changes, changed_text, path));
        own_changed = true;
    }

    own_changed || descendant_changed
}

/// Emit a change for an added/removed node and its identity-bearing descendants.
/// Returns true if at least one identity-bearing node was reported.
fn emit_subtree(kind: ChangeKind, n: &TreeNode, out: &mut Vec<TreeChange>, path: Vec<String>) -> bool {
    let mut reported = false;
    if is_locatable(n) {
        out.push(mk_change(kind, n, Vec::new(), None, path.clone()));
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

/// The kind of gluing obstruction that produced a [`TreeConflict`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
pub enum ConflictKind {
    /// Both sides set the same attribute on the same node to different values.
    Attr,
    /// Both sides changed the same node's text content to different values.
    Text,
    /// Both sides added a node at the same path with differing content.
    AddAdd,
    /// One side modified a node the other side removed.
    ModifyDelete,
}

/// One node-level conflict in a tree merge — a point where the two sections
/// disagree and cannot be glued. The merged tree still contains a best-effort
/// value (favouring `ours`); this record flags that the choice was forced.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TreeConflict {
    pub kind: ConflictKind,
    /// Path from root (`kind#identity` segments) locating the conflicting node.
    pub path: Vec<String>,
    /// Attribute name for [`ConflictKind::Attr`]; empty for node-level conflicts.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "String::is_empty"))]
    pub attr: String,
    pub base: String,
    pub ours: String,
    pub theirs: String,
}

/// 3-way merge of two trees that diverged from a common base.
///
/// This is the **display-oriented** merge: it drives off [`tree_diff`] (the
/// lossy, human-facing diff) so that its [`TreeConflict`]s carry rich,
/// path-and-attribute-level detail for a UI (which attribute, old/ours/theirs
/// text, add/add vs modify/delete). It is a **total function** — it always
/// returns a tree carrying a best-effort value (favouring `ours`); a non-empty
/// `conflicts` list means that value was forced and needs review.
///
/// # Relationship to the exact pushout
///
/// The *precise* statement "merge is the **pushout** of the span
/// `ours ← base → theirs`" is realised by
/// [`crate::patch::merge_sections`], which computes the pushout in the
/// sheaf category on the tree space (the identity poset with its Alexandrov
/// topology): a pointwise per-field pushout followed by **sheafification**
/// that drops present nodes whose parent is absent (referential integrity).
/// Its conflict set has two classes: `Field` (per-stalk value disagreements,
/// the only kind a discrete model sees) and `Dangling` (structural
/// obstructions from the ancestry topology, invisible to any discrete
/// per-field merge).
///
/// `tree_merge` is the **display-oriented** merge: it drives off [`tree_diff`]
/// (the lossy, human-facing diff) so that its [`TreeConflict`]s carry rich,
/// path-and-attribute-level detail for a UI. It does not perform
/// sheafification; use `merge_sections` when you want the algebra with
/// structural-conflict detection.
///
/// See `paper/main.tex` §4 for the full treatment.
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

    let conflicts = detect_conflicts(base, ours, theirs, &diff_o, &diff_t);
    let tree = merge_nodes(base, ours, theirs, &diff_o, &diff_t);

    TreeMergeResult { tree, conflicts }
}

fn path_key(path: &[String]) -> String {
    path.join("/")
}

/// Find the gluing obstructions between the two branches.
///
/// A 3-way merge glues two sections (`ours`, `theirs`) that both restrict to
/// `base` on the unchanged locus. Where their changes overlap incompatibly the
/// gluing fails; each such point is a [`TreeConflict`]. The merged tree still
/// carries a best-effort value (favouring `ours`) — these records make the
/// forced choice explicit rather than silently dropping `theirs`.
fn detect_conflicts(
    base: &TreeNode,
    ours: &TreeNode,
    theirs: &TreeNode,
    diff_o: &TreeDiff,
    diff_t: &TreeDiff,
) -> Vec<TreeConflict> {
    let mut conflicts = Vec::new();

    // Index each side's attribute modifications by path#attr → the change.
    let attr_index = |diff: &TreeDiff| -> std::collections::HashMap<String, AttrChange> {
        let mut m = std::collections::HashMap::new();
        for c in &diff.changes {
            if c.kind == ChangeKind::Modified {
                for a in &c.changed_attrs {
                    m.insert(format!("{}#{}", path_key(&c.path), a.name), a.clone());
                }
            }
        }
        m
    };
    let attr_o = attr_index(diff_o);
    let attr_t = attr_index(diff_t);

    // (1) attr/attr: both sides set the same attribute to different values.
    for (key, ao) in &attr_o {
        if let Some(at) = attr_t.get(key) {
            if ao.new != at.new {
                let (path, attr) = split_attr_key(key);
                conflicts.push(TreeConflict {
                    kind: ConflictKind::Attr,
                    path,
                    attr,
                    base: ao.old.clone(),
                    ours: ao.new.clone(),
                    theirs: at.new.clone(),
                });
            }
        }
    }

    // (1b) text/text: both sides changed the same node's text differently.
    let text_index = |diff: &TreeDiff| -> std::collections::HashMap<String, (String, String)> {
        let mut m = std::collections::HashMap::new();
        for c in &diff.changes {
            if c.kind == ChangeKind::Modified {
                if let Some(t) = &c.changed_text {
                    m.insert(path_key(&c.path), t.clone());
                }
            }
        }
        m
    };
    let text_o = text_index(diff_o);
    let text_t = text_index(diff_t);
    for (pk, to) in &text_o {
        if let Some(tt) = text_t.get(pk) {
            // to = (base_text, ours_text), tt = (base_text, theirs_text).
            if to.1 != tt.1 {
                let path: Vec<String> = if pk.is_empty() {
                    Vec::new()
                } else {
                    pk.split('/').map(String::from).collect()
                };
                conflicts.push(TreeConflict {
                    kind: ConflictKind::Text,
                    path,
                    attr: String::new(),
                    base: to.0.clone(),
                    ours: to.1.clone(),
                    theirs: tt.1.clone(),
                });
            }
        }
    }

    let paths_of = |diff: &TreeDiff, kind: ChangeKind| -> std::collections::HashSet<String> {
        diff.changes
            .iter()
            .filter(|c| c.kind == kind)
            .map(|c| path_key(&c.path))
            .collect()
    };
    let added_o = paths_of(diff_o, ChangeKind::Added);
    let removed_o = paths_of(diff_o, ChangeKind::Removed);
    let removed_t = paths_of(diff_t, ChangeKind::Removed);
    let modified_o = paths_of(diff_o, ChangeKind::Modified);
    let modified_t = paths_of(diff_t, ChangeKind::Modified);

    // (2) add/add: both sides added a node at the same path with differing content.
    for c in &diff_t.changes {
        if c.kind != ChangeKind::Added {
            continue;
        }
        let pk = path_key(&c.path);
        if !added_o.contains(&pk) {
            continue;
        }
        let node_o = find_node_by_path(ours, &c.path);
        let node_t = find_node_by_path(theirs, &c.path);
        if node_o != node_t {
            conflicts.push(TreeConflict {
                kind: ConflictKind::AddAdd,
                path: c.path.clone(),
                attr: String::new(),
                base: String::new(),
                ours: node_label(node_o),
                theirs: node_label(node_t),
            });
        }
    }

    // (3) modify/delete: one side modified a node the other side removed.
    for pk in modified_o.iter() {
        if removed_t.contains(pk) {
            conflicts.push(mk_modify_delete(diff_o, base, pk, /*ours_modified=*/ true));
        }
    }
    for pk in modified_t.iter() {
        if removed_o.contains(pk) {
            conflicts.push(mk_modify_delete(diff_t, base, pk, /*ours_modified=*/ false));
        }
    }

    conflicts
}

/// Split a `path#attr` key back into its path segments and attribute name.
fn split_attr_key(key: &str) -> (Vec<String>, String) {
    match key.rsplit_once('#') {
        Some((path, attr)) => (
            if path.is_empty() { Vec::new() } else { path.split('/').map(String::from).collect() },
            attr.to_string(),
        ),
        None => (Vec::new(), key.to_string()),
    }
}

/// A short human label for a node, for conflict reporting.
fn node_label(n: Option<&TreeNode>) -> String {
    match n {
        Some(n) if !n.label.is_empty() => n.label.clone(),
        Some(n) => node_key(n),
        None => String::new(),
    }
}

/// Build a modify/delete conflict. `ours_modified` records which side made the
/// modification (the other side removed the node).
fn mk_modify_delete(mod_diff: &TreeDiff, base: &TreeNode, pk: &str, ours_modified: bool) -> TreeConflict {
    let path: Vec<String> = if pk.is_empty() {
        Vec::new()
    } else {
        pk.split('/').map(String::from).collect()
    };
    let base_label = node_label(find_node_by_path(base, &path));
    // Describe the modification concisely from the diff record.
    let mod_desc = mod_diff
        .changes
        .iter()
        .find(|c| c.kind == ChangeKind::Modified && path_key(&c.path) == pk)
        .map(|c| {
            let mut parts: Vec<String> =
                c.changed_attrs.iter().map(|a| format!("{}={}", a.name, a.new)).collect();
            if let Some((_, new_text)) = &c.changed_text {
                parts.push(format!("text={new_text}"));
            }
            if parts.is_empty() {
                "modified".to_string()
            } else {
                parts.join(", ")
            }
        })
        .unwrap_or_else(|| "modified".to_string());
    let (ours, theirs) = if ours_modified {
        (mod_desc, "deleted".to_string())
    } else {
        ("deleted".to_string(), mod_desc)
    };
    TreeConflict {
        kind: ConflictKind::ModifyDelete,
        path,
        attr: String::new(),
        base: base_label,
        ours,
        theirs,
    }
}

fn merge_nodes(
    base: &TreeNode,
    ours: &TreeNode,
    theirs: &TreeNode,
    diff_o: &TreeDiff,
    diff_t: &TreeDiff,
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

    // Paths where ours changed the node's text — theirs' text change there is
    // either a no-op (same value) or a conflict (already recorded), so we keep
    // ours and do not overwrite.
    let ours_text_modified: std::collections::HashSet<String> = diff_o
        .changes
        .iter()
        .filter(|c| c.kind == ChangeKind::Modified && c.changed_text.is_some())
        .map(|c| path_key(&c.path))
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
        // Apply theirs' text change unless ours also changed this node's text
        // (a clean text edit on one side only auto-merges; a two-sided text
        // change is either identical or already flagged as a Text conflict).
        if let Some((_, new_text)) = &c.changed_text {
            if !ours_text_modified.contains(&pk) {
                apply_text(&mut merged, &c.path, new_text);
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

/// Set the text content of the node at `path`, mirroring [`apply_attr`].
fn apply_text(tree: &mut TreeNode, path: &[String], value: &str) {
    if path.is_empty() {
        tree.text = value.to_string();
        return;
    }
    let key = &path[0];
    for child in &mut tree.children {
        if node_key(child) == *key {
            apply_text(child, &path[1..], value);
            return;
        }
    }
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

    // ── conflict detection: gluing obstructions must be recorded, not swallowed ──

    #[test]
    fn conflicting_attr_change_is_recorded() {
        // Both sides change the SAME attribute to DIFFERENT values → attr conflict.
        let base = TreeNode::new("root")
            .with_child(TreeNode::new("e").with_identity("u1").with_attr("v", "1"));
        let ours = TreeNode::new("root")
            .with_child(TreeNode::new("e").with_identity("u1").with_attr("v", "9"));
        let theirs = TreeNode::new("root")
            .with_child(TreeNode::new("e").with_identity("u1").with_attr("v", "7"));
        let r = tree_merge(&base, &ours, &theirs);
        assert_eq!(r.conflicts.len(), 1, "divergent attr edit must conflict");
        let c = &r.conflicts[0];
        assert_eq!(c.kind, ConflictKind::Attr);
        assert_eq!(c.attr, "v");
        assert_eq!(c.path, vec!["e#u1"]);
        assert_eq!((c.base.as_str(), c.ours.as_str(), c.theirs.as_str()), ("1", "9", "7"));
        // Merged tree still carries a best-effort value (favour ours).
        assert_eq!(r.tree.children[0].attr("v"), Some("9"));
    }

    #[test]
    fn same_attr_same_value_is_not_a_conflict() {
        // Both sides make the identical edit → no obstruction (glues cleanly).
        let base = TreeNode::new("root")
            .with_child(TreeNode::new("e").with_identity("u1").with_attr("v", "1"));
        let side = TreeNode::new("root")
            .with_child(TreeNode::new("e").with_identity("u1").with_attr("v", "9"));
        let r = tree_merge(&base, &side, &side);
        assert_eq!(r.conflicts.len(), 0);
        assert_eq!(r.tree.children[0].attr("v"), Some("9"));
    }

    #[test]
    fn independent_attr_changes_do_not_conflict() {
        // Different attributes on the same node → both apply, no conflict.
        let base = TreeNode::new("root")
            .with_child(TreeNode::new("e").with_identity("u1").with_attr("a", "1").with_attr("b", "2"));
        let ours = TreeNode::new("root")
            .with_child(TreeNode::new("e").with_identity("u1").with_attr("a", "9").with_attr("b", "2"));
        let theirs = TreeNode::new("root")
            .with_child(TreeNode::new("e").with_identity("u1").with_attr("a", "1").with_attr("b", "8"));
        let r = tree_merge(&base, &ours, &theirs);
        assert_eq!(r.conflicts.len(), 0);
    }

    #[test]
    fn add_add_divergent_is_recorded() {
        // Both sides add a node at the same path but with different content.
        let base = TreeNode::new("root");
        let ours = TreeNode::new("root")
            .with_child(TreeNode::new("e").with_identity("u1").with_attr("v", "ours"));
        let theirs = TreeNode::new("root")
            .with_child(TreeNode::new("e").with_identity("u1").with_attr("v", "theirs"));
        let r = tree_merge(&base, &ours, &theirs);
        assert_eq!(r.conflicts.len(), 1);
        assert_eq!(r.conflicts[0].kind, ConflictKind::AddAdd);
        assert_eq!(r.conflicts[0].path, vec!["e#u1"]);
    }

    #[test]
    fn add_add_identical_is_not_a_conflict() {
        // Both sides add the SAME node → glues cleanly.
        let base = TreeNode::new("root");
        let node = TreeNode::new("e").with_identity("u1").with_attr("v", "same");
        let side = TreeNode::new("root").with_child(node);
        let r = tree_merge(&base, &side, &side);
        assert_eq!(r.conflicts.len(), 0);
        assert_eq!(r.tree.children.len(), 1);
    }

    #[test]
    fn modify_delete_is_recorded() {
        // ours modifies u1; theirs deletes u1 → modify/delete conflict.
        let base = TreeNode::new("root")
            .with_child(TreeNode::new("e").with_identity("u1").with_attr("v", "1"));
        let ours = TreeNode::new("root")
            .with_child(TreeNode::new("e").with_identity("u1").with_attr("v", "9"));
        let theirs = TreeNode::new("root");
        let r = tree_merge(&base, &ours, &theirs);
        assert_eq!(r.conflicts.len(), 1);
        let c = &r.conflicts[0];
        assert_eq!(c.kind, ConflictKind::ModifyDelete);
        assert_eq!(c.path, vec!["e#u1"]);
        assert_eq!(c.theirs, "deleted");
        assert!(c.ours.contains("v=9"));
    }

    #[test]
    fn delete_modify_is_recorded_symmetrically() {
        // Mirror of the above: theirs modifies, ours deletes.
        let base = TreeNode::new("root")
            .with_child(TreeNode::new("e").with_identity("u1").with_attr("v", "1"));
        let ours = TreeNode::new("root");
        let theirs = TreeNode::new("root")
            .with_child(TreeNode::new("e").with_identity("u1").with_attr("v", "9"));
        let r = tree_merge(&base, &ours, &theirs);
        assert_eq!(r.conflicts.len(), 1);
        let c = &r.conflicts[0];
        assert_eq!(c.kind, ConflictKind::ModifyDelete);
        assert_eq!(c.ours, "deleted");
        assert!(c.theirs.contains("v=9"));
    }

    #[test]
    fn text_change_is_applied_not_dropped() {
        // ours changes text; theirs untouched → merge keeps ours' text.
        // theirs changes a different node's text → both survive.
        let base = TreeNode::new("root")
            .with_child(TreeNode::new("a").with_identity("u1").with_text("1"))
            .with_child(TreeNode::new("b").with_identity("u2").with_text("2"));
        let ours = TreeNode::new("root")
            .with_child(TreeNode::new("a").with_identity("u1").with_text("ONE"))
            .with_child(TreeNode::new("b").with_identity("u2").with_text("2"));
        let theirs = TreeNode::new("root")
            .with_child(TreeNode::new("a").with_identity("u1").with_text("1"))
            .with_child(TreeNode::new("b").with_identity("u2").with_text("TWO"));
        let r = tree_merge(&base, &ours, &theirs);
        assert_eq!(r.conflicts.len(), 0, "disjoint text edits must not conflict");
        assert_eq!(r.tree.children[0].text, "ONE", "ours' text kept");
        assert_eq!(r.tree.children[1].text, "TWO", "theirs' text applied, not dropped");
    }

    #[test]
    fn conflicting_text_change_is_recorded() {
        // Both change the same node's text to different values → Text conflict.
        let base = TreeNode::new("root")
            .with_child(TreeNode::new("cell").with_identity("c1").with_text("orig"));
        let ours = TreeNode::new("root")
            .with_child(TreeNode::new("cell").with_identity("c1").with_text("mine"));
        let theirs = TreeNode::new("root")
            .with_child(TreeNode::new("cell").with_identity("c1").with_text("yours"));
        let r = tree_merge(&base, &ours, &theirs);
        assert_eq!(r.conflicts.len(), 1);
        let c = &r.conflicts[0];
        assert_eq!(c.kind, ConflictKind::Text);
        assert_eq!(c.path, vec!["cell#c1"]);
        assert_eq!((c.base.as_str(), c.ours.as_str(), c.theirs.as_str()), ("orig", "mine", "yours"));
    }

    #[test]
    fn same_text_change_both_sides_is_not_a_conflict() {
        let base = TreeNode::new("root")
            .with_child(TreeNode::new("cell").with_identity("c1").with_text("orig"));
        let side = TreeNode::new("root")
            .with_child(TreeNode::new("cell").with_identity("c1").with_text("same"));
        let r = tree_merge(&base, &side, &side);
        assert_eq!(r.conflicts.len(), 0);
        assert_eq!(r.tree.children[0].text, "same");
    }

    #[test]
    fn text_change_reported_in_diff() {
        // Regression: tree_diff must surface a text change in changed_text.
        let a = TreeNode::new("root").with_child(TreeNode::new("n").with_identity("u1").with_text("old"));
        let b = TreeNode::new("root").with_child(TreeNode::new("n").with_identity("u1").with_text("new"));
        let d = tree_diff(&a, &b);
        assert_eq!(d.changes.len(), 1);
        assert_eq!(d.changes[0].changed_text, Some(("old".to_string(), "new".to_string())));
    }

    #[test]
    fn clean_merge_reports_no_conflicts() {
        // The pre-existing merge_independent_attr_changes scenario must stay clean.
        let base = TreeNode::new("root")
            .with_child(TreeNode::new("e").with_identity("u1").with_attr("a", "1"));
        let ours = TreeNode::new("root")
            .with_child(TreeNode::new("e").with_identity("u1").with_attr("a", "1"))
            .with_child(TreeNode::new("f").with_identity("u2"));
        let theirs = TreeNode::new("root")
            .with_child(TreeNode::new("e").with_identity("u1").with_attr("a", "2"));
        let r = tree_merge(&base, &ours, &theirs);
        // ours adds u2, theirs modifies u1 — disjoint → no conflict.
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