//! Benchmark: false-conflict rate of line-based merge vs identity-based merge.
//!
//! Generates random identity-keyed trees, applies random single-location edits
//! to each branch, and compares whether git-style line merge would conflict
//! vs tate's field-wise pushout merge.
//!
//! The key metric: when two branches edit DIFFERENT identities (logically
//! disjoint), how often does a text-based diff see them in the same "hunk"
//! (within `context_lines` of each other) and produce a false conflict?

use tate::tree::TreeNode;

fn main() {
    println!("tree_size  edits  line_conflict_rate  identity_conflict_rate  false_conflict_rate");
    println!("---------  -----  ------------------  ----------------------  --------------------");

    for &tree_size in &[10, 20, 50, 100, 200, 500] {
        for &context in &[3usize] {
            let trials = 10000;
            let mut line_conflicts = 0usize;
            let mut identity_conflicts = 0usize;
            let mut false_conflicts = 0usize; // line conflicts where identity merge is clean

            for _ in 0..trials {
                let tree = random_tree(tree_size);
                let identities: Vec<String> = tree.to_section().nodes.keys().cloned().collect();
                if identities.len() < 4 {
                    continue;
                }

                // Pick two distinct identities to edit.
                let i1 = identities[fastrand_usize(identities.len())].clone();
                let i2 = loop {
                    let candidate = identities[fastrand_usize(identities.len())].clone();
                    if candidate != i1 { break candidate; }
                };

                let ours = modify_attr(&tree, &i1, "v", "modified_a");
                let theirs = modify_attr(&tree, &i2, "v", "modified_b");

                // Identity-based merge (tate).
                let result = tate::patch::merge_sections(
                    &tree.to_section(),
                    &ours.to_section(),
                    &theirs.to_section(),
                );
                let id_clean = result.conflicts.is_empty();

                // Simulate line-based merge: serialize both trees to text lines,
                // check if the two edited locations fall within `context` lines
                // of each other in the serialized form.
                let base_lines = serialize_tree(&tree);
                let ours_lines = serialize_tree(&ours);
                let theirs_lines = serialize_tree(&theirs);

                let line_conflict = line_merge_would_conflict(
                    &base_lines, &ours_lines, &theirs_lines, context,
                );

                if line_conflict { line_conflicts += 1; }
                if !id_clean { identity_conflicts += 1; }
                if line_conflict && id_clean { false_conflicts += 1; }
            }

            println!(
                "{:9} {:5} {:6.1}% {:6.1}% {:6.1}%",
                tree_size,
                context,
                line_conflicts as f64 / trials as f64 * 100.0,
                identity_conflicts as f64 / trials as f64 * 100.0,
                false_conflicts as f64 / trials as f64 * 100.0,
            );
        }
    }
}

/// Generate a random identity-keyed tree with approximately `n` nodes.
fn random_tree(n: usize) -> TreeNode {
    let mut root = TreeNode::new("root").with_identity("root");
    let count = n.saturating_sub(1); // root takes one slot
    for i in 0..count {
        let depth = fastrand_usize(3);
        let node = TreeNode::new(if depth == 0 { "leaf" } else { "branch" })
            .with_identity(format!("n{}", i))
            .with_attr("v", format!("val{}", i));
        root = root.with_child(node);
    }
    root
}

/// Modify one attribute of one node (by identity), returning a new tree.
fn modify_attr(tree: &TreeNode, identity: &str, key: &str, value: &str) -> TreeNode {
    let mut t = tree.clone();
    modify_attr_rec(&mut t, identity, key, value);
    t
}

fn modify_attr_rec(node: &mut TreeNode, identity: &str, key: &str, value: &str) {
    if node.identity.as_deref() == Some(identity) {
        let mut found = false;
        for (k, v) in &mut node.attributes {
            if k == key {
                *v = value.to_string();
                found = true;
            }
        }
        if !found {
            node.attributes.push((key.to_string(), value.to_string()));
        }
    }
    for child in &mut node.children {
        modify_attr_rec(child, identity, key, value);
    }
}

/// Serialize a tree to text lines (simulating JSON output).
fn serialize_tree(node: &TreeNode) -> Vec<String> {
    let mut lines = Vec::new();
    serialize_rec(node, 0, &mut lines);
    lines
}

fn serialize_rec(node: &TreeNode, depth: usize, lines: &mut Vec<String>) {
    let indent = "  ".repeat(depth);
    let id = node.identity.as_deref().unwrap_or("?");
    lines.push(format!("{indent}{} (id={}) kind={}", node.text, id, node.kind));
    for (k, v) in &node.attributes {
        lines.push(format!("{indent}  {} = {}", k, v));
    }
    for child in &node.children {
        serialize_rec(child, depth + 1, lines);
    }
}

/// Simulate whether a 3-way line-based merge would produce a conflict.
///
/// Two edits conflict (from line-based merge's perspective) if they modify
/// line ranges that overlap or are within `context` lines of each other.
fn line_merge_would_conflict(
    base: &[String],
    ours: &[String],
    theirs: &[String],
    context: usize,
) -> bool {
    let our_changes = diff_line_ranges(base, ours);
    let their_changes = diff_line_ranges(base, theirs);

    for &(os, oe) in &our_changes {
        for &(ts, te) in &their_changes {
            // Check if the two change ranges are within `context` lines.
            if oe + context >= ts && te + context >= os {
                return true; //Ranges overlap or are adjacent → line merge conflicts.
            }
        }
    }
    false
}

/// Find (start, end) line ranges where `b` differs from `a`.
fn diff_line_ranges(a: &[String], b: &[String]) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut i = 0;
    while i < a.len().min(b.len()) {
        if a[i] != b[i] {
            let start = i;
            while i < a.len().min(b.len()) && a[i] != b[i] {
                i += 1;
            }
            ranges.push((start, i));
        } else {
            i += 1;
        }
    }
    // Handle length differences.
    if a.len() != b.len() {
        let start = a.len().min(b.len());
        ranges.push((start, a.len().max(b.len())));
    }
    ranges
}

fn fastrand_usize(max: usize) -> usize {
    if max == 0 { return 0; }
    // Simple deterministic PRNG for reproducibility.
    use std::cell::Cell;
    thread_local! {
        static STATE: Cell<u64> = Cell::new(0x123456789ABCDEF0);
    }
    STATE.with(|s| {
        let mut x = s.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.set(x);
        (x as usize) % max
    })
}
