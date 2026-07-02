# tate

A self-contained structured diff, patch, and merge library for Rust — one tree algebra (diff / 3-way merge / lossless patch) plus keying adapters (line diff, grid alignment) and inline word-level highlighting.

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

## Overview

Tate provides several facilities for diffing structured data, all with zero external diff-engine dependencies:

- **`lines`** — A production line-diff engine (patience anchors + LCS + Hirschberg linear-space + block-replacement bailout). Produces `Equal | Delete | Insert` ops; feed the result into `inline` to get `Replace` rows with word-level highlights.

- **`inline`** — Pairs delete+insert blocks into `Replace` ops carrying word-level inline segments. A line that turned `foo bar` into `foo baz` becomes one row highlighting only `baz`, not a noisy delete+insert pair.

- **`grid`** — Aligns two 2D grids of strings (rows × columns) via row/column coordinate descent and emits one aligned grid with per-cell status (`equal` / `modified` / `added` / `removed`). Works against arbitrary `&[Vec<String>]` inputs. Doubles as a keying adapter: the alignment it computes gives an un-keyed grid stable identities so it can be merged as a tree.

- **`tree`** — Structural diff of two `TreeNode`s (a format-agnostic intermediate representation). Callers convert from their format (XML, JSON, YAML, …) into `TreeNode` before calling `tree_diff`. Zero format-parsing dependencies. Also provides `tree_merge` — tate's **single** 3-way merge — which records every gluing obstruction (attribute, text, add/add, modify/delete) as a `TreeConflict`.

- **`section`** — The canonical object: a `Section` is the flat `location → value` form of a tree (the sheaf section the algebra runs on). Convert with `TreeNode::to_section` / `Section::to_tree`. Identity is the location; structural position (`order`) and scalar content are values, so moves and renames are value changes, not delete+add.

- **`patch`** — A lossless patch algebra over sections: `diff` / `apply` / `invert` / `compose`, the morphisms of the versioned-structure category. Unlike `tree_diff` (a lossy display diff), it round-trips. The laws (`apply(diff(a, b), a) == b`, `invert` undoes `apply`, `compose` equals sequential `apply`, and associativity) are verified by `proptest`.

## Usage

```toml
[dependencies]
tate = "0.1"
```

### Complete text diff pipeline

```rust
use tate::lines::diff;
use tate::inline::{pair_replacements, OpType, DEFAULT_SIMILARITY};

let a = vec!["hello world".into(), "foo bar".into()];
let b = vec!["hello world".into(), "foo baz".into()];

let ops = diff(&a, &b);
let paired = pair_replacements(ops, DEFAULT_SIMILARITY);
assert_eq!(paired[1].typ, OpType::Replace);
```

### 2D grid alignment

```rust
use tate::grid::{grid_diff, GridOptions};

let a = vec![
    vec!["name".into(), "amount".into()],
    vec!["Alice".into(), "100".into()],
];
let b = vec![
    vec!["name".into(), "amount".into()],
    vec!["Alice".into(), "250".into()],
];

let diff = grid_diff(&a, &b, &GridOptions::default());
assert_eq!(diff.modified_rows, 1);
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

## Design

- **Self-contained.** Zero external dependencies beyond `serde` (optional, behind the default `serde` feature). No `similar`, no `roxmltree`, no `imara-diff`.

- **Format-agnostic.** `grid_diff` accepts `&[Vec<String>]`. `tree_diff` operates on `TreeNode` — callers convert from XML, JSON, YAML, or any tree format. tate has no file-format knowledge.

- **Configurable.** Every heuristic (header detection ratio, row similarity threshold, LCS row budget) is exposed via `GridOptions` with sensible defaults.

- **Tested.** 42 unit tests + 3 doctests covering edge cases (empty inputs, inserted rows/columns, keyless node bubbling, budget fallback, root tag rename detection, asymmetric grid widths, 300K-line diff without OOM).

## License

MIT