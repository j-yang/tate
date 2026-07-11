//! Property-based verification of the patch-algebra and merge laws.
//!
//! Laws checked:
//! 1. apply(diff(a, b), a) == b
//! 2. apply(diff(a, a), a) == a and diff(a, a) is empty
//! 3. apply(invert(p), apply(p, a)) == a
//! 4. apply(compose(p, q), a) == apply(q, apply(p, a))
//! 5. compose(p, invert(p)) is the empty patch
//! 6. compose(p, empty) == p == compose(empty, p)
//! 7. tree_merge conflict set is symmetric under swapping ours/theirs
//! 8. merge_sections(base, x, x) is conflict-free and returns x
//! 9. N-way pushout: merge_sections_nway is point-wise correct

use proptest::prelude::*;
use tate::patch::{
    apply, compose, diff, invert, merge_sections, merge_sections_nway,
    Patch, SectionConflictKind,
};
use tate::tree::{tree_merge, TreeNode};

fn attrs_strategy() -> impl Strategy<Value = Vec<(String, String)>> {
    prop::collection::vec(
        (
            prop::sample::select(vec!["a", "b", "c"]).prop_map(String::from),
            prop::sample::select(vec!["0", "1", "2", "x"]).prop_map(String::from),
        ),
        0..3,
    )
    .prop_map(|mut v| {
        v.sort_by(|x, y| x.0.cmp(&y.0));
        v.dedup_by(|x, y| x.0 == y.0);
        v
    })
}

fn tree_strategy() -> impl Strategy<Value = TreeNode> {
    let leaf = (
        prop::sample::select(vec!["item", "field", "node"]).prop_map(String::from),
        attrs_strategy(),
        prop::sample::select(vec!["", "t1", "t2"]).prop_map(String::from),
    )
        .prop_map(|(kind, attrs, text)| {
            let mut n = TreeNode::new(kind);
            for (k, v) in attrs { n = n.with_attr(k, v); }
            if !text.is_empty() { n = n.with_text(text); }
            n
        });

    leaf.prop_recursive(4, 32, 4, |inner| {
        (
            prop::sample::select(vec!["group", "section", "list"]).prop_map(String::from),
            attrs_strategy(),
            prop::collection::vec(inner, 0..4),
        )
            .prop_map(|(kind, attrs, children)| {
                let mut n = TreeNode::new(kind);
                for (k, v) in attrs { n = n.with_attr(k, v); }
                for c in children { n = n.with_child(c); }
                n
            })
    })
    .prop_map(assign_identities)
}

fn assign_identities(mut root: TreeNode) -> TreeNode {
    let mut counter = 0usize;
    assign_rec(&mut root, &mut counter);
    root
}

fn assign_rec(node: &mut TreeNode, counter: &mut usize) {
    node.identity = Some(format!("n{}", *counter));
    *counter += 1;
    for c in &mut node.children {
        assign_rec(c, counter);
    }
}

fn tree_pair() -> impl Strategy<Value = (TreeNode, TreeNode)> {
    (tree_strategy(), tree_strategy()).prop_map(|(mut a, mut b)| {
        a.identity = Some("root".into());
        b.identity = Some("root".into());
        (a, b)
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn law_diff_apply_roundtrip((a, b) in tree_pair()) {
        let p = diff(&a, &b);
        prop_assert_eq!(apply(&p, &a).unwrap(), b);
    }

    #[test]
    fn law_identity(a in tree_strategy()) {
        let p = diff(&a, &a);
        prop_assert!(p.is_empty());
        prop_assert_eq!(apply(&Patch::empty(), &a).unwrap(), a);
    }

    #[test]
    fn law_invert_undoes((a, b) in tree_pair()) {
        let p = diff(&a, &b);
        let forward = apply(&p, &a).unwrap();
        prop_assert_eq!(apply(&invert(&p), &forward).unwrap(), a);
    }

    #[test]
    fn law_compose_is_sequential((a, m) in tree_pair(), c in tree_strategy()) {
        let mut b = c;
        b.identity = Some("root".into());
        let p = diff(&a, &m);
        let q = diff(&m, &b);
        let pq = compose(&p, &q);
        let sequential = apply(&q, &apply(&p, &a).unwrap()).unwrap();
        let composed = apply(&pq, &a).unwrap();
        prop_assert_eq!(composed, sequential);
    }

    #[test]
    fn law_compose_associative(
        (a, m) in tree_pair(),
        n_seed in tree_strategy(),
        b_seed in tree_strategy(),
    ) {
        let mut n = n_seed;
        n.identity = Some("root".into());
        let mut b = b_seed;
        b.identity = Some("root".into());

        let p = diff(&a, &m);
        let q = diff(&m, &n);
        let r = diff(&n, &b);

        let left = compose(&compose(&p, &q), &r);
        let right = compose(&p, &compose(&q, &r));
        prop_assert_eq!(left, right);
    }

    #[test]
    fn law_compose_inverse_cancels((a, b) in tree_pair()) {
        let p = diff(&a, &b);
        prop_assert_eq!(compose(&p, &invert(&p)), Patch::empty());
    }

    #[test]
    fn law_compose_identity((a, b) in tree_pair()) {
        let p = diff(&a, &b);
        prop_assert_eq!(compose(&p, &Patch::empty()), p.clone());
        prop_assert_eq!(compose(&Patch::empty(), &p), p);
    }

    #[test]
    fn law_merge_identical_branches_clean((base, side) in tree_pair()) {
        let r = tree_merge(&base, &side, &side);
        prop_assert_eq!(r.conflicts.len(), 0);
    }

    #[test]
    fn law_merge_conflict_set_symmetric(
        (base, ours) in tree_pair(),
        theirs_seed in tree_strategy(),
    ) {
        let mut theirs = theirs_seed;
        theirs.identity = Some("root".into());

        let ot = tree_merge(&base, &ours, &theirs);
        let to = tree_merge(&base, &theirs, &ours);

        let key = |c: &tate::tree::TreeConflict| (c.kind, c.path.clone(), c.attr.clone());
        let mut a: Vec<_> = ot.conflicts.iter().map(key).collect();
        let mut b: Vec<_> = to.conflicts.iter().map(key).collect();
        a.sort();
        b.sort();
        prop_assert_eq!(a, b);
    }

    /// The merged section at every identity must be consistent with the
    /// field-wise pushout: each field (parent, kind, text, attrs, order)
    /// is independently merged. Identities flagged Dangling (a structural
    /// obstruction from the tree topology) are dropped from the result, so
    /// they have no entry in `merged`.
    #[test]
    fn law_merge_is_fieldwise_pushout((base, ours) in tree_pair(), theirs_seed in tree_strategy()) {
        let mut theirs = theirs_seed;
        theirs.identity = Some("root".into());

        let (sb, so, st) = (base.to_section(), ours.to_section(), theirs.to_section());
        let r = merge_sections(&sb, &so, &st);

        let dangling: std::collections::BTreeSet<String> =
            r.conflicts.iter()
                .filter(|c| c.kind == SectionConflictKind::Dangling)
                .map(|c| c.identity.clone())
                .collect();
        let conflict_ids: std::collections::BTreeSet<String> =
            r.conflicts.iter().map(|c| c.identity.clone()).collect();

        let mut ids: std::collections::BTreeSet<String> = sb.nodes.keys().cloned().collect();
        ids.extend(so.nodes.keys().cloned());
        ids.extend(st.nodes.keys().cloned());

        for id in &ids {
            let b = sb.nodes.get(id);
            let o = so.nodes.get(id);
            let t = st.nodes.get(id);
            let m = r.merged.nodes.get(id);

            if dangling.contains(id) {
                prop_assert!(m.is_none(), "dangling id {} must be dropped", id);
                continue;
            }

            if o == b {
                prop_assert_eq!(m, t, "only theirs moved at {}", id);
                prop_assert!(!conflict_ids.contains(id));
            } else if t == b || o == t {
                prop_assert_eq!(m, o, "only ours moved / both agree at {}", id);
                prop_assert!(!conflict_ids.contains(id));
            } else if conflict_ids.contains(id) {
                prop_assert_eq!(m, o, "field conflict favours ours at {}", id);
            }
        }
    }

    /// Sheaf consistency invariant: every present node in the merged section
    /// has a present parent (or is the root). This is the defining property
    /// that distinguishes the tree-space sheaf merge from the discrete model.
    #[test]
    fn law_merged_section_has_no_dangling_parents((base, ours) in tree_pair(), theirs_seed in tree_strategy()) {
        let mut theirs = theirs_seed;
        theirs.identity = Some("root".into());

        let (sb, so, st) = (base.to_section(), ours.to_section(), theirs.to_section());
        let r = merge_sections(&sb, &so, &st);

        for (id, n) in &r.merged.nodes {
            if let Some(p) = &n.parent {
                prop_assert!(r.merged.nodes.contains_key(p),
                    "dangling parent {} at {} after merge", p, id);
            }
        }
    }

    #[test]
    fn law_merge_sections_identical_branches((base, side) in tree_pair()) {
        let (sb, ss) = (base.to_section(), side.to_section());
        let r = merge_sections(&sb, &ss, &ss);
        prop_assert!(r.conflicts.is_empty());
        prop_assert_eq!(r.merged, ss);
    }

    #[test]
    fn law_nway_merge_is_pointwise_pushout(
        base_seed in tree_strategy(),
        branch_seeds in prop::collection::vec(tree_strategy(), 2..5),
    ) {
        let mut base = base_seed;
        base.identity = Some("root".into());
        let base = base;

        let base_sec = base.to_section();
        let branches: Vec<_> = branch_seeds.into_iter().map(|mut t| {
            t.identity = Some("root".into());
            t.to_section()
        }).collect();

        let r = merge_sections_nway(&base_sec, &branches);

        let field_conflict: std::collections::BTreeSet<&String> = r.conflicts.iter()
            .filter(|c| c.kind == SectionConflictKind::Field)
            .map(|c| &c.identity).collect();
        let dangling: std::collections::BTreeSet<&String> = r.conflicts.iter()
            .filter(|c| c.kind == SectionConflictKind::Dangling)
            .map(|c| &c.identity).collect();

        let mut ids: std::collections::BTreeSet<String> = base_sec.nodes.keys().cloned().collect();
        for b in &branches {
            ids.extend(b.nodes.keys().cloned());
        }

        for id in &ids {
            let b = base_sec.nodes.get(id);
            let moved: std::collections::BTreeSet<Option<&tate::section::Node>> = branches.iter()
                .map(|s| s.nodes.get(id)).filter(|v| v != &b).collect();

            if moved.len() > 1 {
                prop_assert!(field_conflict.contains(id),
                    "field conflict at {} but missing from conflict set", id);
            }
            if dangling.contains(id) {
                prop_assert!(!r.merged.nodes.contains_key(id),
                    "dangling id {} must be dropped", id);
            }
        }

        // Sheaf consistency invariant: every surviving node has a present parent.
        for (id, n) in &r.merged.nodes {
            if let Some(p) = &n.parent {
                prop_assert!(r.merged.nodes.contains_key(p),
                    "dangling parent {} at {} after n-way merge", p, id);
            }
        }
    }
}
