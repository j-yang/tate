# tate

Structured diff, patch algebra, and sheaf-pushout merge for tree-shaped
data (JSON / YAML / TOML). Merge is the pushout in the sheaf category on
the tree space: a pointwise per-field pushout followed by sheafification
that enforces referential integrity. The library is dependency-light
(`serde` optional); a companion CLI lives in this same repo as
[`tate-cli`](cli).

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

## The identity-keyed model

**Identity is separated from location.** A node's key in the section is its
**identity** (a string); its position (`parent`, `order`) is stored as a
**field** of the node — not as part of the key.

This separation enables:

- **Move is a field change, not delete+insert.** Moving a node changes its
  `parent` field at a stable identity key.
- **Move + Modify merge cleanly.** Alice moves a node (changes `parent`),
  Bob modifies its value (changes `text`/`attrs`). Different fields →
  field-wise pushout merge resolves both independently → no false conflict.
- **Field-wise merge.** Each field (`parent`, `kind`, `label`, `text`,
  `attrs`, `order`) is merged independently. Two branches that change
  different attributes of the same node merge cleanly.
- **Structural (Dangling) conflicts.** Sheafification drops present nodes
  whose parent was concurrently deleted — an obstruction no discrete
  per-field model can detect.

## Overview

- **`section`** — The canonical object: `Section = BTreeMap<Identity, Node>`.
  Each `Node` stores its `parent` (position), `kind`, `label`, `text`,
  `attrs`, and `order`. Identity is the key; position is a value.

- **`patch`** — Lossless patch algebra: `diff` / `apply` / `invert` /
  `compose`. Plus `merge_sections` (3-way) and `merge_sections_nway`
  (N-branch) — the **sheaf pushout** on the tree space. Stage 1 merges
  each field independently; Stage 2 (sheafification) drops dangling-parent
  nodes, reporting `Field` and `Dangling` conflicts. Laws + the sheaf
  consistency invariant verified by proptest (2000 cases).

- **`tree`** — The nested `TreeNode` view, its structural `tree_diff`, and
  `tree_merge` — the display-oriented merge for UIs.

- **`change`** — Versioned change sets with metadata for audit pipelines.

- **`repo`** — VCS kernel: content-addressed sections, commit DAG,
  merge (pushout), cherry-pick, revert, branches.

## Usage

```toml
[dependencies]
tate = { version = "3", features = ["serde"] }
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

### Move + Modify clean merge

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

## Command-line tool (`tate-cli`)

A thin CLI over the algebra — structural diff, patch algebra, and a
**git merge driver** that runs the sheaf merge during `git merge`. It is a
separate crate in this workspace so the library stays dependency-light.

```bash
cargo install tate-cli
```

```bash
tate diff a.json b.json
tate patch diff a.json b.json > p.json && tate patch apply p.json a.json
tate git-merge base.json ours.json theirs.json   # writes result to ours; exit 1 on conflict
```

### Use as a git merge driver

Once installed, register it so every `git merge` on the registered
extensions runs the sheaf merge automatically:

```bash
git config --global merge.tate.driver 'tate git-merge %O %A %B'
echo '*.json merge=tate' >> .gitattributes
```

On a **Field** conflict the driver writes standard git conflict markers
(`<<<<<<<` / `=======` / `>>>>>>>`) into the file for an editor or
`git mergetool` to resolve. A **Dangling** (structural) conflict is
reported on stderr and the orphaned node is dropped.

## Mathematical foundation

tate models structured data as a **sheaf on the tree space**. The base
space is the identity poset with the **Alexandrov topology of ancestry**
(opens = ancestor-closed subtrees), not a discrete key set. A section of
the sheaf assigns each identity a node state and must satisfy
**referential integrity**: a present node has a present parent.

**Merge** is the **pushout in the sheaf category** of the span
`ours ← base → theirs`, computed in two stages:

1. **Pointwise per-field pushout** — at each identity, each field
   (`parent`, `kind`, `label`, `text`, `attrs`, `order`) is merged by the
   four-way rule (take-t / take-o / take-agreed / conflict).
2. **Sheafification** — a fixpoint that drops present nodes whose `parent`
   is absent, enforcing the integrity constraint.

This yields **two conflict classes**:

- `Field` — both branches changed the same field to incompatible values
  (the only kind a discrete per-field model can see).
- `Dangling` — the pointwise pushout left a present node referencing an
  absent parent. This is a *structural* obstruction that the ancestry
  topology surfaces and any discrete model is blind to (proven strictly:
  there are inputs where the discrete merge reports zero conflicts yet
  returns a non-section).

**Identity-location separation** composes with the sheaf: a node's
identity is its key, its position (`parent`) is a value. Move is a
parent-field change, not a delete+insert; so Move + Modify touch different
fields of the same node and merge cleanly (Stage 1), with Stage 2 never
firing because the moved node's new parent is present.

**Patches** form a **groupoid**: every patch has an inverse (`invert`),
composition is associative, and the identity is the empty patch. The laws
are verified by proptest (2000 random cases each).

On a flat tree the topology is discrete, sheafification is the identity,
and the merge specialises to the classical per-field pushout — the new
content is exactly what the ancestry topology buys.

The full treatment — definitions, theorems (Sheaf Pushout Correctness,
Sheaf Consistency, Strict Refinement), and proofs — is in
[`paper/main.tex`](paper/main.tex).

## Design

- **Self-contained.** Zero external dependencies beyond `serde` (optional).
- **Identity-keyed.** The fundamental data structure is
  `BTreeMap<Identity, Node>`, not `BTreeMap<Path, Value>`.
- **Sheaf on the tree space.** Merge is the sheaf pushout; output is always
  a consistent section (referential integrity enforced by sheafification).
- **VCS kernel.** Content-addressed storage + commit DAG + pushout merge.
- **Tested.** 60 unit + 13 property + 12 CLI + 5 doctests = 90 total.

## Limitations

| Gap | Impact |
|-----|--------|
| **No move detection in `tree_diff`.** A moved node (same identity, new parent) is reported as remove + add. The section-level diff (`patch::diff`) does capture move as a `parent` field change. | Display diff is coarser than the algebra. |
| **Keyless siblings collide.** `to_section` keys a node by its `kind` when no identity is set; same-kind keyless siblings overwrite each other silently. Assigning stable identities is the caller's responsibility. | Arrays / positional lists need an external keying step. |
| **Snapshot-based merge.** Merge works on sections, not on a patch-commutation algebra; cherry-pick/rebase are `apply`-based and less flexible than Pijul's. | Rare in the identity-keyed setting, but a real expressiveness gap. |
| **`order` integrity not enforced.** Sheafification checks parent integrity only; duplicate or non-contiguous `order` values among siblings are not flagged. | Future sheaf constraint. |

## License

MIT
