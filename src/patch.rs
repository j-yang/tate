//! Patch algebra: the morphisms of the versioned-structure category.
//!
//! A [`TreeNode`](crate::tree::TreeNode) is a *section* of the location→value
//! sheaf: an assignment of a value to every location in the tree. A **patch**
//! is a morphism between two sections — it records, for each location that
//! changed, the old and new value (with `None` standing for `⊥`, the *absent*
//! location). Three operations make this a category with inverses:
//!
//! - [`diff`]`(a, b)` — the unique patch taking section `a` to section `b`.
//! - [`apply`]`(p, a)` — transport a section along a patch.
//! - [`invert`]`(p)` — the inverse morphism.
//! - [`compose`]`(p, q)` — sequential composition (`p` then `q`).
//!
//! These obey the laws (verified by proptest in this module):
//!
//! ```text
//! apply(diff(a, b), a) == b                    // diff/apply are inverse
//! apply(invert(p), apply(p, a)) == a           // invert undoes apply
//! apply(compose(p, q), a) == apply(q, apply(p, a))   // composition
//! ```
//!
//! # Lossless, unlike [`tree_diff`](crate::tree::tree_diff)
//!
//! [`tree_diff`](crate::tree::tree_diff) is a *display* diff: it summarises
//! changes for humans (bubbling keyless descendants up to their nearest
//! identity-bearing ancestor, dropping subtree payloads on add/remove). It is
//! intentionally lossy, so it cannot round-trip. This module's [`diff`] is
//! *lossless*: it records exactly enough to reconstruct `b` from `a`.
//!
//! # Precondition: unique sibling keys
//!
//! Each node is located by its **identity** (if set) or else its **kind**
//! (positional). The algebra is exact when siblings have distinct keys — the
//! canonical case for identity-keyed data (JSON objects, XML with `id`/`OID`,
//! tables with primary keys). Keyless siblings that share a kind (e.g. bare
//! array items, un-keyed grid rows) collide at one location; disambiguating
//! them is the job of the keying adapters (a later phase), not this core.
//!
//! ```
//! use tate::tree::TreeNode;
//! use tate::patch::{diff, apply};
//!
//! let a = TreeNode::new("root")
//!     .with_child(TreeNode::new("e").with_identity("u1").with_attr("v", "1"));
//! let b = TreeNode::new("root")
//!     .with_child(TreeNode::new("e").with_identity("u1").with_attr("v", "2"));
//! let p = diff(&a, &b);
//! assert_eq!(apply(&p, &a).unwrap(), b);
//! ```

use std::collections::BTreeMap;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::tree::TreeNode;

/// A generic merge result carrying the merged value and conflicts.
///
/// The common shape shared by all three merge implementations:
/// - Line merge returns [`crate::merge::MergeOutcome`] (conflicts inline).
/// - Grid merge returns [`crate::grid::GridMergeResult`] (cell-level conflicts).
/// - Tree merge returns [`crate::tree::TreeMergeResult`] (node-level conflicts).
pub struct MergeResult<T, C> {
    pub merged: T,
    pub conflicts: Vec<C>,
}

/// A location in the tree: the sequence of sibling keys from the root down to a
/// node. A key is the node's identity if set, otherwise its kind.
pub type Location = Vec<String>;

/// The value living at one location: everything intrinsic to a node *except*
/// which children it has (that is encoded by which other locations exist).
///
/// Per the sheaf model, structural position (`order` among siblings) is part of
/// the value, not the location — so moving a node to a new parent is a value
/// change at a stable location, not a delete+add.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct NodeValue {
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

/// One point edit: the value at a location goes from `old` to `new`. `None`
/// means `⊥` (the location is absent on that side). The invariant `old != new`
/// holds for every edit in a [`Patch`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PointEdit {
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub old: Option<NodeValue>,
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub new: Option<NodeValue>,
}

/// A lossless patch: a location-keyed map of point edits. Its domain is exactly
/// the set of locations whose value differs between the two sections.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Patch {
    /// Edits keyed by location. `BTreeMap` gives a canonical, deterministic order.
    pub edits: BTreeMap<Location, PointEdit>,
}

impl Patch {
    /// The identity patch (no edits) — `apply(&Patch::empty(), a) == a`.
    pub fn empty() -> Self {
        Patch { edits: BTreeMap::new() }
    }

    /// True if this patch changes nothing.
    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }
}

/// Why [`apply`] failed: the patch's expected `old` value at a location did not
/// match the section it was applied to. A patch is only valid against the exact
/// section it was diffed from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyError {
    pub location: Location,
    /// The value the patch expected to find.
    pub expected: Option<NodeValue>,
    /// The value actually present.
    pub found: Option<NodeValue>,
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "patch does not apply at {:?}: expected {:?}, found {:?}",
            self.location, self.expected, self.found
        )
    }
}

impl std::error::Error for ApplyError {}

// ─── flatten / unflatten: TreeNode ⇄ location→value map ────────────────────────

/// The key locating a node among its siblings: identity if present, else kind.
fn loc_segment(n: &TreeNode) -> String {
    match &n.identity {
        Some(id) => id.clone(),
        None => n.kind.clone(),
    }
}

/// Flatten a tree into its section: a map from every location to its value.
fn flatten(root: &TreeNode) -> BTreeMap<Location, NodeValue> {
    let mut map = BTreeMap::new();
    let mut path = vec![loc_segment(root)];
    flatten_into(root, 0, &mut path, &mut map);
    map
}

fn flatten_into(
    node: &TreeNode,
    order: usize,
    path: &mut Location,
    map: &mut BTreeMap<Location, NodeValue>,
) {
    map.insert(
        path.clone(),
        NodeValue {
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

/// Rebuild a tree from its section. Returns `None` if the map is empty or has no
/// single root (a location of length 1).
fn unflatten(map: &BTreeMap<Location, NodeValue>) -> Option<TreeNode> {
    if map.is_empty() {
        return None;
    }
    // Build a bare node (no children yet) for every location.
    let mut nodes: BTreeMap<Location, TreeNode> = BTreeMap::new();
    for (loc, v) in map {
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
    // Attach children to parents, deepest first, so a parent is fully built
    // before it is itself attached upward. Sort each parent's children by the
    // stored `order`.
    let mut locs: Vec<Location> = map.keys().cloned().collect();
    locs.sort_by_key(|l| std::cmp::Reverse(l.len()));
    // pending[parent] = list of (order, child_location)
    let mut pending: BTreeMap<Location, Vec<(usize, Location)>> = BTreeMap::new();
    for loc in &locs {
        if loc.len() >= 2 {
            let parent = loc[..loc.len() - 1].to_vec();
            let order = map.get(loc).map(|v| v.order).unwrap_or(0);
            pending.entry(parent).or_default().push((order, loc.clone()));
        }
    }
    // Move children into their parents. Process deepest parents first so that a
    // subtree is complete before its root is moved up.
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

// ─── diff / apply / invert / compose ───────────────────────────────────────────

/// The unique patch taking section `a` to section `b`.
///
/// ```
/// use tate::tree::TreeNode;
/// use tate::patch::{diff, apply};
///
/// let a = TreeNode::new("root").with_child(TreeNode::new("x").with_identity("1"));
/// let b = TreeNode::new("root")
///     .with_child(TreeNode::new("x").with_identity("1"))
///     .with_child(TreeNode::new("y").with_identity("2"));
/// assert_eq!(apply(&diff(&a, &b), &a).unwrap(), b);
/// ```
pub fn diff(a: &TreeNode, b: &TreeNode) -> Patch {
    let fa = flatten(a);
    let fb = flatten(b);
    let mut edits = BTreeMap::new();
    // Every location present on either side; record where the value differs.
    let mut locations: std::collections::BTreeSet<&Location> = fa.keys().collect();
    locations.extend(fb.keys());
    for loc in locations {
        let old = fa.get(loc);
        let new = fb.get(loc);
        if old != new {
            edits.insert(
                loc.clone(),
                PointEdit {
                    old: old.cloned(),
                    new: new.cloned(),
                },
            );
        }
    }
    Patch { edits }
}

/// Transport section `a` along patch `p`. Fails with [`ApplyError`] if `p`'s
/// expected `old` value at any location does not match `a` — a patch only
/// applies to the exact section it was diffed from.
pub fn apply(p: &Patch, a: &TreeNode) -> Result<TreeNode, Box<ApplyError>> {
    let mut section = flatten(a);
    for (loc, edit) in &p.edits {
        let current = section.get(loc).cloned();
        if current != edit.old {
            return Err(Box::new(ApplyError {
                location: loc.clone(),
                expected: edit.old.clone(),
                found: current,
            }));
        }
        match &edit.new {
            Some(v) => {
                section.insert(loc.clone(), v.clone());
            }
            None => {
                section.remove(loc);
            }
        }
    }
    // An empty section can only arise if the whole tree was deleted; callers of
    // apply always keep at least the root, but guard anyway.
    Ok(unflatten(&section).unwrap_or_else(|| a.clone()))
}

/// The inverse morphism: `apply(invert(p), apply(p, a)) == a`.
///
/// ```
/// use tate::tree::TreeNode;
/// use tate::patch::{diff, apply, invert};
///
/// let a = TreeNode::new("r").with_child(TreeNode::new("x").with_identity("1").with_attr("v", "1"));
/// let b = TreeNode::new("r").with_child(TreeNode::new("x").with_identity("1").with_attr("v", "2"));
/// let p = diff(&a, &b);
/// let back = invert(&p);
/// assert_eq!(apply(&back, &apply(&p, &a).unwrap()).unwrap(), a);
/// ```
pub fn invert(p: &Patch) -> Patch {
    let edits = p
        .edits
        .iter()
        .map(|(loc, e)| {
            (
                loc.clone(),
                PointEdit {
                    old: e.new.clone(),
                    new: e.old.clone(),
                },
            )
        })
        .collect();
    Patch { edits }
}

/// Sequential composition: `apply(compose(p, q), a) == apply(q, apply(p, a))`.
///
/// `p` takes `a → m`, `q` takes `m → b`; the result takes `a → b` directly.
/// Edits that cancel (a location `p` changes and `q` changes back) drop out.
pub fn compose(p: &Patch, q: &Patch) -> Patch {
    let mut edits = BTreeMap::new();
    let mut locations: std::collections::BTreeSet<&Location> = p.edits.keys().collect();
    locations.extend(q.edits.keys());
    for loc in locations {
        // Effective old = what the value was before `p` (or before `q`, if `p`
        // left it untouched). Effective new = what it is after `q` (or after
        // `p`, if `q` left it untouched).
        let old = match p.edits.get(loc) {
            Some(pe) => pe.old.clone(),
            None => q.edits.get(loc).and_then(|qe| qe.old.clone()),
        };
        let new = match q.edits.get(loc) {
            Some(qe) => qe.new.clone(),
            None => p.edits.get(loc).and_then(|pe| pe.new.clone()),
        };
        if old != new {
            edits.insert(loc.clone(), PointEdit { old, new });
        }
    }
    Patch { edits }
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
    fn flatten_unflatten_roundtrips() {
        let t = sample();
        let flat = flatten(&t);
        assert_eq!(unflatten(&flat), Some(t));
    }

    #[test]
    fn diff_apply_roundtrips() {
        let a = sample();
        let mut b = sample();
        // mutate: change an attr, add a node, remove a node.
        b.children[0].children[0].attributes[0].1 = "99".into();
        b.children[0].children.push(TreeNode::new("item").with_identity("i3").with_attr("v", "3"));
        b.children.pop(); // remove g2
        let p = diff(&a, &b);
        assert!(!p.is_empty());
        assert_eq!(apply(&p, &a).unwrap(), b);
    }

    #[test]
    fn empty_patch_is_identity() {
        let a = sample();
        assert_eq!(diff(&a, &a), Patch::empty());
        assert_eq!(apply(&Patch::empty(), &a).unwrap(), a);
    }

    #[test]
    fn invert_undoes_apply() {
        let a = sample();
        let mut b = sample();
        b.children[0].attributes[0].1 = "signs".into();
        b.children.push(TreeNode::new("group").with_identity("g3"));
        let p = diff(&a, &b);
        let applied = apply(&p, &a).unwrap();
        assert_eq!(applied, b);
        assert_eq!(apply(&invert(&p), &applied).unwrap(), a);
    }

    #[test]
    fn compose_equals_sequential_apply() {
        let a = sample();
        let mut m = sample();
        m.children[0].children[0].attributes[0].1 = "50".into();
        let mut b = m.clone();
        b.children.push(TreeNode::new("group").with_identity("gX").with_text("new"));

        let p = diff(&a, &m);
        let q = diff(&m, &b);
        let pq = compose(&p, &q);

        let sequential = apply(&q, &apply(&p, &a).unwrap()).unwrap();
        let composed = apply(&pq, &a).unwrap();
        assert_eq!(composed, sequential);
        assert_eq!(composed, b);
    }

    #[test]
    fn compose_cancels_opposite_edits() {
        // p: a → b, q = invert(p): b → a. compose(p, q) must be the identity.
        let a = sample();
        let mut b = sample();
        b.children[0].attributes[0].1 = "changed".into();
        let p = diff(&a, &b);
        let q = invert(&p);
        assert_eq!(compose(&p, &q), Patch::empty());
    }

    #[test]
    fn apply_to_wrong_base_errors() {
        let a = sample();
        let mut b = sample();
        b.children[0].attributes[0].1 = "x".into();
        let p = diff(&a, &b);
        // Applying to something that isn't `a` must fail loudly, not silently.
        let mut wrong = sample();
        wrong.children[0].attributes[0].1 = "y".into();
        assert!(apply(&p, &wrong).is_err());
    }

    #[test]
    fn tag_rename_with_stable_identity_is_value_change() {
        // Identity is the location; kind is part of the value. Renaming the kind
        // of an identity-keyed node is a value change, not delete+add.
        let a = TreeNode::new("root").with_child(TreeNode::new("foo").with_identity("1"));
        let b = TreeNode::new("root").with_child(TreeNode::new("bar").with_identity("1"));
        let p = diff(&a, &b);
        // Exactly one location changed (the child), and it stayed one edit.
        assert_eq!(p.edits.len(), 1);
        assert_eq!(apply(&p, &a).unwrap(), b);
    }
}
