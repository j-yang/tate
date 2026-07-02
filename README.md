# tate

A pure structured diff, patch, and merge algebra for Rust — one object (a tree = a section of a `location → value` sheaf), one 3-way merge, and a lossless patch algebra with proptest-verified laws. Zero format-parsing, zero external diff-engine dependencies.

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

## Overview

Tate is built on one commitment: **every structure is a tree**, and a tree is a *section* of a `location → value` sheaf. Diff, 3-way merge, and a lossless patch algebra are defined once, on that single object. Four modules, zero format-parsing and zero external diff-engine dependencies (only optional `serde`):

- **`section`** — The canonical object: a `Section` is the flat `location → value` form of a tree (the sheaf section the algebra runs on). Convert with `TreeNode::to_section` / `Section::to_tree`. Identity is the location; structural position (`order`) and scalar content are values, so moves and renames are value changes, not delete+add.

- **`tree`** — The nested `TreeNode` view, its structural `tree_diff`, and `tree_merge` — tate's **single** 3-way merge. Merge is the **pushout** of two branch patches in the category of sections; when change sets are disjoint, the pushout exists and the merge glues cleanly. Where both branches change the same location incompatibly, the obstruction (a `TreeConflict`) is recorded — the conflict set is the first Čech cohomology H¹ of the cover {U_ours, U_theirs}.

- **`patch`** — A lossless patch algebra over sections: `diff` / `apply` / `invert` / `compose`, the morphisms of the versioned-structure **groupoid**. Unlike `tree_diff` (a lossy display diff), it round-trips. The laws (`apply(diff(a, b), a) == b`, `invert` undoes `apply`, `compose` equals sequential `apply`, and associativity) are verified by `proptest`.

- **`change`** — A `ChangeSet`: a tree diff or patch tagged with version metadata (labels, timestamp, author) for audit and cross-language pipelines.

> **Byte parsing and alignment live elsewhere.** Turning *files* (Excel, PDF, Word, JSON, plain text) into a tree — including the LCS and coordinate-descent alignment that give un-keyed sequence/grid data stable identities — is the job of the [`mumford`](https://github.com/j-yang/mumford) crate. tate itself is the pure algebra: no format-parsing, no serde_json, no alignment heuristics.

## Usage

```toml
[dependencies]
tate = { version = "0.6", features = ["serde"] }
```

### Tree diff (format-agnostic)

```rust
use tate::tree::{TreeNode, tree_diff, ChangeKind};

let a = TreeNode::new("root")
    .with_child(TreeNode::new("entry").with_identity("u1").with_attr("level", "1"));
let b = TreeNode::new("root")
    .with_child(TreeNode::new("entry").with_identity("u1").with_attr("level", "99"));

let diff = tree_diff(&a, &b);
assert_eq!(diff.changes.len(), 1);
assert_eq!(diff.changes[0].kind, ChangeKind::Modified);
```

### 3-way merge (the single merge)

```rust
use tate::tree::{TreeNode, tree_merge};

let base = TreeNode::new("root").with_child(TreeNode::new("e").with_identity("u1").with_attr("v", "1"));
let ours = TreeNode::new("root").with_child(TreeNode::new("e").with_identity("u1").with_attr("v", "9"));
let theirs = base.clone();

let r = tree_merge(&base, &ours, &theirs);      // ours changed v, theirs did not
assert_eq!(r.conflicts.len(), 0);               // clean glue
assert_eq!(r.tree.children[0].attr("v"), Some("9"));
```

### Lossless patch (round-trips)

```rust
use tate::tree::TreeNode;
use tate::patch::{diff, apply};

let a = TreeNode::new("root").with_child(TreeNode::new("x").with_identity("1"));
let b = TreeNode::new("root")
    .with_child(TreeNode::new("x").with_identity("1"))
    .with_child(TreeNode::new("y").with_identity("2"));

let p = diff(&a, &b);
assert_eq!(apply(&p, &a).unwrap(), b);          // apply(diff(a, b), a) == b
```

## Mathematical foundation

tate models structured data as **sections of a location→value sheaf** on the Alexandrov topology of a tree's prefix poset. This is not a metaphor — it is the design:

- **Diff** is point-wise comparison of sections (discrete base space).
- **Merge** is the **pushout** of two branch patches in the category of sections. Clean merge = pushout exists; conflict = obstruction (pushout does not exist).
- **Conflicts** are the **first Čech cohomology** H¹ of the two-cover {U_ours, U_theirs}. Each conflicting location is a generator of H¹.
- **Patches** form a **groupoid**: every patch has an inverse (`invert`), composition is associative, and the identity is the empty patch. The laws are verified by `proptest` (2000 random cases each).

See [`MATHEMATICS.md`](MATHEMATICS.md) for the full treatment.

## Design

- **Self-contained.** Zero external dependencies beyond `serde` (optional, behind the default `serde` feature). No `similar`, no `roxmltree`, no `serde_json`, no `imara-diff`.

- **Format-agnostic.** `tree_diff` / `tree_merge` / `patch` operate on `TreeNode` (equivalently, its `Section`) — callers convert from XML, JSON, YAML, or any tree format. tate has no file-format knowledge; parsing and alignment live in `mumford`.

- **One object, one merge.** seq / grid / table are not parallel cases — they are trees once keyed. The unification is at the level of the object (`Section`) and the laws, not a pile of per-shape algorithms.

- **Tested.** 42 unit tests + 8 property tests (proptest-verified groupoid laws) + 9 doctests, covering keyless-node bubbling, root tag rename, the four conflict classes, text-vs-attribute merge, and the diff/apply/invert/compose laws.

## License

MIT
