//! Identity-keyed section: the canonical form of a tree.
//!
//! In tate 2.0, a [`Section`] maps [`Identity`] → [`Node`], where each Node
//! stores its parent's identity (not its path from root). This separation of
//! identity from position enables:
//!
//! - **Move as a field-level change**: moving a node changes its `parent`
//!   field, not its key in the map.
//! - **Move + Modify merge cleanly**: moving (parent field) and modifying
//!   (value fields) touch different fields of the same node → commute.
//! - **Sheaf on the tree space**: a section is a global section of the sheaf
//!   on the ancestry Alexandrov topology; it satisfies referential integrity
//!   (present ⇒ parent present). Merge's sheafification stage
//!   ([`crate::patch::merge_sections`]) enforces this.

use std::collections::BTreeMap;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::tree::TreeNode;

/// A node's unique identifier. Used as the key in a [`Section`].
pub type Identity = String;

/// A node in the section: everything intrinsic to a tree node, including its
/// parent reference (position) and scalar content (value).
///
/// The `parent` field separates identity (the key) from position (where the
/// node sits in the tree). A move is a `parent` change; a modification is a
/// value change. They touch different fields → they commute.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Node {
    /// Parent's identity. `None` for the root.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub parent: Option<Identity>,
    /// Element type (XML tag name, JSON object key, etc.)
    pub kind: String,
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "String::is_empty"))]
    pub label: String,
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "String::is_empty"))]
    pub text: String,
    /// Key-value attributes, in original order.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Vec::is_empty"))]
    pub attrs: Vec<(String, String)>,
    /// Index among siblings.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "is_zero_usize"))]
    pub order: usize,
}

fn is_zero_usize(v: &usize) -> bool {
    *v == 0
}

/// A section: identity → node. The canonical, flat form of a tree.
///
/// This is the object on which diff/patch/merge are defined. Moving a node
/// changes its `parent` field at a stable identity key — not a delete+insert
/// at two different location keys.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Section {
    pub nodes: BTreeMap<Identity, Node>,
}

impl Section {
    pub fn new() -> Self {
        Section { nodes: BTreeMap::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Rebuild the nested [`TreeNode`] from this section.
    ///
    /// Returns `None` if the section is empty or has no root (a node with
    /// `parent = None`). Children are ordered by their `order` field.
    pub fn to_tree(&self) -> Option<TreeNode> {
        if self.nodes.is_empty() {
            return None;
        }
        let root_id = self.nodes.iter()
            .find(|(_, n)| n.parent.is_none())
            .map(|(id, _)| id.clone())?;
        self.build_node(&root_id)
    }

    fn build_node(&self, id: &str) -> Option<TreeNode> {
        let node = self.nodes.get(id)?;
        let mut tree = TreeNode::new(&node.kind);
        // Recover identity: if key != kind, it was an explicit identity.
        if id != node.kind {
            tree.identity = Some(id.to_string());
        }
        tree.label = node.label.clone();
        tree.text = node.text.clone();
        tree.attributes = node.attrs.clone();

        // Find children (nodes whose parent == this id), sorted by order.
        let mut children: Vec<(&Identity, &Node)> = self.nodes.iter()
            .filter(|(_, n)| n.parent.as_deref() == Some(id))
            .collect();
        children.sort_by_key(|(_, n)| n.order);

        for (cid, _) in children {
            if let Some(child) = self.build_node(cid) {
                tree.children.push(child);
            }
        }
        Some(tree)
    }
}

impl TreeNode {
    /// Flatten this tree into a [`Section`]: identity → node.
    ///
    /// Nodes with explicit identity use it as their key. Nodes without
    /// identity use their kind as the key (matching the old location-segment
    /// behavior). Keyless siblings of the same kind collide — that is the
    /// keying adapter's job to resolve, not tate core's.
    pub fn to_section(&self) -> Section {
        let mut nodes = BTreeMap::new();
        flatten_node(self, None, 0, &mut nodes);
        Section { nodes }
    }
}

fn flatten_node(
    node: &TreeNode,
    parent: Option<&str>,
    order: usize,
    nodes: &mut BTreeMap<Identity, Node>,
) {
    let id = node.identity.clone().unwrap_or_else(|| node.kind.clone());
    nodes.insert(id.clone(), Node {
        parent: parent.map(String::from),
        kind: node.kind.clone(),
        label: node.label.clone(),
        text: node.text.clone(),
        attrs: node.attributes.clone(),
        order,
    });
    for (i, child) in node.children.iter().enumerate() {
        flatten_node(child, Some(&id), i, nodes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> TreeNode {
        TreeNode::new("root")
            .with_child(
                TreeNode::new("group").with_identity("g1").with_attr("name", "vitals")
                    .with_child(TreeNode::new("item").with_identity("i1").with_attr("v", "1"))
                    .with_child(TreeNode::new("item").with_identity("i2").with_attr("v", "2")),
            )
            .with_child(TreeNode::new("group").with_identity("g2").with_text("empty"))
    }

    #[test]
    fn roundtrip() {
        let t = sample();
        let s = t.to_section();
        assert!(!s.is_empty());
        assert_eq!(s.to_tree(), Some(t));
    }

    #[test]
    fn identity_is_key() {
        let s = sample().to_section();
        assert!(s.nodes.contains_key("root"));
        assert!(s.nodes.contains_key("g1"));
        assert!(s.nodes.contains_key("i1"));
    }

    #[test]
    fn parent_field_set() {
        let s = sample().to_section();
        assert_eq!(s.nodes["root"].parent, None);
        assert_eq!(s.nodes["g1"].parent.as_deref(), Some("root"));
        assert_eq!(s.nodes["i1"].parent.as_deref(), Some("g1"));
    }

    #[test]
    fn order_field_set() {
        let s = sample().to_section();
        assert_eq!(s.nodes["g1"].order, 0);
        assert_eq!(s.nodes["g2"].order, 1);
    }

    #[test]
    fn move_is_parent_change() {
        // Build a tree, move a node, check that only the parent field changed.
        let t = sample();
        let s1 = t.to_section();

        // Move i1 from g1 to g2.
        let mut moved = t.clone();
        moved.children[1].children.push(TreeNode::new("item").with_identity("i1").with_attr("v", "1"));
        moved.children[0].children.retain(|c| c.identity.as_deref() != Some("i1"));
        let s2 = moved.to_section();

        // i1's identity is the same key in both sections.
        assert!(s1.nodes.contains_key("i1"));
        assert!(s2.nodes.contains_key("i1"));
        // Only the parent changed.
        assert_eq!(s1.nodes["i1"].parent.as_deref(), Some("g1"));
        assert_eq!(s2.nodes["i1"].parent.as_deref(), Some("g2"));
        // Other fields unchanged.
        assert_eq!(s1.nodes["i1"].attrs, s2.nodes["i1"].attrs);
    }

    #[test]
    fn empty_section_has_no_tree() {
        assert!(Section::new().to_tree().is_none());
    }
}
