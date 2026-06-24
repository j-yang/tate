//! Structural tree diff: walk two tree documents (XML today, other tree formats
//! later) in parallel and emit `added | removed | modified` changes per node,
//! keyed by identity attributes. Schema-agnostic — no CDISC / define.xml /
//! domain / variable semantics here; callers layer those on top of [`TreeChange`].
//!
//! Algorithm (ported from `shtuka-core/src/xml.rs`, sans the CDISC-specific
//! `ItemDef` / `CodeList` / `id_prefix` logic):
//! 1. Parse both documents via `roxmltree`.
//! 2. Walk them in parallel, matching children by [`node_key`] — a stable
//!    identity drawn from the first present attribute in
//!    `["OID", "Name", "CodedValue", "ItemOID", "MethodOID", "leafID", "Context"]`,
//!    falling back to tag name for keyless nodes (positional pairing).
//! 3. For matched nodes: compare attributes and text; recurse into children.
//! 4. For unmatched nodes: emit `Added` / `Removed`, recursively surfacing their
//!    identity-bearing descendants.
//! 5. A change in a keyless descendant bubbles up to the nearest identity-bearing
//!    ancestor (which is what appears in the change list).
//!
//! The output [`TreeChange`] is generic: it has `kind`, `elem`, `oid`, `label`,
//! `changed_attrs`, but no schema-specific fields. Applications that know their
//! schema (e.g. CDISC define.xml) wrap this with their own adapter that maps
//! `elem` + `oid` to domain/variable/codelist semantics.

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
/// fields — applications layer domain/variable/codelist semantics on top.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeChange {
    pub kind: ChangeKind,
    /// Local tag name of the element (e.g. "ItemDef", "CodeList", "div").
    #[serde(rename = "elemType")]
    pub elem_type: String,
    /// Value of the `OID` attribute (or empty when absent).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub oid: String,
    /// Value of the `Name` attribute (or empty when absent).
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

/// Diff two XML strings and return the structural changes.
pub fn tree_diff(xml_a: &str, xml_b: &str) -> Result<TreeDiff, String> {
    let da = Document::parse(xml_a).map_err(|e| format!("parse A: {e}"))?;
    let db = Document::parse(xml_b).map_err(|e| format!("parse B: {e}"))?;
    let mut changes = Vec::new();
    diff_node(da.root_element(), db.root_element(), &mut changes);
    Ok(TreeDiff { changes, notes: Vec::new() })
}

/// A stable key for matching a node among its siblings. Many tree formats have
/// no OID/Name — their identity is another attribute. First present in the
/// priority list wins. Callers with domain-specific identity attributes can
/// wrap this or build their own key function.
fn node_key(n: roxmltree::Node) -> String {
    let tag = n.tag_name().name();
    for attr in ["OID", "Name", "CodedValue", "ItemOID", "MethodOID", "leafID", "Context"] {
        if let Some(v) = n.attribute(attr) {
            return format!("{tag}#{v}");
        }
    }
    tag.to_string()
}

fn local_tag<'a>(n: roxmltree::Node<'a, 'a>) -> &'a str {
    n.tag_name().name()
}

/// Build a change record for a node (added/removed), pulling location hints.
fn mk_change(kind: ChangeKind, n: roxmltree::Node, changed_attrs: Vec<AttrChange>) -> TreeChange {
    let elem_type = local_tag(n).to_string();
    let oid = n.attribute("OID").unwrap_or("").to_string();
    let name = n.attribute("Name").unwrap_or("").to_string();
    let label = if !name.is_empty() { name.clone() } else { oid.clone() };
    TreeChange {
        kind,
        elem_type,
        oid,
        label,
        changed_attrs,
    }
}

/// Canonical signature of an element subtree (tag + sorted attrs + text +
/// children, recursively). Two structurally identical subtrees produce the same
/// string, so we can detect a change in any descendant by comparing signatures.
#[allow(dead_code)]
fn subtree_sig(n: roxmltree::Node) -> String {
    let mut s = String::new();
    s.push('<');
    s.push_str(n.tag_name().name());
    let mut attrs: Vec<(&str, &str)> = n.attributes().map(|a| (a.name(), a.value())).collect();
    attrs.sort_unstable();
    for (k, v) in attrs {
        s.push(' ');
        s.push_str(k);
        s.push('=');
        s.push_str(v);
    }
    s.push('>');
    let t = direct_text(n);
    if !t.is_empty() {
        s.push_str(&t);
    }
    for c in n.children().filter(|c| c.is_element()) {
        s.push_str(&subtree_sig(c));
    }
    s
}

/// direct_text = concatenation of this node's immediate text children (trimmed).
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

/// Locatable = has a rendered anchor (OID/Name). These can appear in the change
/// list and be highlighted; keyed-only nodes (CodedValue/ItemOID) cannot.
fn node_is_locatable(n: roxmltree::Node) -> bool {
    n.attribute("OID").is_some() || n.attribute("Name").is_some()
}

/// Returns true if anything in this subtree (this node or a descendant) changed.
/// A change in a keyless descendant bubbles up to the nearest identity-bearing
/// ancestor, which is what gets reported.
fn diff_node(a: roxmltree::Node, b: roxmltree::Node, out: &mut Vec<TreeChange>) -> bool {
    let locatable = node_is_locatable(b);
    let attr_changes = attr_diffs(a, b);
    let text_changed = direct_text(a) != direct_text(b);
    let mut own_changed = !attr_changes.is_empty() || text_changed;

    let a_children: Vec<roxmltree::Node> = a.children().filter(|c| c.is_element()).collect();
    let b_children: Vec<roxmltree::Node> = b.children().filter(|c| c.is_element()).collect();
    let mut a_by_key: BTreeMap<String, Vec<roxmltree::Node>> = BTreeMap::new();
    for c in &a_children {
        a_by_key.entry(node_key(*c)).or_default().push(*c);
    }
    let mut a_used: BTreeMap<String, usize> = BTreeMap::new();
    let mut descendant_changed = false;

    for &bc in &b_children {
        let key = node_key(bc);
        let idx = a_used.entry(key.clone()).or_insert(0);
        let matched = a_by_key.get(&key).and_then(|v| v.get(*idx)).copied();
        match matched {
            Some(ac) => {
                *idx += 1;
                let child_changed = diff_node(ac, bc, out);
                if child_changed && !node_is_locatable(bc) {
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
        let ch = mk_change(ChangeKind::Modified, b, attr_changes);
        out.push(ch);
        own_changed = true;
    }

    own_changed || descendant_changed
}

/// Emit a change for an added/removed node and its identity-bearing descendants.
/// Returns true if at least one OID/Name-bearing node was reported (so the caller
/// knows whether the change is independently locatable, or must bubble up).
fn emit_subtree(kind: ChangeKind, n: roxmltree::Node, out: &mut Vec<TreeChange>) -> bool {
    let mut reported = false;
    if n.attribute("OID").is_some() || n.attribute("Name").is_some() {
        out.push(mk_change(kind, n, Vec::new()));
        reported = true;
    }
    for c in n.children().filter(|c| c.is_element()) {
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
        let a = r#"<ODM><ItemDef OID="IT.DM.SEX" Name="SEX" DataType="text" Length="1"/></ODM>"#;
        let b = r#"<ODM><ItemDef OID="IT.DM.SEX" Name="SEX" DataType="text" Length="99"/></ODM>"#;
        let d = tree_diff(a, b).unwrap();
        assert_eq!(d.changes.len(), 1);
        assert_eq!(d.changes[0].kind, ChangeKind::Modified);
        assert!(d.changes[0].changed_attrs.iter().any(|c| c.name == "Length"));
        assert!(d.changes[0].changed_attrs.iter().any(|c| c.name == "Length" && c.old == "1" && c.new == "99"));
    }

    #[test]
    fn added_node_is_reported() {
        let a = r#"<ODM><ItemDef OID="IT.DM.AGE"/></ODM>"#;
        let b = r#"<ODM><ItemDef OID="IT.DM.AGE"/><ItemDef OID="IT.DM.SEX"/></ODM>"#;
        let d = tree_diff(a, b).unwrap();
        assert!(d.changes.iter().any(|c| c.kind == ChangeKind::Added && c.oid == "IT.DM.SEX"));
    }

    #[test]
    fn removed_node_is_reported() {
        let a = r#"<ODM><ItemDef OID="IT.DM.AGE"/><ItemDef OID="IT.DM.SEX"/></ODM>"#;
        let b = r#"<ODM><ItemDef OID="IT.DM.AGE"/></ODM>"#;
        let d = tree_diff(a, b).unwrap();
        assert!(d.changes.iter().any(|c| c.kind == ChangeKind::Removed && c.oid == "IT.DM.SEX"));
    }

    #[test]
    fn identical_docs_no_changes() {
        let a = r#"<ODM><ItemDef OID="IT.DM.AGE" Name="AGE"/></ODM>"#;
        let b = r#"<ODM><ItemDef OID="IT.DM.AGE" Name="AGE"/></ODM>"#;
        let d = tree_diff(a, b).unwrap();
        assert!(d.changes.is_empty());
    }

    #[test]
    fn keyless_descendant_bubbles_up() {
        let a = r#"<ODM><CodeList OID="CL.X" Name="X"><Item CodedValue="A"/></CodeList></ODM>"#;
        let b = r#"<ODM><CodeList OID="CL.X" Name="X"><Item CodedValue="B"/></CodeList></ODM>"#;
        let d = tree_diff(a, b).unwrap();
        // The Item (keyless, no OID/Name) should bubble up to the CodeList.
        assert!(d.changes.iter().any(|c| c.elem_type == "CodeList" && c.kind == ChangeKind::Modified));
        assert!(!d.changes.iter().any(|c| c.elem_type == "Item"), "keyless Item should not appear directly");
    }

    #[test]
    fn reordered_nodes_match_by_key() {
        let a = r#"<ODM><ItemDef OID="A"/><ItemDef OID="B"/></ODM>"#;
        let b = r#"<ODM><ItemDef OID="B"/><ItemDef OID="A"/></ODM>"#;
        let d = tree_diff(a, b).unwrap();
        assert!(d.changes.is_empty(), "reordering by key should not report changes");
    }

    #[test]
    fn parse_error_returned() {
        let result = tree_diff("<not xml", "<ODM/>");
        assert!(result.is_err());
    }
}