//! tate 2.0: a version control kernel for structured data.
//!
//! tate rests on one commitment: **every structure is a tree**, and a tree is
//! a *section* — an identity-keyed map from each node's identity to its data.
//! Identity is separated from position: a node's `parent` is a field (part of
//! its value), not part of its key. This means **Move is a field-level change**,
//! not a delete+insert, and **Move + Modify merge cleanly**.
//!
//! - [`section`] — the canonical object: [`Section`] maps `Identity → Node`,
//!   where each [`Node`] stores its parent, kind, text, attributes, and order.
//! - [`patch`] — the lossless patch algebra: `diff` / `apply` / `invert` /
//!   `compose`, plus [`patch::merge_sections`] — the field-wise pushout merge.
//!   At each identity, each field is merged independently, so two branches
//!   that change different fields (including Move + Modify) merge cleanly.
//! - [`tree`] — the nested [`TreeNode`] view, its structural `tree_diff`, and
//!   `tree_merge` — the display-oriented merge for UIs.
//! - [`change`] — versioned change sets with metadata for audit pipelines.
//! - [`repo`] — a VCS kernel: content-addressed sections, commit DAG,
//!   merge (pushout), cherry-pick, revert, branches.
//!
//! ```
//! use tate::tree::{TreeNode, tree_diff, ChangeKind};
//!
//! let a = TreeNode::new("root").with_child(TreeNode::new("e").with_identity("u1").with_attr("v", "1"));
//! let b = TreeNode::new("root").with_child(TreeNode::new("e").with_identity("u1").with_attr("v", "2"));
//! let d = tree_diff(&a, &b);
//! assert_eq!(d.changes[0].kind, ChangeKind::Modified);
//! ```

pub mod change;
pub mod patch;
pub mod repo;
pub mod section;
pub mod tree;
