# tate

A version control kernel for structured data — identity-keyed sections with
field-wise pushout merge and a lossless patch algebra. Zero format-parsing,
zero external diff-engine dependencies.

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

## What changed in 2.0

**Identity is separated from location.** In tate 1.x, a node's key in the
section was its full path from root. In tate 2.0, the key is the node's
**identity** (a string), and its position (parent, order) is stored as a
**field** of the node.

This separation enables:

- **Move is a field change, not delete+insert.** Moving a node changes its
  `parent` field at a stable identity key.
- **Move + Modify merge cleanly.** Alice moves a node (changes `parent`),
  Bob modifies its value (changes `text`/`attrs`). Different fields →
  field-wise pushout merge resolves both independently → no false conflict.
- **Field-wise merge.** Each field (`parent`, `kind`, `label`, `text`,
  `attrs`, `order`) is merged independently. Two branches that change
  different attributes of the same node merge cleanly.

## Overview

- **`section`** — The canonical object: `Section = BTreeMap<Identity, Node>`.
  Each `Node` stores its `parent` (position), `kind`, `label`, `text`,
  `attrs`, and `order`. Identity is the key; position is a value.

- **`patch`** — Lossless patch algebra: `diff` / `apply` / `invert` /
  `compose`. Plus `merge_sections` (3-way) and `merge_sections_nway`
  (N-branch) — the **field-wise pushout** merge. At each identity, each
  field is merged independently. Laws verified by proptest (2000 cases).

- **`tree`** — The nested `TreeNode` view, its structural `tree_diff`, and
  `tree_merge` — the display-oriented merge for UIs.

- **`change`** — Versioned change sets with metadata for audit pipelines.

- **`repo`** — VCS kernel: content-addressed sections, commit DAG,
  merge (pushout), cherry-pick, revert, branches.

## Usage

```toml
[dependencies]
tate = { version = "2", features = ["serde"] }
```

### Tree diff

```rust
use tate::tree::{TreeNode, tree_diff, ChangeKind};

let a = TreeNode::new("root")
    .with_child(TreeNode::new("server").with_identity("s1").with_attr("port", "8080"));
let b = TreeNode::new("root")
    .with_child(TreeNode::new("server").with_identity("s1").with_attr("port", "9090"));

let d = tree_diff(&a, &b);
assert_eq!(d.changes[0].kind, ChangeKind::Modified);
```

### Move + Modify clean merge (the 2.0 feature)

```rust
use tate::tree::{TreeNode, tree_merge};

let base = TreeNode::new("root")
    .with_child(TreeNode::new("server").with_identity("s1").with_attr("port", "8080"))
    .with_child(TreeNode::new("db").with_identity("d1"));

// Alice moves s1 under d1.
let mut moved = base.clone();
let s1 = moved.children.remove(0);
moved.children[0].children.push(s1);

// Bob modifies s1's port.
let mut modified = base.clone();
modified.children[0].attributes[0].1 = "9090".into();

// Merge: move (parent field) + modify (attrs field) → clean.
let r = tree_merge(&base, &moved, &modified);
// tree_merge may still conflict (it's display-oriented);
// use merge_sections for the field-wise pushout:
use tate::patch::merge_sections;
let result = merge_sections(
    &base.to_section(),
    &moved.to_section(),
    &modified.to_section(),
);
assert!(result.conflicts.is_empty()); // field-wise merge resolves cleanly
```

### In-app version control (Repo)

```rust
use tate::tree::TreeNode;
use tate::repo::Repo;

let mut repo = Repo::new();

let v0 = repo.commit("initial", &[], &TreeNode::new("root")
    .with_child(TreeNode::new("server").with_identity("s1").with_attr("port", "8080")));

let v1 = repo.commit("port -> 9090", &[v0], &TreeNode::new("root")
    .with_child(TreeNode::new("server").with_identity("s1").with_attr("port", "9090")));

let patch = repo.diff(v0, v1);
for (id, edit) in &patch.edits {
    println!("  {id}: {:?} -> {:?}", edit.old, edit.new);
}
```

## Mathematical foundation

tate models structured data as **identity-keyed sections**. A section maps
`Identity → Node`, where each `Node` stores both position (`parent`, `order`)
and content (`kind`, `text`, `attrs`).

**Merge** is the **field-wise pushout** of the span `ours ← base → theirs`.
At each identity, each field is merged independently:

- If only one side changed a field → take that value.
- If both changed it to the same value → take it.
- If both changed it to different values → conflict.

This is the categorical pushout in the product category
∏<sub>(identity, field)</sub> Set — one factor per (identity, field) pair.

**Patches** form a **groupoid**: every patch has an inverse (`invert`),
composition is associative, and the identity is the empty patch. The laws
are verified by proptest (2000 random cases each).

**Key insight**: separating identity from location enables Move + Modify
commutation — they touch different fields of the same node, so they
commute trivially. This is impossible in location-keyed models (including
Pijul's line-based patch theory), where Move changes the key.

See [`MATHEMATICS.md`](MATHEMATICS.md) for the full treatment.

## Design

- **Self-contained.** Zero external dependencies beyond `serde` (optional).
- **Identity-keyed.** The fundamental data structure is
  `BTreeMap<Identity, Node>`, not `BTreeMap<Path, Value>`.
- **Field-wise merge.** Different fields of the same node merge independently.
- **VCS kernel.** Content-addressed storage + commit DAG + pushout merge.
- **Tested.** 58 unit tests + 12 property tests + 5 doctests = 75 total.

## Migration from 1.x

**Breaking changes:**

- `Section.values: BTreeMap<Location, Value>` → `Section.nodes: BTreeMap<Identity, Node>`
- `Patch.edits: BTreeMap<Location, PointEdit>` → `Patch.edits: BTreeMap<Identity, NodeEdit>`
- `SectionConflict.location` → `SectionConflict.identity`
- `Value` → `Node` (adds `parent` field)
- `PointEdit` → `NodeEdit`

The tree-facing API (`TreeNode`, `tree_diff`, `tree_merge`) is unchanged.

## License

MIT
