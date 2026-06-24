//! Tate: structured diff primitives.
//!
//! Three independent facilities for diffing structured data. Each consumes the
//! raw edit script a line-diff engine produces and builds a richer, structure-aware
//! output that a UI can render with cell- or word-level highlights.
//!
//! - [`inline`] pairs delete+insert blocks into Replace ops carrying word-level
//!   inline segments, so a line that turned `foo bar` into `foo baz` is one row
//!   highlighting only `baz`, not a noisy delete+insert pair.
//! - [`grid`] aligns two 2D grids of strings (rows × columns) and emits a single
//!   aligned grid with per-cell status: equal / modified / added / removed. It
//!   works against arbitrary `&[Vec<String>]` inputs and has no knowledge of how
//!   those rows were produced (Excel, CSV, RTF, SQL results, …).
//! - [`tree`] does a structural diff of two tree documents (XML today, JSON[K]V
//!   tomorrow) keyed by identity attributes, emitting `added | removed | modified`
//!   changes per node with no schema-specific semantics.
//!
//! The line-diff itself (Myers / patience / LCS) is delegated to external crates
//! such as `similar` or `imara-diff` — Tate only consumes their edit scripts
//! (via [`inline::pair_replacements`]) or runs its own internal line-diff when it
//! needs word-level alignment inside one pair.

pub mod grid;
pub mod inline;
pub mod tree;