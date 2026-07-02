//! Tate: a self-contained structured diff library for Rust.
//!
//! Six facilities for diffing structured data, all with zero external
//! diff-engine dependencies:
//!
//! - [`lines`] — a production line-diff engine (patience anchors + LCS +
//!   Hirschberg linear-space + block-replacement bailout). Accepts any
//!   `&[impl AsRef<str>]`.
//! - [`inline`] — pairs delete+insert blocks into `Replace` ops carrying
//!   word-level inline segments. Also provides [`inline::stats`] for diff
//!   statistics.
//! - [`grid`] — aligns two 2D grids of strings (rows × columns) and emits one
//!   aligned grid with per-cell status. Works against arbitrary
//!   `&[Vec<String>]` (Excel, CSV, HTML tables, SQL result sets, …).
//! - [`tree`] — structural diff of two [`tree::TreeNode`]s (a format-agnostic
//!   intermediate representation). Callers convert from their format (XML, JSON,
//!   YAML, …) into `TreeNode` before calling `tree_diff`.
//! - [`unified`] — render an edit script as unified diff text (`git diff`
//!   style) with hunks and context lines.
//! - [`grid`] — 2D grid alignment (row/column coordinate descent). Produces the
//!   [`grid::GridDiff`] display result; also serves as a keying adapter that
//!   turns an un-keyed grid into a stably-keyed tree for the merge algebra.
//! - [`tree`] — structural tree diff and 3-way tree merge (the sole merge).
//! - [`section`] — the canonical object: a [`section::Section`] is the flat
//!   `location → value` form of a tree (the sheaf section the algebra runs on).
//!   Convert with [`tree::TreeNode::to_section`] / [`section::Section::to_tree`].
//! - [`patch`] — lossless patch algebra over sections: `diff` / `apply` /
//!   `invert` / `compose`, the morphisms of the versioned-structure category,
//!   with laws verified by proptest.
//! - [`change`] — versioned change sets: diff results with metadata (version
//!   labels, timestamp, author) for audit and cross-language pipelines.
//!
//! Typical pipeline for text file diff:
//! ```
//! use tate::lines::diff;
//! use tate::inline::{pair_replacements, OpType, DEFAULT_SIMILARITY};
//!
//! let a: Vec<String> = vec!["hello world".into(), "foo bar".into()];
//! let b: Vec<String> = vec!["hello world".into(), "foo baz".into()];
//! let ops = diff(&a, &b);
//! let paired = pair_replacements(ops, DEFAULT_SIMILARITY);
//! assert_eq!(paired[1].typ, OpType::Replace);
//! ```
//!
//! Unified diff output:
//! ```
//! use tate::lines::diff;
//! use tate::unified::to_unified;
//!
//! let ops = diff(&["a", "b", "c"], &["a", "x", "c"]);
//! let text = to_unified(&ops, 3);
//! assert!(text.contains("@@"));
//! ```
//!
//! 3-way merge (the single merge, over trees):
//! ```
//! use tate::tree::{TreeNode, tree_merge};
//!
//! let base = TreeNode::new("root").with_child(TreeNode::new("e").with_identity("u1").with_attr("v", "1"));
//! let ours = TreeNode::new("root").with_child(TreeNode::new("e").with_identity("u1").with_attr("v", "9"));
//! let theirs = base.clone();
//! let result = tree_merge(&base, &ours, &theirs);
//! assert_eq!(result.conflicts.len(), 0);
//! assert_eq!(result.tree.children[0].attr("v"), Some("9"));
//! ```

pub mod change;
pub mod grid;
pub mod inline;
pub mod lines;
pub mod patch;
pub mod section;
pub mod tree;
pub mod unified;

mod lcs;
