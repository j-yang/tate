//! Tate: a pure structured diff, patch, and merge algebra for Rust.
//!
//! tate rests on one commitment: **every structure is a tree**, and a tree is a
//! *section* of a `location → value` sheaf — the flat map from each addressable
//! location to the value living there. Diff, 3-way merge, and a lossless patch
//! algebra are defined **once**, on that single object. There is no per-format
//! machinery here: turning bytes (XML, JSON, spreadsheets, text) into a tree —
//! and, for un-keyed data, computing the alignment that gives it stable
//! identities — lives in a separate layer (the `mumford` crate). tate has zero
//! format-parsing and zero diff-engine dependencies (only optional `serde`).
//!
//! - [`section`] — the canonical object: a [`section::Section`] is the flat
//!   `location → value` form of a tree. Identity is the location; structural
//!   position (`order`) and scalar content are values, so a moved or renamed
//!   node is a value change, not a delete+add. Convert with
//!   [`tree::TreeNode::to_section`] / [`section::Section::to_tree`].
//! - [`patch`] — the lossless patch algebra over sections: `diff` / `apply` /
//!   `invert` / `compose`, the morphisms of the versioned-structure groupoid,
//!   plus [`patch::merge_sections`] (3-way) and [`patch::merge_sections_nway`]
//!   (N-branch) — the merge realised as the exact **pushout** of the span
//!   `ours ← base → theirs`, computed point-wise on the
//!   [`section::Section`]. Unlike [`tree::tree_diff`] (a lossy display diff) the
//!   patch algebra round-trips; its laws (including the pushout construction)
//!   are verified by proptest.
//! - [`tree`] — the nested [`tree::TreeNode`] view, its structural
//!   [`tree::tree_diff`], and [`tree::tree_merge`] — the display-oriented 3-way
//!   merge. It agrees with [`patch::merge_sections`] on *where* conflicts occur,
//!   but reports each gluing obstruction (attribute, text, add/add,
//!   modify/delete) with tree-level detail as a [`tree::TreeConflict`], for UIs.
//!   Both merges are total: they always return a best-effort result.
//! - [`change`] — versioned change sets: a tree diff or patch tagged with
//!   metadata (version labels, timestamp, author) for audit and cross-language
//!   pipelines.
//! - [`repo`] — a version control kernel: content-addressed sections, commit DAG,
//!   merge (pushout), cherry-pick, revert, branches.
//!
//! Diff two trees:
//! ```
//! use tate::tree::{TreeNode, tree_diff, ChangeKind};
//!
//! let a = TreeNode::new("root").with_child(TreeNode::new("e").with_identity("u1").with_attr("v", "1"));
//! let b = TreeNode::new("root").with_child(TreeNode::new("e").with_identity("u1").with_attr("v", "2"));
//! let d = tree_diff(&a, &b);
//! assert_eq!(d.changes[0].kind, ChangeKind::Modified);
//! ```
//!
//! Lossless patch round-trip:
//! ```
//! use tate::tree::TreeNode;
//! use tate::patch::{diff, apply};
//!
//! let a = TreeNode::new("root").with_child(TreeNode::new("x").with_identity("1"));
//! let b = TreeNode::new("root")
//!     .with_child(TreeNode::new("x").with_identity("1"))
//!     .with_child(TreeNode::new("y").with_identity("2"));
//! assert_eq!(apply(&diff(&a, &b), &a).unwrap(), b);
//! ```
//!
//! 3-way merge (the single merge):
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
pub mod patch;
pub mod repo;
pub mod section;

/// serde helpers for the location-keyed maps. A [`section::Location`] is a
/// `Vec<String>`, and JSON object keys must be strings — so the derived map
/// serialization ("key must be a string") fails for any non-self-describing or
/// JSON format. These (de)serialize such maps as a sequence of `[key, value]`
/// pairs instead, keeping the in-memory type a `BTreeMap` and the ordering
/// canonical while making `Patch`/`Section` round-trip through JSON.
#[cfg(feature = "serde")]
pub(crate) mod loc_map_serde {
    use serde::de::{Deserialize, Deserializer};
    use serde::ser::{Serialize, Serializer};
    use std::collections::BTreeMap;

    pub fn serialize<K, V, S>(map: &BTreeMap<K, V>, ser: S) -> Result<S::Ok, S::Error>
    where
        K: Serialize + Ord,
        V: Serialize,
        S: Serializer,
    {
        // As a sequence of (key, value) pairs — valid in every serde format.
        ser.collect_seq(map.iter())
    }

    pub fn deserialize<'de, K, V, D>(de: D) -> Result<BTreeMap<K, V>, D::Error>
    where
        K: Deserialize<'de> + Ord,
        V: Deserialize<'de>,
        D: Deserializer<'de>,
    {
        let pairs: Vec<(K, V)> = Vec::deserialize(de)?;
        Ok(pairs.into_iter().collect())
    }
}
pub mod tree;
