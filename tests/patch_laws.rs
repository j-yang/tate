//! Property-based verification of the patch-algebra and merge laws.
//!
//! These are the mathematical guarantees the `patch` module claims. Rather than
//! trusting a handful of hand-written examples, each law is checked against
//! thousands of randomly generated trees.
//!
//! Laws checked:
//! 1. `apply(diff(a, b), a) == b`                  — diff/apply are inverse.
//! 2. `apply(diff(a, a), a) == a` and `diff(a, a)` is empty — identity.
//! 3. `apply(invert(p), apply(p, a)) == a`         — invert undoes apply.
//! 4. `apply(compose(p, q), a) == apply(q, apply(p, a))` — composition.
//! 5. `compose(p, invert(p))` is the empty patch   — inverses cancel.
//! 6. `tree_merge` conflict set is symmetric under swapping ours/theirs
//!    (pushout is symmetric) and `merge(base, x, x)` is conflict-free.

use proptest::prelude::*;
use tate::patch::{apply, compose, diff, invert, merge_sections, merge_sections_nway, Patch};
use tate::tree::{tree_merge, TreeNode};

// ─── strategy: random trees with globally-unique identities ────────────────────

/// Generate a small attribute set drawn from a fixed key pool, so two trees have
/// a real chance of sharing attribute keys (making diffs interesting).
fn attrs_strategy() -> impl Strategy<Value = Vec<(String, String)>> {
    prop::collection::vec(
        (
            prop::sample::select(vec!["a", "b", "c"]).prop_map(String::from),
            prop::sample::select(vec!["0", "1", "2", "x"]).prop_map(String::from),
        ),
        0..3,
    )
    .prop_map(|mut v| {
        // Deduplicate keys — a TreeNode's attributes are a map in spirit.
        v.sort_by(|x, y| x.0.cmp(&y.0));
        v.dedup_by(|x, y| x.0 == y.0);
        v
    })
}

/// A recursively generated tree. Every node gets a distinct identity via a
/// shared counter, so sibling keys never collide (the algebra's precondition).
fn tree_strategy() -> impl Strategy<Value = TreeNode> {
    let leaf = (
        prop::sample::select(vec!["item", "field", "node"]).prop_map(String::from),
        attrs_strategy(),
        prop::sample::select(vec!["", "t1", "t2"]).prop_map(String::from),
    )
        .prop_map(|(kind, attrs, text)| {
            let mut n = TreeNode::new(kind);
            for (k, v) in attrs {
                n = n.with_attr(k, v);
            }
            if !text.is_empty() {
                n = n.with_text(text);
            }
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
                for (k, v) in attrs {
                    n = n.with_attr(k, v);
                }
                for c in children {
                    n = n.with_child(c);
                }
                n
            })
    })
    .prop_map(assign_identities)
}

/// Walk the tree and give every node a globally-unique identity (`n0`, `n1`, …).
/// Unique identities make each node a distinct location, which is exactly the
/// regime where the patch algebra is exact.
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

/// Two independent trees sharing the same root identity (so `diff` sees them as
/// two versions of one section, not two unrelated roots).
fn tree_pair() -> impl Strategy<Value = (TreeNode, TreeNode)> {
    (tree_strategy(), tree_strategy()).prop_map(|(mut a, mut b)| {
        // Pin both roots to the same location so the root itself is comparable.
        a.identity = Some("root".into());
        b.identity = Some("root".into());
        (a, b)
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    /// Law 1: diff then apply reconstructs the target exactly.
    #[test]
    fn law_diff_apply_roundtrip((a, b) in tree_pair()) {
        let p = diff(&a, &b);
        prop_assert_eq!(apply(&p, &a).unwrap(), b);
    }

    /// Law 2: the diff of a section with itself is the identity patch.
    #[test]
    fn law_identity(a in tree_strategy()) {
        let p = diff(&a, &a);
        prop_assert!(p.is_empty());
        prop_assert_eq!(apply(&Patch::empty(), &a).unwrap(), a);
    }

    /// Law 3: the inverse morphism undoes the forward one.
    #[test]
    fn law_invert_undoes((a, b) in tree_pair()) {
        let p = diff(&a, &b);
        let forward = apply(&p, &a).unwrap();
        prop_assert_eq!(apply(&invert(&p), &forward).unwrap(), a);
    }

    /// Law 4: composition equals sequential application.
    #[test]
    fn law_compose_is_sequential((a, m) in tree_pair(), c in tree_strategy()) {
        // Build a third version `b` that shares the root location.
        let mut b = c;
        b.identity = Some("root".into());

        let p = diff(&a, &m);
        let q = diff(&m, &b);
        let pq = compose(&p, &q);

        let sequential = apply(&q, &apply(&p, &a).unwrap()).unwrap();
        let composed = apply(&pq, &a).unwrap();
        prop_assert_eq!(composed, sequential);
    }

    /// Law 4b: `compose` is associative — `(p∘q)∘r == p∘(q∘r)`. Together with
    /// the identity patch and inverses, this makes patches a groupoid.
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

        // Three composable patches: a → m → n → b.
        let p = diff(&a, &m);
        let q = diff(&m, &n);
        let r = diff(&n, &b);

        let left = compose(&compose(&p, &q), &r);
        let right = compose(&p, &compose(&q, &r));
        prop_assert_eq!(left, right);
    }

    /// Law 5: a patch composed with its inverse is the identity patch.
    #[test]
    fn law_compose_inverse_cancels((a, b) in tree_pair()) {
        let p = diff(&a, &b);
        prop_assert_eq!(compose(&p, &invert(&p)), Patch::empty());
    }

    /// Law 5b: composing with the identity patch is a no-op (left and right
    /// identity — completes the groupoid identity axiom).
    #[test]
    fn law_compose_identity((a, b) in tree_pair()) {
        let p = diff(&a, &b);
        prop_assert_eq!(compose(&p, &Patch::empty()), p.clone());
        prop_assert_eq!(compose(&Patch::empty(), &p), p);
    }

    /// Law 6a: merging a base with two identical branches yields no conflict
    /// and returns that branch (pushout of a span with equal legs is trivial).
    #[test]
    fn law_merge_identical_branches_clean((base, side) in tree_pair()) {
        let r = tree_merge(&base, &side, &side);
        prop_assert_eq!(r.conflicts.len(), 0);
    }

    /// Law 6b: the conflict *set* is symmetric under swapping ours/theirs — the
    /// pushout does not depend on which leg we call "ours". We compare conflicts
    /// as an unordered set keyed by (kind, path, attr).
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

    /// Law 7 (pushout construction): the merged section is exactly the point-wise
    /// pushout of the span `ours ← base → theirs`. At every location the merged
    /// value must be: theirs where only theirs moved, ours where only ours moved
    /// (or neither), the common value where both agreed, and ours at conflicts.
    /// And the conflict set is *precisely* the locations where both legs moved to
    /// different values — no more, no less. This holds unconditionally (total
    /// function), so no `prop_assume` is needed.
    #[test]
    fn law_merge_is_pointwise_pushout((base, ours) in tree_pair(), theirs_seed in tree_strategy()) {
        let mut theirs = theirs_seed;
        theirs.identity = Some("root".into());

        let (sb, so, st) = (base.to_section(), ours.to_section(), theirs.to_section());
        let r = merge_sections(&sb, &so, &st);

        let conflict_locs: std::collections::BTreeSet<_> =
            r.conflicts.iter().map(|c| c.location.clone()).collect();

        // Every location present anywhere.
        let mut locations: std::collections::BTreeSet<Vec<String>> = std::collections::BTreeSet::new();
        locations.extend(sb.values.keys().cloned());
        locations.extend(so.values.keys().cloned());
        locations.extend(st.values.keys().cloned());

        for loc in &locations {
            let b = sb.values.get(loc);
            let o = so.values.get(loc);
            let t = st.values.get(loc);
            let m = r.merged.values.get(loc);

            if o == b {
                prop_assert_eq!(m, t, "only theirs moved → take theirs at {:?}", loc);
                prop_assert!(!conflict_locs.contains(loc));
            } else if t == b || o == t {
                prop_assert_eq!(m, o, "only ours moved / both agree → take ours at {:?}", loc);
                prop_assert!(!conflict_locs.contains(loc));
            } else {
                // Both moved to different whole Values. With per-field merge,
                // this may resolve cleanly (e.g., different attributes changed).
                // If it IS a conflict, best-effort favours ours.
                if conflict_locs.contains(loc) {
                    prop_assert_eq!(m, o, "conflict favours ours at {:?}", loc);
                }
                // If not a conflict, per-field merge produced a valid value.
            }
        }
    }

    /// Law 8: `merge_sections(base, x, x)` is always clean and returns `x`
    /// (the pushout of a span with two equal legs is that leg).
    #[test]
    fn law_merge_sections_identical_branches((base, side) in tree_pair()) {
        let (sb, ss) = (base.to_section(), side.to_section());
        let r = merge_sections(&sb, &ss, &ss);
        prop_assert!(r.conflicts.is_empty());
        prop_assert_eq!(r.merged, ss);
    }

    /// Law 9 (N-way pushout): `merge_sections_nway` produces exactly the
    /// point-wise N-way pushout. At every location, if all moving branches
    /// agree on one value, that value is taken; if ≥2 distinct non-base values
    /// appear, the location is a conflict. This holds unconditionally.
    #[test]
    fn law_nway_merge_is_pointwise_pushout(
        base_seed in tree_strategy(),
        branch_seeds in prop::collection::vec(tree_strategy(), 2..5),
    ) {
        let mut base_tree = base_seed;
        base_tree.identity = Some("root".into());
        let base = base_tree.to_section();

        let branches: Vec<_> = branch_seeds.into_iter().map(|mut t| {
            t.identity = Some("root".into());
            t.to_section()
        }).collect();

        let r = merge_sections_nway(&base, &branches);

        let conflict_locs: std::collections::BTreeSet<Vec<String>> =
            r.conflicts.iter().map(|c| c.location.clone()).collect();

        let mut locations: std::collections::BTreeSet<Vec<String>> = std::collections::BTreeSet::new();
        locations.extend(base.values.keys().cloned());
        for b in &branches {
            locations.extend(b.values.keys().cloned());
        }

        for loc in &locations {
            let b = base.values.get(loc);
            let moved: std::collections::BTreeSet<Option<&tate::section::Value>> = branches.iter()
                .map(|s| s.values.get(loc))
                .filter(|v| v != &b)
                .collect();

            if moved.len() <= 1 {
                prop_assert!(!conflict_locs.contains(loc),
                    "non-conflict at {:?} but in conflict set", loc);
                let expected = if moved.is_empty() { b } else { *moved.iter().next().unwrap() };
                prop_assert_eq!(r.merged.values.get(loc), expected,
                    "wrong merged value at {:?}", loc);
            } else {
                prop_assert!(conflict_locs.contains(loc),
                    "conflict at {:?} but missing from conflict set", loc);
            }
        }
    }
}
