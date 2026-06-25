# tate

Structured diff primitives for Rust — grid alignment, tree diff, and inline replacement pairing.

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

## Overview

Tate provides three format-agnostic facilities for diffing structured data. Each consumes plain Rust values (`&str`, `&[Vec<String>]`, XML) and returns an aligned, change-tagged result that a UI can render with cell- or word-level highlights. No knowledge of any particular file format or domain schema is built in.

- **`inline`** — Pairs delete+insert blocks produced by any line-diff engine into `Replace` ops carrying word-level inline segments. A line that turned `foo bar` into `foo baz` becomes one row highlighting only `baz`, not a noisy delete+insert pair.

- **`grid`** — Aligns two 2D grids of strings (rows × columns) and emits one aligned grid with per-cell status (`equal` / `modified` / `added` / `removed`). Works against arbitrary `&[Vec<String>]` inputs — Excel, CSV, HTML tables, SQL result sets, parsed log tables.

- **`tree`** — Structural diff of two XML documents keyed by identity attributes, emitting `added` / `removed` / `modified` changes per node. Schema-agnostic: callers configure which attributes are identity-bearing via `TreeOptions`.

The line-diff itself (Myers, patience, LCS, Hirschberg, …) is the caller's responsibility — tate consumes their edit scripts via `inline::pair_replacements` and runs its own small word-level LCS diff via `inline::inline_segments` for single-line highlights. No external diff engine is required.

## Usage

```toml
[dependencies]
tate = "0.1"
```

### Inline highlights from any line-diff

```rust
use tate::inline::{pair_replacements, Op, OpType, DEFAULT_SIMILARITY};

// Your line-diff engine produces Equal/Delete/Insert ops:
let raw = vec![
    Op::delete(1, "Section A.1 Overview .... 17"),
    Op::insert(1, "Section A.1 Overview .... 18"),
];

// tate pairs similar delete+insert blocks into Replace with word-level segments:
let ops = pair_replacements(raw, DEFAULT_SIMILARITY);
assert_eq!(ops.len(), 1);
assert_eq!(ops[0].typ, OpType::Replace);
```

### 2D grid alignment

```rust
use tate::grid::{grid_diff, GridOptions};

let a = vec![
    vec!["name".into(), "amount".into()],
    vec!["Alice".into(), "100".into()],
    vec!["Bob".into(), "200".into()],
];
let b = vec![
    vec!["name".into(), "amount".into()],
    vec!["Alice".into(), "100".into()],
    vec!["Bob".into(), "250".into()],
];

let diff = grid_diff(&a, &b, &GridOptions::default());
assert_eq!(diff.modified_rows, 1);
```

### XML tree diff

```rust
use tate::tree::tree_diff;

let a = r#"<root><entry id="u1" name="alice" level="1"/></root>"#;
let b = r#"<root><entry id="u1" name="alice" level="99"/></root>"#;

let diff = tree_diff(a, b).unwrap();
assert_eq!(diff.changes.len(), 1);
assert_eq!(diff.changes[0].kind, tate::tree::ChangeKind::Modified);
```

### Custom identity attributes

```rust
use tate::tree::{tree_diff_with, TreeOptions};

let opts = TreeOptions { identity_attrs: vec!["ref".into()] };
let a = r#"<doc><node ref="x" value="1"/></doc>"#;
let b = r#"<doc><node ref="x" value="9"/></doc>"#;

let diff = tree_diff_with(a, b, &opts).unwrap();
assert_eq!(diff.changes.len(), 1);
```

## Design

- **Self-contained.** Zero external diff-engine dependencies. The only crate dependencies are `roxmltree` (XML parsing for `tree`) and `serde` (optional, behind the default `serde` feature).

- **Format-agnostic.** `grid_diff` accepts `&[Vec<String>]` — it doesn't know whether rows came from Excel, CSV, or a database query. `tree_diff` outputs generic `TreeChange` with no schema-specific fields.

- **Configurable.** Every heuristic (header detection ratio, row similarity threshold, LCS row budget, identity attributes) is exposed via `GridOptions` / `TreeOptions` with sensible defaults.

- **Tested.** 37 unit tests covering edge cases (empty inputs, inserted rows/columns, keyless node bubbling, budget fallback, custom identity attributes, root tag rename detection, asymmetric grid widths).

## License

MIT