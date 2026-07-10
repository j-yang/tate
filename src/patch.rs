//! Patch algebra over identity-keyed sections.
//!
//! A [`Patch`] is a map of [`Identity`] → [`NodeEdit`]. Each edit records the
//! old and new [`Node`]. The key operations are [`diff`], [`apply`],
//! [`invert`], [`compose`], and [`merge_sections`].
//!
//! Merge is the field-wise pushout: at each identity, if all three sides
//! exist but differ, each field (parent, kind, label, text, attrs, order)
//! is merged independently. This lets two branches that change different
//! fields of the same node — including Move (parent) + Modify (value) —
//! merge cleanly.

use std::collections::BTreeMap;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::section::{Identity, Node, Section};

/// A generic merge result.
pub struct MergeResult<T, C> {
    pub merged: T,
    pub conflicts: Vec<C>,
}

/// One obstruction to the pushout: an identity where `ours` and `theirs`
/// both diverged from `base` to incompatible values.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct SectionConflict {
    pub identity: Identity,
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub base: Option<Node>,
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub ours: Option<Node>,
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub theirs: Option<Node>,
}

/// One node edit: old and new state. `None` = absent (⊥).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct NodeEdit {
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub old: Option<Node>,
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub new: Option<Node>,
}

/// A lossless patch: identity → edit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Patch {
    pub edits: BTreeMap<Identity, NodeEdit>,
}

impl Patch {
    pub fn empty() -> Self { Patch { edits: BTreeMap::new() } }
    pub fn is_empty(&self) -> bool { self.edits.is_empty() }
}

/// Why [`apply`] failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyError {
    pub identity: Identity,
    pub expected: Option<Node>,
    pub found: Option<Node>,
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "patch does not apply at {:?}: expected {:?}, found {:?}", self.identity, self.expected, self.found)
    }
}
impl std::error::Error for ApplyError {}

// ─── diff / apply / invert / compose ───────────────────────────

pub fn diff_sections(a: &Section, b: &Section) -> Patch {
    let mut edits = BTreeMap::new();
    let mut ids: std::collections::BTreeSet<&Identity> = a.nodes.keys().collect();
    ids.extend(b.nodes.keys());
    for id in ids {
        let old = a.nodes.get(id);
        let new = b.nodes.get(id);
        if old != new {
            edits.insert(id.clone(), NodeEdit { old: old.cloned(), new: new.cloned() });
        }
    }
    Patch { edits }
}

pub fn apply_to_section(p: &Patch, a: &Section) -> Result<Section, Box<ApplyError>> {
    let mut nodes = a.nodes.clone();
    for (id, edit) in &p.edits {
        let current = nodes.get(id);
        if current.cloned() != edit.old {
            return Err(Box::new(ApplyError {
                identity: id.clone(),
                expected: edit.old.clone(),
                found: current.cloned(),
            }));
        }
        match &edit.new {
            Some(v) => { nodes.insert(id.clone(), v.clone()); }
            None => { nodes.remove(id); }
        }
    }
    Ok(Section { nodes })
}

pub fn diff(a: &crate::tree::TreeNode, b: &crate::tree::TreeNode) -> Patch {
    diff_sections(&a.to_section(), &b.to_section())
}

pub fn apply(p: &Patch, a: &crate::tree::TreeNode) -> Result<crate::tree::TreeNode, Box<ApplyError>> {
    let result = apply_to_section(p, &a.to_section())?;
    Ok(result.to_tree().unwrap_or_else(|| a.clone()))
}

pub fn invert(p: &Patch) -> Patch {
    let edits = p.edits.iter().map(|(id, e)| {
        (id.clone(), NodeEdit { old: e.new.clone(), new: e.old.clone() })
    }).collect();
    Patch { edits }
}

pub fn compose(p: &Patch, q: &Patch) -> Patch {
    let mut edits = BTreeMap::new();
    let mut ids: std::collections::BTreeSet<&Identity> = p.edits.keys().collect();
    ids.extend(q.edits.keys());
    for id in ids {
        let old = match p.edits.get(id) {
            Some(pe) => pe.old.clone(),
            None => q.edits.get(id).and_then(|qe| qe.old.clone()),
        };
        let new = match q.edits.get(id) {
            Some(qe) => qe.new.clone(),
            None => p.edits.get(id).and_then(|pe| pe.new.clone()),
        };
        if old != new {
            edits.insert(id.clone(), NodeEdit { old, new });
        }
    }
    Patch { edits }
}

// ─── field-wise merge helpers ──────────────────────────────────

fn merge_field<T: PartialEq + Clone>(o: &T, b: &T, t: &T) -> Option<T> {
    if o == b { Some(t.clone()) }
    else if t == b { Some(o.clone()) }
    else if o == t { Some(o.clone()) }
    else { None }
}

fn merge_node(base: &Node, ours: &Node, theirs: &Node) -> Option<Node> {
    let parent = merge_field(&ours.parent, &base.parent, &theirs.parent)?;
    let kind = merge_field(&ours.kind, &base.kind, &theirs.kind)?;
    let label = merge_field(&ours.label, &base.label, &theirs.label)?;
    let text = merge_field(&ours.text, &base.text, &theirs.text)?;
    let order = merge_field(&ours.order, &base.order, &theirs.order)?;
    let attrs = merge_attrs(&ours.attrs, &base.attrs, &theirs.attrs)?;
    Some(Node { parent, kind, label, text, attrs, order })
}

fn merge_attrs(
    ours: &[(String, String)],
    base: &[(String, String)],
    theirs: &[(String, String)],
) -> Option<Vec<(String, String)>> {
    let om: BTreeMap<&str, &str> = ours.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let bm: BTreeMap<&str, &str> = base.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let tm: BTreeMap<&str, &str> = theirs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();

    let mut keys: std::collections::BTreeSet<&str> = om.keys().copied().collect();
    keys.extend(bm.keys());
    keys.extend(tm.keys());

    let mut result = Vec::new();
    for key in keys {
        let merged = match (om.get(key).copied(), bm.get(key).copied(), tm.get(key).copied()) {
            (Some(o), Some(b), Some(t)) => {
                if o == b { Some(t) }
                else if t == b { Some(o) }
                else if o == t { Some(o) }
                else { return None }
            }
            (Some(o), Some(b), None) => { if o == b { None } else { return None } }
            (Some(o), None, Some(t)) => { if o == t { Some(o) } else { return None } }
            (None, Some(b), Some(t)) => { if b == t { None } else { return None } }
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

// ─── merge: the section pushout ────────────────────────────────

pub fn merge_sections(base: &Section, ours: &Section, theirs: &Section) -> MergeResult<Section, SectionConflict> {
    let mut merged = BTreeMap::new();
    let mut conflicts = Vec::new();

    let mut ids: std::collections::BTreeSet<&Identity> = base.nodes.keys().collect();
    ids.extend(ours.nodes.keys());
    ids.extend(theirs.nodes.keys());

    for id in ids {
        let b = base.nodes.get(id);
        let o = ours.nodes.get(id);
        let t = theirs.nodes.get(id);

        let chosen: Option<Node> = if o == b {
            t.cloned()
        } else if t == b {
            o.cloned()
        } else if o == t {
            o.cloned()
        } else if let (Some(bv), Some(ov), Some(tv)) = (b, o, t) {
            match merge_node(bv, ov, tv) {
                Some(n) => Some(n),
                None => {
                    conflicts.push(SectionConflict {
                        identity: id.clone(), base: b.cloned(), ours: o.cloned(), theirs: t.cloned(),
                    });
                    o.cloned()
                }
            }
        } else {
            conflicts.push(SectionConflict {
                identity: id.clone(), base: b.cloned(), ours: o.cloned(), theirs: t.cloned(),
            });
            o.cloned()
        };

        if let Some(n) = chosen {
            merged.insert(id.clone(), n);
        }
    }

    MergeResult { merged: Section { nodes: merged }, conflicts }
}

pub fn merge_sections_nway(base: &Section, branches: &[Section]) -> MergeResult<Section, SectionConflict> {
    match branches.len() {
        0 => return MergeResult { merged: base.clone(), conflicts: Vec::new() },
        1 => return MergeResult { merged: branches[0].clone(), conflicts: Vec::new() },
        _ => {}
    }

    let mut merged_map = BTreeMap::new();
    let mut conflicts = Vec::new();

    let mut ids: std::collections::BTreeSet<&Identity> = base.nodes.keys().collect();
    for b in branches {
        ids.extend(b.nodes.keys());
    }

    for id in ids {
        let b = base.nodes.get(id);
        let moved: std::collections::BTreeSet<Option<&Node>> = branches.iter()
            .map(|s| s.nodes.get(id)).filter(|v| v != &b).collect();

        if moved.is_empty() {
            if let Some(n) = b { merged_map.insert(id.clone(), n.clone()); }
        } else if moved.len() == 1 {
            if let Some(n) = *moved.iter().next().unwrap() {
                merged_map.insert(id.clone(), n.clone());
            }
        } else {
            let ours = branches.first().and_then(|s| s.nodes.get(id));
            let theirs = branches.iter().skip(1).find(|s| s.nodes.get(id) != ours).and_then(|s| s.nodes.get(id));
            conflicts.push(SectionConflict {
                identity: id.clone(), base: b.cloned(), ours: ours.cloned(), theirs: theirs.cloned(),
            });
            if let Some(n) = ours { merged_map.insert(id.clone(), n.clone()); }
        }
    }

    MergeResult { merged: Section { nodes: merged_map }, conflicts }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::TreeNode;

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
    fn diff_apply_roundtrips() {
        let a = sample();
        let mut b = sample();
        b.children[0].children[0].attributes[0].1 = "99".into();
        b.children[0].children.push(TreeNode::new("item").with_identity("i3").with_attr("v", "3"));
        b.children.pop();
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
        assert_eq!(apply(&invert(&p), &apply(&p, &a).unwrap()).unwrap(), a);
    }

    #[test]
    fn compose_equals_sequential() {
        let a = sample();
        let mut m = sample();
        m.children[0].children[0].attributes[0].1 = "50".into();
        let mut b = m.clone();
        b.children.push(TreeNode::new("group").with_identity("gX").with_text("new"));
        let p = diff(&a, &m);
        let q = diff(&m, &b);
        let pq = compose(&p, &q);
        assert_eq!(apply(&pq, &a).unwrap(), b);
    }

    #[test]
    fn compose_cancels() {
        let a = sample();
        let mut b = sample();
        b.children[0].attributes[0].1 = "changed".into();
        let p = diff(&a, &b);
        assert_eq!(compose(&p, &invert(&p)), Patch::empty());
    }

    #[test]
    fn merge_disjoint_clean() {
        let base = sample();
        let mut ours = sample();
        ours.children[0].children[0].attributes[0].1 = "9".into();
        let mut theirs = sample();
        theirs.children[0].children[1].attributes[0].1 = "8".into();
        let r = merge_sections(&base.to_section(), &ours.to_section(), &theirs.to_section());
        assert!(r.conflicts.is_empty());
    }

    #[test]
    fn merge_different_attrs_same_node_clean() {
        // Two branches change DIFFERENT attributes of the SAME node.
        let base = sample();
        let mut ours = sample();
        ours.children[0].attributes.push(("foo".to_string(), "1".into()));
        let mut theirs = sample();
        theirs.children[0].attributes.push(("bar".to_string(), "2".into()));
        let r = merge_sections(&base.to_section(), &ours.to_section(), &theirs.to_section());
        assert!(r.conflicts.is_empty(), "different attrs must merge cleanly");
    }

    #[test]
    fn merge_move_plus_modify_clean() {
        // KEY TEST: Alice moves a node, Bob modifies its value.
        // In the identity-keyed model, these touch different fields → clean.
        let base = sample();

        // Alice: move i1 from g1 to g2.
        let mut moved = base.clone();
        moved.children[1].children.push(TreeNode::new("item").with_identity("i1").with_attr("v", "1"));
        moved.children[0].children.retain(|c| c.identity.as_deref() != Some("i1"));

        // Bob: modify i1's value.
        let mut modified = base.clone();
        modified.children[0].children[0].attributes[0].1 = "99".into();

        let r = merge_sections(&base.to_section(), &moved.to_section(), &modified.to_section());

        // Move changed parent field, modify changed value field → clean merge.
        assert!(r.conflicts.is_empty(), "move + modify must merge cleanly");

        // Check: i1 should be under g2 (from move) with value 99 (from modify).
        let i1 = r.merged.nodes.get("i1").unwrap();
        assert_eq!(i1.parent.as_deref(), Some("g2"), "i1 moved to g2");
        assert_eq!(i1.attrs.iter().find(|(k, _)| k == "v").unwrap().1, "99", "i1 value modified");
    }

    #[test]
    fn merge_same_attr_conflict() {
        let base = sample();
        let mut ours = sample();
        ours.children[0].children[0].attributes[0].1 = "9".into();
        let mut theirs = sample();
        theirs.children[0].children[0].attributes[0].1 = "7".into();
        let r = merge_sections(&base.to_section(), &ours.to_section(), &theirs.to_section());
        assert_eq!(r.conflicts.len(), 1);
    }

    #[test]
    fn merge_identical_clean() {
        let base = sample();
        let mut side = sample();
        side.children[0].children[0].attributes[0].1 = "same".into();
        let r = merge_sections(&base.to_section(), &side.to_section(), &side.to_section());
        assert!(r.conflicts.is_empty());
    }

    #[test]
    fn apply_to_wrong_base_errors() {
        let a = sample();
        let mut b = sample();
        b.children[0].attributes[0].1 = "x".into();
        let p = diff(&a, &b);
        let mut wrong = sample();
        wrong.children[0].attributes[0].1 = "y".into();
        assert!(apply(&p, &wrong).is_err());
    }
}
