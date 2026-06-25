//! Tate: structured diff primitives for Rust.
//!
//! Three format-agnostic facilities for diffing structured data. Each consumes
//! inputs as plain Rust values (`&str`, `&[Vec<String>]`, XML) and returns an
//! aligned, change-tagged result that a UI can render with cell- or
//! word-level highlights — no knowledge of any particular file format or
//! domain schema is built in.
//!
//! - [`inline`] pairs delete+insert blocks produced by a line-diff engine
//!   (`similar`, `imara-diff`, a hand-rolled Myers, …) into `Replace` ops
//!   carrying word-level segments, so a line that turned `foo bar` into
//!   `foo baz` is one row highlighting only `baz`, not a noisy delete+insert.
//! - [`grid`] aligns two 2D grids of strings (rows × columns) and emits one
//!   aligned grid with per-cell status: equal / modified / added / removed.
//!   It works against arbitrary `&[Vec<String>]` inputs and has no opinion
//!   about how those rows were produced (Excel, CSV, HTML tables, SQL result
//!   sets, parsed log tables, …).
//! - [`tree`] does a structural diff of two XML documents keyed by identity
//!   attributes, emitting `added | removed | modified` changes per node with
//!   no schema-specific semantics. Callers layer domain semantics on top of
//!   [`tree::TreeChange`] for their own schema (CDISC, BPMN, Maven POM, SVG,
//!   …) via [`tree::TreeOptions`].
//!
//! The line-diff itself (Myers, patience, LCS, Hirschberg, …) is the caller's
//! responsibility — tate consumes their edit scripts (via
//! [`inline::pair_replacements`]) and runs its own small word-level LCS diff
//! (via [`inline::inline_segments`]) for single-line highlights. No external
//! diff engine is required.

pub mod grid;
pub mod inline;
pub mod tree;

mod lcs;