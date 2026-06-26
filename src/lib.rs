//! Tate: a self-contained structured diff library for Rust.
//!
//! Four facilities for diffing structured data, all with zero external
//! diff-engine dependencies:
//!
//! - [`lines`] — a production line-diff engine (patience anchors + LCS +
//!   Hirschberg linear-space + block-replacement bailout). Produces
//!   `Equal | Delete | Insert` ops; feed the result into [`inline`] to get
//!   `Replace` rows with word-level highlights.
//! - [`inline`] — pairs delete+insert blocks into `Replace` ops carrying
//!   word-level inline segments, so a line that turned `foo bar` into
//!   `foo baz` is one row highlighting only `baz`.
//! - [`grid`] — aligns two 2D grids of strings (rows × columns) and emits one
//!   aligned grid with per-cell status. Works against arbitrary
//!   `&[Vec<String>]` (Excel, CSV, HTML tables, SQL result sets, …).
//! - [`tree`] — structural diff of two [`tree::TreeNode`]s (a format-agnostic
//!   intermediate representation). Callers convert from their format (XML, JSON,
//!   YAML, …) into `TreeNode` before calling `tree_diff`. Zero format-parsing
//!   dependencies.
//!
//! Typical pipeline for text-file diff:
//! ```
//! use tate::lines::diff;
//! use tate::inline::{pair_replacements, OpType, DEFAULT_SIMILARITY};
//!
//! let a = vec!["hello world".into(), "foo bar".into()];
//! let b = vec!["hello world".into(), "foo baz".into()];
//! let ops = diff(&a, &b);
//! let paired = pair_replacements(ops, DEFAULT_SIMILARITY);
//! assert_eq!(paired[1].typ, OpType::Replace);
//! ```

pub mod grid;
pub mod inline;
pub mod lines;
pub mod tree;

mod lcs;