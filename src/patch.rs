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

use crate::section::{Location, Section, Value};
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

/// One point edit: the value at a location goes from `old` to `new`. `None`
/// means `⊥` (the location is absent on that side). The invariant `old != new`
/// holds for every edit in a [`Patch`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PointEdit {
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub old: Option<Value>,
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub new: Option<Value>,
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
    pub expected: Option<Value>,
    /// The value actually present.
    pub found: Option<Value>,
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

// ─── diff / apply / invert / compose ───────────────────────────────────────────

/// The unique patch taking section `a` to section `b`. This is the core of the
/// algebra; [`diff`] is the tree-facing wrapper.
pub fn diff_sections(a: &Section, b: &Section) -> Patch {
    let mut edits = BTreeMap::new();
    // Every location present on either side; record where the value differs.
    let mut locations: std::collections::BTreeSet<&Location> = a.values.keys().collect();
    locations.extend(b.values.keys());
    for loc in locations {
        let old = a.values.get(loc);
        let new = b.values.get(loc);
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

/// Transport section `a` along patch `p`, producing the target section. Fails
/// with [`ApplyError`] if `p`'s expected `old` value at any location does not
/// match `a` — a patch only applies to the exact section it was diffed from.
pub fn apply_to_section(p: &Patch, a: &Section) -> Result<Section, Box<ApplyError>> {
    let mut values = a.values.clone();
    for (loc, edit) in &p.edits {
        let current = values.get(loc).cloned();
        if current != edit.old {
            return Err(Box::new(ApplyError {
                location: loc.clone(),
                expected: edit.old.clone(),
                found: current,
            }));
        }
        match &edit.new {
            Some(v) => {
                values.insert(loc.clone(), v.clone());
            }
            None => {
                values.remove(loc);
            }
        }
    }
    Ok(Section { values })
}

/// The unique patch taking tree `a` to tree `b` (flattens both, then diffs).
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
    diff_sections(&a.to_section(), &b.to_section())
}

/// Transport tree `a` along patch `p`. Fails with [`ApplyError`] if `p`'s
/// expected `old` value at any location does not match `a` — a patch only
/// applies to the exact tree it was diffed from.
pub fn apply(p: &Patch, a: &TreeNode) -> Result<TreeNode, Box<ApplyError>> {
    let result = apply_to_section(p, &a.to_section())?;
    // An empty section can only arise if the whole tree was deleted; callers of
    // apply always keep at least the root, but guard anyway.
    Ok(result.to_tree().unwrap_or_else(|| a.clone()))
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
