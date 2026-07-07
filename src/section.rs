//! The canonical object: a *section* of the location→value sheaf.
//!
//! tate has two views of the same data:
//!
//! - [`TreeNode`](crate::tree) — the **nested** view. Parsers produce
//!   it, UIs consume it, humans read it.
//! - [`Section`] — the **flat** view: a map from every [`Location`] to the
//!   [`Value`] living there. This is the object the diff/patch/merge algebra is
//!   defined on, because on the flat view an edit is a point change and the laws
//!   (`apply(diff(a,b),a)==b`, invertibility, composition) are clean.
//!
//! The two are interconvertible: [`TreeNode::to_section`] flattens,
//! [`Section::to_tree`] rebuilds. Round-tripping is the identity on trees whose
//! siblings have distinct keys (see below).
//!
//! # Location and Value — the sheaf split
//!
//! A [`Location`] is the path of sibling **keys** from the root to a node. A key
//! is the node's identity if it has one, otherwise its kind. Identity-as-key is
//! the load-bearing choice: a node keeps its location when its content changes,
//! so a moved or renamed node is a *value* change at a stable location, not a
//! delete+add.
//!
//! A [`Value`] is everything intrinsic to a node *except* which children it has:
//! kind, label, text, attributes, and `order` (its index among siblings). Which
//! children a node has is encoded structurally — by which *other* locations
//! exist in the section — so it is not stored in the value. Per the sheaf model,
//! structural position (`order`) is part of the value, not the location.
//!
//! `⊥` (the absent value) is represented by a location simply not being present
//! in the map. A [`crate::patch::Patch`] uses `Option<Value>` to talk about the
//! absent state explicitly (`None` = ⊥).
//!
//! # Precondition: unique sibling keys
//!
//! The flat view is faithful when siblings have distinct keys — the canonical
//! case for identity-keyed data (JSON objects, XML with `id`/`OID`, tables with
//! primary keys). Keyless siblings that share a kind (bare array items, un-keyed
//! grid rows) collide at one location; giving them stable keys is the job of the
//! keying adapters, not this core.

use std::collections::BTreeMap;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::tree::TreeNode;

/// A location in the tree: the sequence of sibling keys from the root down to a
/// node. A key is the node's identity if set, otherwise its kind.
pub type Location = Vec<String>;

/// The value living at one location: everything intrinsic to a node *except*
/// which children it has (that is encoded by which other locations exist).
///
/// Per the sheaf model, structural position (`order` among siblings) is part of
/// the value, not the location — so moving a node to a new parent is a value
/// change at a stable location, not a delete+add.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Value {
    pub kind: String,
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "String::is_empty"))]
    pub label: String,
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "String::is_empty"))]
    pub text: String,
    /// Attributes kept in their original order — reordering is a value change.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Vec::is_empty"))]
    pub attrs: Vec<(String, String)>,
    /// Index among the parent's children (structural position as value).
    pub order: usize,
}

/// A section of the location→value sheaf: the flat, canonical form of a tree.
///
/// This is *the object* of tate's algebra. [`crate::patch::diff`] takes two
/// sections to a patch; [`crate::patch::apply`] transports a section along one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Section {
    /// Location → value. `BTreeMap` gives a canonical, deterministic order.
    /// Serialized as a sequence of `[location, value]` pairs so it round-trips
    /// through JSON (whose object keys must be strings; a location is a list).
    #[cfg_attr(feature = "serde", serde(with = "crate::loc_map_serde"))]
    pub values: BTreeMap<Location, Value>,
}

/// The key locating a node among its siblings: identity if present, else kind.
pub fn loc_segment(n: &TreeNode) -> String {
    match &n.identity {
        Some(id) => id.clone(),
        None => n.kind.clone(),
    }
}

impl Section {
    /// An empty section (no locations) — the flat form of `⊥` everywhere.
    pub fn new() -> Self {
        Section { values: BTreeMap::new() }
    }

    /// True if this section has no locations.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Rebuild the nested [`TreeNode`] from this section.
    ///
    /// Returns `None` if the section is empty or has no single root (a location
    /// of length 1). Children are ordered by their stored [`Value::order`].
    pub fn to_tree(&self) -> Option<TreeNode> {
        if self.values.is_empty() {
            return None;
        }
        // Build a bare node (no children yet) for every location.
        let mut nodes: BTreeMap<Location, TreeNode> = BTreeMap::new();
        for (loc, v) in &self.values {
            nodes.insert(
                loc.clone(),
                TreeNode {
                    kind: v.kind.clone(),
                    identity: loc.last().and_then(|seg| {
                        // Recover identity: a node is identity-keyed iff its key
                        // differs from its kind (kind-keyed nodes are positional).
                        if seg != &v.kind { Some(seg.clone()) } else { None }
                    }),
                    label: v.label.clone(),
                    attributes: v.attrs.clone(),
                    text: v.text.clone(),
                    children: Vec::new(),
                },
            );
        }
        // Group children under their parent location.
        let mut pending: BTreeMap<Location, Vec<(usize, Location)>> = BTreeMap::new();
        for loc in self.values.keys() {
            if loc.len() >= 2 {
                let parent = loc[..loc.len() - 1].to_vec();
                let order = self.values.get(loc).map(|v| v.order).unwrap_or(0);
                pending.entry(parent).or_default().push((order, loc.clone()));
            }
        }
        // Attach children to parents, deepest parents first, so a subtree is
        // complete before its root is itself moved upward. Sort by stored order.
        let mut parent_locs: Vec<Location> = pending.keys().cloned().collect();
        parent_locs.sort_by_key(|l| std::cmp::Reverse(l.len()));
        for parent in parent_locs {
            let mut kids = pending.remove(&parent).unwrap_or_default();
            kids.sort_by_key(|(order, _)| *order);
            for (_, child_loc) in kids {
                if let Some(child) = nodes.remove(&child_loc) {
                    if let Some(p) = nodes.get_mut(&parent) {
                        p.children.push(child);
                    }
                }
            }
        }
        // The sole remaining length-1 location is the root.
        let root_loc = nodes.keys().find(|l| l.len() == 1).cloned()?;
        nodes.remove(&root_loc)
    }
}

impl TreeNode {
    /// Flatten this tree into its [`Section`]: a map from every location to the
    /// value living there. The inverse of [`Section::to_tree`].
    pub fn to_section(&self) -> Section {
        let mut values = BTreeMap::new();
        let mut path = vec![loc_segment(self)];
        flatten_into(self, 0, &mut path, &mut values);
        Section { values }
    }
}

fn flatten_into(
    node: &TreeNode,
    order: usize,
    path: &mut Location,
    map: &mut BTreeMap<Location, Value>,
) {
    map.insert(
        path.clone(),
        Value {
            kind: node.kind.clone(),
            label: node.label.clone(),
            text: node.text.clone(),
            attrs: node.attributes.clone(),
            order,
        },
    );
    for (i, child) in node.children.iter().enumerate() {
        path.push(loc_segment(child));
        flatten_into(child, i, path, map);
        path.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::TreeNode;

    fn sample() -> TreeNode {
        TreeNode::new("root")
            .with_child(
                TreeNode::new("group")
                    .with_identity("g1")
                    .with_attr("name", "vitals")
                    .with_child(TreeNode::new("item").with_identity("i1").with_attr("v", "1"))
                    .with_child(TreeNode::new("item").with_identity("i2").with_attr("v", "2")),
            )
            .with_child(TreeNode::new("group").with_identity("g2").with_text("empty"))
    }

    #[test]
    fn to_section_to_tree_roundtrips() {
        let t = sample();
        let s = t.to_section();
        assert!(!s.is_empty());
        assert_eq!(s.to_tree(), Some(t));
    }

    #[test]
    fn section_records_order_as_value() {
        let s = sample().to_section();
        // g1 is child 0, g2 is child 1 of root.
        let g1 = s.values.get(&vec!["root".to_string(), "g1".to_string()]).unwrap();
        let g2 = s.values.get(&vec!["root".to_string(), "g2".to_string()]).unwrap();
        assert_eq!(g1.order, 0);
        assert_eq!(g2.order, 1);
    }

    #[test]
    fn empty_section_has_no_tree() {
        assert_eq!(Section::new().to_tree(), None);
    }

    #[test]
    fn keyless_node_keyed_by_kind() {
        // A node without identity is located by its kind.
        let t = TreeNode::new("root").with_child(TreeNode::new("leaf").with_text("x"));
        let s = t.to_section();
        assert!(s.values.contains_key(&vec!["root".to_string(), "leaf".to_string()]));
        // Round-trips: kind-keyed node recovers to identity = None.
        let back = s.to_tree().unwrap();
        assert_eq!(back.children[0].identity, None);
        assert_eq!(back, t);
    }
}
