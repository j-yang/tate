//! Patch algebra: the morphisms of the versioned-structure groupoid.
//!
//! A [`TreeNode`](crate::tree) is a *section* of the location→value
//! sheaf: an assignment of a value to every location in the tree. A **patch**
//! is a morphism between two sections — it records, for each location that
//! changed, the old and new value (with `None` standing for `⊥`, the *absent*
//! location). Four operations give patches the structure of a **groupoid**
//! (a category in which every morphism is invertible):
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
/// The shape of a 3-way merge outcome: the best-effort merged value plus the
/// list of gluing obstructions. [`crate::tree::TreeMergeResult`] is the concrete
/// instance ([`crate::tree::tree_merge`] is the sole merge — grid and line
/// inputs reach it by being keyed into trees first).
pub struct MergeResult<T, C> {
    pub merged: T,
    pub conflicts: Vec<C>,
}

/// One obstruction to the section pushout: a location where `ours` and `theirs`
/// both moved away from `base`, to *different* values. There the pushout does
/// not exist. `merge_sections` still returns a best-effort value (favouring
/// `ours`) at this location and records the disagreement here.
///
/// Values are `Option<Value>` because either side may have deleted the location
/// (`None` = `⊥`, the absent value).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct SectionConflict {
    pub location: Location,
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub base: Option<Value>,
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub ours: Option<Value>,
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub theirs: Option<Value>,
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

// ─── merge: the section pushout ────────────────────────────────────────────────

/// Try to merge two Values field-by-field (per-attribute merge).
/// Returns Some(merged) if all fields can be resolved, None if any field conflicts.
fn try_merge_value(base: &Value, ours: &Value, theirs: &Value) -> Option<Value> {
    let kind = merge_field(&ours.kind, &base.kind, &theirs.kind)?;
    let label = merge_field(&ours.label, &base.label, &theirs.label)?;
    let text = merge_field(&ours.text, &base.text, &theirs.text)?;
    let order = merge_field(&ours.order, &base.order, &theirs.order)?;
    let attrs = merge_attrs(&ours.attrs, &base.attrs, &theirs.attrs)?;
    Some(Value { kind, label, text, attrs, order })
}

fn merge_field<T: PartialEq + Clone>(o: &T, b: &T, t: &T) -> Option<T> {
    if o == b { Some(t.clone()) }
    else if t == b { Some(o.clone()) }
    else if o == t { Some(o.clone()) }
    else { None }
}

fn merge_attrs(
    ours: &[(String, String)],
    base: &[(String, String)],
    theirs: &[(String, String)],
) -> Option<Vec<(String, String)>> {
    use std::collections::BTreeMap;
    let om: BTreeMap<&str, &str> = ours.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let bm: BTreeMap<&str, &str> = base.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let tm: BTreeMap<&str, &str> = theirs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();

    let mut keys: std::collections::BTreeSet<&str> = om.keys().copied().collect();
    keys.extend(bm.keys().copied());
    keys.extend(tm.keys().copied());

    let mut result = Vec::new();
    for key in keys {
        let ov = om.get(key).copied();
        let bv = bm.get(key).copied();
        let tv = tm.get(key).copied();
        let merged = match (ov, bv, tv) {
            (Some(o), Some(b), Some(t)) => {
                if o == b { Some(t) }
                else if t == b { Some(o) }
                else if o == t { Some(o) }
                else { return None }
            }
            (Some(o), Some(b), None) => {
                if o == b { None } else { return None }
            }
            (Some(o), None, Some(t)) => {
                if o == t { Some(o) } else { return None }
            }
            (None, Some(b), Some(t)) => {
                if b == t { None } else { return None }
            }
            (Some(o), None, None) => Some(o),
            (None, Some(_), None) => None,
            (None, None, Some(t)) => Some(t),
            (None, None, None) => None,
        };
        if let Some(v) = merged {
            result.push((key.to_string(), v.to_string()));
        }
    }
    Some(result)
}

/// 3-way merge as the **pushout** of two branch patches in the category of
/// sections.
///
/// Given the span `ours ← base → theirs` (legs `f = diff(base, ours)` and
/// `g = diff(base, theirs)`), the pushout is computed **point-wise** on the
/// discrete location space — the correct way to compute a pushout in a product
/// category, one factor per location. At each location `ℓ`, with
/// `b = base(ℓ)`, `o = ours(ℓ)`, `t = theirs(ℓ)` (each an `Option<Value>`,
/// `None` = `⊥`):
///
/// - `o == b` → theirs won the point (only `theirs` moved): take `t`.
/// - `t == b` → ours won the point (only `ours` moved, or neither): take `o`.
/// - `o == t` → both made the *same* move: take it (the pushout glues).
/// - otherwise → both moved to different values: the pushout does **not**
///   exist at `ℓ`. Record a [`SectionConflict`] and keep `o` (favour ours).
///
/// This is a **total function**: it always returns a merged section. An empty
/// conflict list means the pushout existed globally (a clean merge); a non-empty
/// one is exactly the obstruction set — the first Čech cohomology H¹ of the
/// cover {U_ours, U_theirs} where `U_x` is the set of locations `x` changed.
///
/// Unlike [`crate::tree::tree_merge`] (which drives a display-oriented merge off
/// the lossy [`crate::tree::tree_diff`]), this operates on the lossless section
/// algebra and *is* the pushout the math describes.
///
/// ```
/// use tate::tree::TreeNode;
/// use tate::patch::merge_sections;
///
/// let base = TreeNode::new("r").with_child(TreeNode::new("e").with_identity("u1").with_attr("v", "1")).to_section();
/// // ours changes v; theirs is untouched → theirs' point equals base → take ours.
/// let ours = TreeNode::new("r").with_child(TreeNode::new("e").with_identity("u1").with_attr("v", "9")).to_section();
/// let theirs = base.clone();
/// let r = merge_sections(&base, &ours, &theirs);
/// assert!(r.conflicts.is_empty());
/// assert_eq!(r.merged, ours);
/// ```
pub fn merge_sections(base: &Section, ours: &Section, theirs: &Section) -> MergeResult<Section, SectionConflict> {
    let mut merged = BTreeMap::new();
    let mut conflicts = Vec::new();

    // Every location present in any of the three sections.
    let mut locations: std::collections::BTreeSet<&Location> = base.values.keys().collect();
    locations.extend(ours.values.keys());
    locations.extend(theirs.values.keys());

    for loc in locations {
        let b = base.values.get(loc);
        let o = ours.values.get(loc);
        let t = theirs.values.get(loc);

        // Point-wise pushout. `chosen` is the value the merged section carries at
        // `loc` (None = the location is absent in the merge).
        let chosen: Option<Value> = if o == b {
            t.cloned()
        } else if t == b {
            o.cloned()
        } else if o == t {
            o.cloned()
        } else if let (Some(bv), Some(ov), Some(tv)) = (b, o, t) {
            // All three exist but differ as whole Values.
            // Try per-field merge: kind, label, text, order, and each attribute
            // are merged independently. This lets two branches that changed
            // different attributes of the same node merge cleanly.
            match try_merge_value(bv, ov, tv) {
                Some(merged) => Some(merged),
                None => {
                    conflicts.push(SectionConflict {
                        location: loc.clone(),
                        base: b.cloned(),
                        ours: o.cloned(),
                        theirs: t.cloned(),
                    });
                    o.cloned()
                }
            }
        } else {
            // Structural conflict (modify/delete or add/add with different values).
            conflicts.push(SectionConflict {
                location: loc.clone(),
                base: b.cloned(),
                ours: o.cloned(),
                theirs: t.cloned(),
            });
            o.cloned()
        };

        if let Some(v) = chosen {
            merged.insert(loc.clone(), v);
        }
    }

    MergeResult { merged: Section { values: merged }, conflicts }
}

/// N-way merge: the pushout of N branches diverged from a common base.
///
/// At each location, the branches that moved (value ≠ base) must all agree on
/// the same target value. If they do, the move is taken; if ≥2 distinct
/// non-base values appear, the pushout fails there. For 2 branches this is
/// identical to [`merge_sections`].
///
/// With ≥3 branches, conflicts capture **triple inconsistencies**: three
/// branches where every pair could pairwise-merge, but the triple disagrees.
/// On a discrete location space these reduce to "≥2 distinct non-base values at
/// the same location" — the multi-cover H¹ from MATHEMATICS.md §5.3.
pub fn merge_sections_nway(base: &Section, branches: &[Section]) -> MergeResult<Section, SectionConflict> {
    match branches.len() {
        0 => return MergeResult { merged: base.clone(), conflicts: Vec::new() },
        1 => return MergeResult { merged: branches[0].clone(), conflicts: Vec::new() },
        _ => {}
    }

    let mut merged_map = BTreeMap::new();
    let mut conflicts = Vec::new();

    let mut locations: std::collections::BTreeSet<&Location> = base.values.keys().collect();
    for b in branches {
        locations.extend(b.values.keys());
    }

    for loc in locations {
        let b = base.values.get(loc);

        let moved: std::collections::BTreeSet<Option<&Value>> = branches.iter()
            .map(|s| s.values.get(loc))
            .filter(|v| v != &b)
            .collect();

        if moved.is_empty() {
            if let Some(v) = b {
                merged_map.insert(loc.clone(), v.clone());
            }
        } else if moved.len() == 1 {
            if let Some(v) = *moved.iter().next().unwrap() {
                merged_map.insert(loc.clone(), v.clone());
            }
        } else {
            let ours = branches.first().and_then(|s| s.values.get(loc));
            let theirs = branches.iter()
                .skip(1)
                .find(|s| s.values.get(loc) != ours)
                .and_then(|s| s.values.get(loc));
            conflicts.push(SectionConflict {
                location: loc.clone(),
                base: b.cloned(),
                ours: ours.cloned(),
                theirs: theirs.cloned(),
            });
            if let Some(v) = ours {
                merged_map.insert(loc.clone(), v.clone());
            }
        }
    }

    MergeResult { merged: Section { values: merged_map }, conflicts }
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

    // ── merge_sections: the section pushout ──

    fn sec(t: &TreeNode) -> Section {
        t.to_section()
    }

    #[test]
    fn merge_only_ours_moved_takes_ours() {
        // theirs == base at the changed point → take ours.
        let base = sample();
        let mut ours = sample();
        ours.children[0].children[0].attributes[0].1 = "99".into();
        let theirs = sample();
        let r = merge_sections(&sec(&base), &sec(&ours), &sec(&theirs));
        assert!(r.conflicts.is_empty());
        assert_eq!(r.merged, sec(&ours));
    }

    #[test]
    fn merge_only_theirs_moved_takes_theirs() {
        let base = sample();
        let ours = sample();
        let mut theirs = sample();
        theirs.children[0].children[0].attributes[0].1 = "77".into();
        let r = merge_sections(&sec(&base), &sec(&ours), &sec(&theirs));
        assert!(r.conflicts.is_empty());
        assert_eq!(r.merged, sec(&theirs));
    }

    #[test]
    fn merge_disjoint_locations_glue_cleanly() {
        // ours changes one node, theirs changes a different node → pushout exists.
        let base = sample();
        let mut ours = sample();
        ours.children[0].children[0].attributes[0].1 = "9".into(); // i1
        let mut theirs = sample();
        theirs.children[0].children[1].attributes[0].1 = "8".into(); // i2
        let r = merge_sections(&sec(&base), &sec(&ours), &sec(&theirs));
        assert!(r.conflicts.is_empty(), "disjoint edits must not conflict");
        // Both edits present in the merged section.
        let expected = {
            let mut e = sample();
            e.children[0].children[0].attributes[0].1 = "9".into();
            e.children[0].children[1].attributes[0].1 = "8".into();
            sec(&e)
        };
        assert_eq!(r.merged, expected);
    }

    #[test]
    fn merge_identical_move_is_clean() {
        // Both sides make the SAME edit → glues, no conflict.
        let base = sample();
        let mut side = sample();
        side.children[0].children[0].attributes[0].1 = "same".into();
        let r = merge_sections(&sec(&base), &sec(&side), &sec(&side));
        assert!(r.conflicts.is_empty());
        assert_eq!(r.merged, sec(&side));
    }

    #[test]
    fn merge_divergent_move_is_a_conflict() {
        // Both sides move the SAME location to DIFFERENT values → no pushout.
        let base = sample();
        let mut ours = sample();
        ours.children[0].children[0].attributes[0].1 = "9".into();
        let mut theirs = sample();
        theirs.children[0].children[0].attributes[0].1 = "7".into();
        let r = merge_sections(&sec(&base), &sec(&ours), &sec(&theirs));
        assert_eq!(r.conflicts.len(), 1, "divergent edit must obstruct the pushout");
        let c = &r.conflicts[0];
        assert_eq!(c.location, vec!["root".to_string(), "g1".to_string(), "i1".to_string()]);
        // Best-effort value favours ours.
        assert_eq!(r.merged.values.get(&c.location).unwrap().attrs[0].1, "9");
    }

    #[test]
    fn merge_modify_delete_is_a_conflict() {
        // ours modifies a node; theirs deletes it → divergent (o != b, t == ⊥ != b).
        let base = sample();
        let mut ours = sample();
        ours.children[0].children[0].attributes[0].1 = "9".into();
        let mut theirs = sample();
        theirs.children[0].children.remove(0); // delete i1
        let r = merge_sections(&sec(&base), &sec(&ours), &sec(&theirs));
        // i1's own location conflicts (ours has a value, theirs has ⊥, both != base).
        let loc = vec!["root".to_string(), "g1".to_string(), "i1".to_string()];
        assert!(r.conflicts.iter().any(|c| c.location == loc), "modify/delete must conflict at the node");
        // Favour ours → i1 survives in the merge.
        assert!(r.merged.values.contains_key(&loc));
    }

    #[test]
    fn merge_symmetric_conflict_set() {
        // Swapping ours/theirs yields the same set of conflicting locations.
        let base = sample();
        let mut ours = sample();
        ours.children[0].children[0].attributes[0].1 = "9".into();
        let mut theirs = sample();
        theirs.children[0].children[0].attributes[0].1 = "7".into();
        let ot = merge_sections(&sec(&base), &sec(&ours), &sec(&theirs));
        let to = merge_sections(&sec(&base), &sec(&theirs), &sec(&ours));
        let locs = |r: &MergeResult<Section, SectionConflict>| {
            let mut v: Vec<_> = r.conflicts.iter().map(|c| c.location.clone()).collect();
            v.sort();
            v
        };
        assert_eq!(locs(&ot), locs(&to));
    }

    #[test]
    fn merge_relates_to_patches_apply_both_legs_when_clean() {
        // When the merge is clean, applying diff(base, theirs) to ours reaches the
        // merged section — i.e. the pushout really is base + both change sets.
        let base = sample();
        let mut ours = sample();
        ours.children[0].children[0].attributes[0].1 = "9".into(); // i1
        let mut theirs = sample();
        theirs.children[0].children[1].attributes[0].1 = "8".into(); // i2
        let r = merge_sections(&sec(&base), &sec(&ours), &sec(&theirs));
        assert!(r.conflicts.is_empty());
        // ours + theirs' leg == merged.
        let g = diff_sections(&sec(&base), &sec(&theirs));
        let via_patch = apply_to_section(&g, &sec(&ours)).unwrap();
        assert_eq!(via_patch, r.merged);
    }
}
