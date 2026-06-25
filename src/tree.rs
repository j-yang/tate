//! Structural tree diff: walk two tree documents in parallel and emit
//! `added | removed | modified` changes per node, keyed by identity attributes.
//! Schema-agnostic — no application-specific concepts (CDISC, BPMN, SVG, …)
//! leak into the output; callers layer those on top of [`TreeChange`].
//!
//! Algorithm:
//! 1. Parse both documents via `roxmltree`.
//! 2. Walk them in parallel, matching children by identity key — a stable
//!    identity drawn from the first present attribute in
//!    [`TreeOptions::identity_attrs`], falling back to the tag name for
//!    keyless nodes (positional pairing).
//! 3. For matched nodes: compare attributes and text; recurse into children.
//! 4. For unmatched nodes: emit `Added` / `Removed`, recursively surfacing
//!    their identity-bearing descendants.
//! 5. A change in a keyless descendant bubbles up to the nearest identity-bearing
//!    ancestor (which is what appears in the change list).

use roxmltree::Document;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Kind of tree change. `Modified` means the node matched on identity but its
/// attributes, text, or descendants changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Added,
    Removed,
    Modified,
}

/// One attribute change: `name`, the old value (or empty when added), and the
/// new value (or empty when removed).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttrChange {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub old: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub new: String,
}

/// One changed node: its kind, identity, and what changed. No schema-specific
/// fields — applications layer domain semantics on top.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeChange {
    pub kind: ChangeKind,
    /// Local tag name of the element (e.g. "entry", "group", "div").
    #[serde(rename = "elemType")]
    pub elem_type: String,
    /// Value of the first identity attribute present on the node, or empty when
    /// the node carries none. Applications use this to anchor a highlight in
    /// their own view; tate does not interpret it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// Human-readable label: typically the value of the `name` attribute when
    /// present, falling back to `id`. Empty when neither is set.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// Attribute changes for a `Modified` node; empty for `Added` / `Removed`.
    #[serde(rename = "changedAttrs", default, skip_serializing_if = "Vec::is_empty")]
    pub changed_attrs: Vec<AttrChange>,
}

/// The result of tree-diffing two documents: the changes and any parse notes.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TreeDiff {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<TreeChange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// Configuration for [`tree_diff_with`]. Controls which attributes are treated
/// as identity-bearing when matching nodes and deciding whether a node can
/// appear in the change list on its own. Defaults to common conventions
/// (`id`, `name`) that work across a wide range of schemas. Override when your
/// schema uses domain-specific identity attributes (e.g. `OID`/`Name` for
/// CDISC ODM, `group`/`artifactId` for Maven POM, `ref`/`id` for SVG).
#[derive(Debug, Clone)]
pub struct TreeOptions {
    /// Identity attributes considered when matching children among siblings and
    /// deciding whether a node is locatable. The first present attribute in this
    /// list wins. Defaults to `["id", "name"]`.
    pub identity_attrs: Vec<String>,
}

impl Default for TreeOptions {
    fn default() -> Self {
        TreeOptions {
            identity_attrs: vec!["id".to_string(), "name".to_string()],
        }
    }
}

/// Diff two XML strings with default [`TreeOptions`].
pub fn tree_diff(xml_a: &str, xml_b: &str) -> Result<TreeDiff, String> {
    tree_diff_with(xml_a, xml_b, &TreeOptions::default())
}

/// Diff two XML strings with explicit [`TreeOptions`].
pub fn tree_diff_with(xml_a: &str, xml_b: &str, opts: &TreeOptions) -> Result<TreeDiff, String> {
    let da = Document::parse(xml_a).map_err(|e| format!("parse A: {e}"))?;
    let db = Document::parse(xml_b).map_err(|e| format!("parse B: {e}"))?;
    let mut changes = Vec::new();
    diff_node(da.root_element(), db.root_element(), &mut changes, opts);
    Ok(TreeDiff { changes, notes: Vec::new() })
}

/// First present identity attribute on `n`, as `(attr_name, value)`.
fn first_identity<'a>(n: roxmltree::Node<'a, 'a>, opts: &'a TreeOptions) -> Option<(&'a str, &'a str)> {
    for attr in &opts.identity_attrs {
        if let Some(v) = n.attribute(attr.as_str()) {
            return Some((attr.as_str(), v));
        }
    }
    None
}

/// A stable key for matching a node among its siblings. Returns `tag#value`
/// when an identity attribute is present, otherwise just the tag (positional
/// pairing handles reordering for keyless nodes).
fn node_key(n: roxmltree::Node, opts: &TreeOptions) -> String {
    let tag = n.tag_name().name();
    if let Some((_, v)) = first_identity(n, opts) {
        format!("{tag}#{v}")
    } else {
        tag.to_string()
    }
}

fn local_tag<'a>(n: roxmltree::Node<'a, 'a>) -> &'a str {
    n.tag_name().name()
}

/// Build a change record for a node (added/removed/modified), pulling the
/// identity and label from the configured identity attributes.
fn mk_change(kind: ChangeKind, n: roxmltree::Node, changed_attrs: Vec<AttrChange>, opts: &TreeOptions) -> TreeChange {
    let elem_type = local_tag(n).to_string();
    let (id, label) = match first_identity(n, opts) {
        Some((_, v)) => {
            let name = n.attribute("name").unwrap_or("").to_string();
            let label = if !name.is_empty() { name } else { v.to_string() };
            (v.to_string(), label)
        }
        None => (String::new(), String::new()),
    };
    TreeChange { kind, elem_type, id, label, changed_attrs }
}

/// direct_text = concatenation of this node's immediate text children, whitespace-
/// normalised.
fn direct_text(n: roxmltree::Node) -> String {
    let mut s = String::new();
    for c in n.children() {
        if c.is_text() {
            s.push_str(c.text().unwrap_or(""));
        }
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// attr_diffs lists attributes that differ as AttrChange records.
fn attr_diffs(a: roxmltree::Node, b: roxmltree::Node) -> Vec<AttrChange> {
    let am: BTreeMap<&str, &str> = a.attributes().map(|x| (x.name(), x.value())).collect();
    let bm: BTreeMap<&str, &str> = b.attributes().map(|x| (x.name(), x.value())).collect();
    let mut out = Vec::new();
    for (k, bv) in &bm {
        match am.get(k) {
            Some(av) if av == bv => {}
            Some(av) => out.push(AttrChange { name: k.to_string(), old: av.to_string(), new: bv.to_string() }),
            None => out.push(AttrChange { name: k.to_string(), old: String::new(), new: bv.to_string() }),
        }
    }
    for (k, av) in &am {
        if !bm.contains_key(k) {
            out.push(AttrChange { name: k.to_string(), old: av.to_string(), new: String::new() });
        }
    }
    out
}

/// Locatable = has a configured identity attribute. These can appear in the
/// change list on their own; keyless nodes cannot (their changes bubble up).
fn node_is_locatable(n: roxmltree::Node, opts: &TreeOptions) -> bool {
    first_identity(n, opts).is_some()
}

/// Returns true if anything in this subtree (this node or a descendant) changed.
/// A change in a keyless descendant bubbles up to the nearest identity-bearing
/// ancestor, which is what gets reported.
fn diff_node(a: roxmltree::Node, b: roxmltree::Node, out: &mut Vec<TreeChange>, opts: &TreeOptions) -> bool {
    let locatable = node_is_locatable(b, opts);
    let attr_changes = attr_diffs(a, b);
    let text_changed = direct_text(a) != direct_text(b);
    let mut own_changed = !attr_changes.is_empty() || text_changed;

    let a_children: Vec<roxmltree::Node> = a.children().filter(|c| c.is_element()).collect();
    let b_children: Vec<roxmltree::Node> = b.children().filter(|c| c.is_element()).collect();
    let mut a_by_key: BTreeMap<String, Vec<roxmltree::Node>> = BTreeMap::new();
    for c in &a_children {
        a_by_key.entry(node_key(*c, opts)).or_default().push(*c);
    }
    let mut a_used: BTreeMap<String, usize> = BTreeMap::new();
    let mut descendant_changed = false;

    for &bc in &b_children {
        let key = node_key(bc, opts);
        let idx = a_used.entry(key.clone()).or_insert(0);
        let matched = a_by_key.get(&key).and_then(|v| v.get(*idx)).copied();
        match matched {
            Some(ac) => {
                *idx += 1;
                let child_changed = diff_node(ac, bc, out, opts);
                if child_changed && !node_is_locatable(bc, opts) {
                    descendant_changed = true;
                }
            }
            None => {
                if !emit_subtree(ChangeKind::Added, bc, out, opts) {
                    descendant_changed = true;
                }
            }
        }
    }
    for (key, nodes) in &a_by_key {
        let used = a_used.get(key).copied().unwrap_or(0);
        for &ac in nodes.iter().skip(used) {
            if !emit_subtree(ChangeKind::Removed, ac, out, opts) {
                descendant_changed = true;
            }
        }
    }

    if locatable && (own_changed || descendant_changed) {
        out.push(mk_change(ChangeKind::Modified, b, attr_changes, opts));
        own_changed = true;
    }

    own_changed || descendant_changed
}

/// Emit a change for an added/removed node and its identity-bearing descendants.
/// Returns true if at least one identity-bearing node was reported (so the
/// caller knows whether the change is independently locatable, or must bubble
/// up).
fn emit_subtree(kind: ChangeKind, n: roxmltree::Node, out: &mut Vec<TreeChange>, opts: &TreeOptions) -> bool {
    let mut reported = false;
    if node_is_locatable(n, opts) {
        out.push(mk_change(kind, n, Vec::new(), opts));
        reported = true;
    }
    for c in n.children().filter(|c| c.is_element()) {
        if emit_subtree(kind, c, out, opts) {
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
        let a = r#"<root><entry id="u1" name="alice" role="user" level="1"/></root>"#;
        let b = r#"<root><entry id="u1" name="alice" role="user" level="99"/></root>"#;
        let d = tree_diff(a, b).unwrap();
        assert_eq!(d.changes.len(), 1);
        assert_eq!(d.changes[0].kind, ChangeKind::Modified);
        assert_eq!(d.changes[0].id, "u1");
        assert_eq!(d.changes[0].label, "alice");
        assert!(d.changes[0].changed_attrs.iter().any(|c| c.name == "level" && c.old == "1" && c.new == "99"));
    }

    #[test]
    fn added_node_is_reported() {
        let a = r#"<root><entry id="u1"/></root>"#;
        let b = r#"<root><entry id="u1"/><entry id="u2"/></root>"#;
        let d = tree_diff(a, b).unwrap();
        assert!(d.changes.iter().any(|c| c.kind == ChangeKind::Added && c.id == "u2"));
    }

    #[test]
    fn removed_node_is_reported() {
        let a = r#"<root><entry id="u1"/><entry id="u2"/></root>"#;
        let b = r#"<root><entry id="u1"/></root>"#;
        let d = tree_diff(a, b).unwrap();
        assert!(d.changes.iter().any(|c| c.kind == ChangeKind::Removed && c.id == "u2"));
    }

    #[test]
    fn identical_docs_no_changes() {
        let a = r#"<root><entry id="u1" name="alice"/></root>"#;
        let b = r#"<root><entry id="u1" name="alice"/></root>"#;
        let d = tree_diff(a, b).unwrap();
        assert!(d.changes.is_empty());
    }

    #[test]
    fn keyless_descendant_bubbles_up() {
        // A node with identity (id) holds a keyless child (an <option> with no
        // identity attr). The change in the child must surface on the parent.
        let a = r#"<root><group id="g1" name="G"><option value="A"/></group></root>"#;
        let b = r#"<root><group id="g1" name="G"><option value="B"/></group></root>"#;
        let d = tree_diff(a, b).unwrap();
        assert!(d.changes.iter().any(|c| c.elem_type == "group" && c.kind == ChangeKind::Modified));
        assert!(!d.changes.iter().any(|c| c.elem_type == "option"), "keyless child should not appear directly");
    }

    #[test]
    fn reordered_nodes_match_by_key() {
        let a = r#"<root><entry id="a"/><entry id="b"/></root>"#;
        let b = r#"<root><entry id="b"/><entry id="a"/></root>"#;
        let d = tree_diff(a, b).unwrap();
        assert!(d.changes.is_empty(), "reordering by key should not report changes");
    }

    #[test]
    fn parse_error_returned() {
        let result = tree_diff("<not xml", "<root/>");
        assert!(result.is_err());
    }

    #[test]
    fn custom_identity_attrs() {
        // A schema that uses `ref` instead of `id` for identity: with default
        // options nothing is locatable so the entries appear as positional
        // (modifications bubble through); with TreeOptions we get clean matches.
        let a = r#"<root><node ref="x"/><node ref="y"/></root>"#;
        let b = r#"<root><node ref="y"/><node ref="x"/></root>"#;
        // Default: ref is not an identity attr → both nodes match by tag only,
        // paired positionally, each subtree equal → no changes.
        let d_default = tree_diff(a, b).unwrap();
        assert!(d_default.changes.is_empty());

        // Custom: declare ref as identity so reordering is matched by key.
        // (Still no changes — identity matching just makes it more robust.)
        let opts = TreeOptions { identity_attrs: vec!["ref".to_string()] };
        let d_custom = tree_diff_with(a, b, &opts).unwrap();
        assert!(d_custom.changes.is_empty());
    }

    #[test]
    fn custom_identity_attrs_detects_actual_change() {
        let a = r#"<doc><node ref="x" value="1"/><node ref="y" value="2"/></doc>"#;
        let b = r#"<doc><node ref="y" value="2"/><node ref="x" value="9"/></doc>"#;
        let opts = TreeOptions { identity_attrs: vec!["ref".to_string()] };
        let d = tree_diff_with(a, b, &opts).unwrap();
        assert_eq!(d.changes.len(), 1);
        assert_eq!(d.changes[0].id, "x");
        assert!(d.changes[0].changed_attrs.iter().any(|c| c.name == "value" && c.old == "1" && c.new == "9"));
    }
}