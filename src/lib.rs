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
//! - [`merge`] — 3-way merge: combine changes from two branches that diverged
//!   from a common base, with conflict detection.
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
//! 3-way merge:
//! ```
//! use tate::merge::merge;
//!
//! let result = merge(&["a", "b", "c"], &["a", "X", "c"], &["a", "b", "Y"]);
//! assert_eq!(result.conflicts, 0);
//! assert_eq!(result.lines, vec!["a", "X", "Y"]);
//! ```

pub mod grid;
pub mod inline;
pub mod lines;
pub mod merge;
pub mod tree;
pub mod unified;

mod lcs;
