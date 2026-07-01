//! Unified patch framework: diff, merge, and conflict types.
//!
//! In the groupoid of versioned structures, objects are data states and
//! morphisms are diffs produced by tate's algorithms. A 3-way merge computes
//! the pushout of two morphisms from a common base.
//!
//! - **Lines**: objects are `Vec<String>`, morphisms are `Vec<Op>`.
//!   [`crate::merge::merge`] computes the pushout.
//! - **Grids**: objects are `Vec<Vec<String>>`, morphisms are [`crate::grid::GridDiff`].
//!   [`crate::grid::grid_merge`] computes the pushout.
//! - **Trees**: objects are [`crate::tree::TreeNode`], morphisms are [`crate::tree::TreeDiff`].
//!   [`crate::tree::tree_merge`] computes the pushout.
//!
//! When the pushout is unique (up to isomorphism), the merge is automatic.
//! When multiple non-isomorphic pushouts exist, the conflict is recorded.
//!
//! ```
//! use tate::merge::merge;
//! use tate::grid::{grid_merge, GridOptions};
//! use tate::tree::{TreeNode, tree_merge};
//!
//! // Line merge
//! let r = merge(&["a", "b", "c"], &["a", "X", "c"], &["a", "b", "Y"]);
//! assert_eq!(r.conflicts, 0);
//!
//! // Grid merge
//! let base = vec![vec!["1".into(), "2".into()]];
//! let ours = vec![vec!["1".into(), "9".into()]];
//! let theirs = vec![vec!["1".into(), "2".into()]];
//! let r = grid_merge(&base, &ours, &theirs, &GridOptions::default());
//! assert_eq!(r.conflicts.len(), 0);
//!
//! // Tree merge
//! let t = TreeNode::new("root");
//! let r = tree_merge(&t, &t, &t);
//! assert_eq!(r.conflicts.len(), 0);
//! ```

/// A generic merge result carrying the merged value and conflicts.
///
/// This is the common shape shared by all three merge implementations:
/// - Line merge returns [`crate::merge::MergeOutcome`] (conflicts inline).
/// - Grid merge returns [`crate::grid::GridMergeResult`] (cell-level conflicts).
/// - Tree merge returns [`crate::tree::TreeMergeResult`] (node-level conflicts).
pub struct MergeResult<T, C> {
    pub merged: T,
    pub conflicts: Vec<C>,
}
