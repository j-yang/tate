//! Versioned change sets — a tree diff or patch with metadata for audit and tracking.
//!
//! A [`ChangeSet`] wraps a tate result with version metadata (source labels or
//! hashes, timestamp, author, note). It is a plain data structure — no I/O, no
//! history management, no state. Callers are responsible for storing snapshots
//! and managing version history; tate only answers "what changed between these
//! two inputs?"
//!
//! With the `serde` feature, `ChangeSet` serializes to JSON, making it suitable
//! for cross-language pipelines (Python, R, CLI tools).
//!
//! ```
//! use tate::change::ChangeSet;
//! use tate::tree::{tree_diff, TreeNode};
//!
//! let a = TreeNode::new("root").with_attr("version", "1");
//! let b = TreeNode::new("root").with_attr("version", "2");
//! let cs = ChangeSet::new_tree(tree_diff(&a, &b), "v1", "v2");
//! assert_eq!(cs.stats.modified, 1);
//! assert_eq!(cs.from_version, "v1");
//! ```

use crate::patch::Patch;
use crate::tree::{ChangeKind as TreeChangeKind, TreeDiff};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Summary statistics for a change set.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Stats {
    pub additions: usize,
    pub deletions: usize,
    pub modified: usize,
    pub unchanged: usize,
}

impl Stats {
    /// `true` when the diff contains no changes.
    pub fn is_clean(&self) -> bool {
        self.additions + self.deletions + self.modified == 0
    }
}

/// Which tate result produced this change set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum ChangeKind {
    /// A structural tree diff (display-oriented).
    Tree,
    /// A lossless patch (round-trippable).
    Patch,
}

/// The payload — either a display tree diff or a lossless patch.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ChangeOps {
    Tree(TreeDiff),
    Patch(Patch),
}

impl ChangeOps {
    pub fn kind(&self) -> ChangeKind {
        match self {
            ChangeOps::Tree(_) => ChangeKind::Tree,
            ChangeOps::Patch(_) => ChangeKind::Patch,
        }
    }

    pub fn stats(&self) -> Stats {
        match self {
            ChangeOps::Tree(td) => tree_stats(td),
            ChangeOps::Patch(p) => patch_stats(p),
        }
    }
}

/// Count added/removed/modified nodes in a tree diff.
fn tree_stats(td: &TreeDiff) -> Stats {
    let mut s = Stats::default();
    for c in &td.changes {
        match c.kind {
            TreeChangeKind::Added => s.additions += 1,
            TreeChangeKind::Removed => s.deletions += 1,
            TreeChangeKind::Modified => s.modified += 1,
        }
    }
    s
}

/// Count point edits in a patch by whether they add (⊥→v), remove (v→⊥), or
/// modify (v→w) a location's value.
fn patch_stats(p: &Patch) -> Stats {
    let mut s = Stats::default();
    for edit in p.edits.values() {
        match (&edit.old, &edit.new) {
            (None, Some(_)) => s.additions += 1,
            (Some(_), None) => s.deletions += 1,
            _ => s.modified += 1,
        }
    }
    s
}

/// A versioned diff result with metadata for audit and tracking.
///
/// Plain data — no I/O, no state. Callers store snapshots and manage history;
/// `ChangeSet` is the answer to "what changed between these two versions?"
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ChangeSet {
    /// Which kind of result the payload holds.
    pub kind: ChangeKind,
    /// The diff or patch itself.
    pub ops: ChangeOps,
    /// Summary statistics derived from `ops`.
    pub stats: Stats,
    /// Version label or hash for the source (A) side.
    pub from_version: String,
    /// Version label or hash for the target (B) side.
    pub to_version: String,
    /// Unix timestamp (seconds) when this change set was created.
    pub timestamp: u64,
    /// Optional author identifier.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub author: Option<String>,
    /// Optional human-readable note.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub note: Option<String>,
}

impl ChangeSet {
    /// Create a `ChangeSet` from a structural tree diff.
    pub fn new_tree(diff: TreeDiff, from: impl Into<String>, to: impl Into<String>) -> Self {
        let stats = tree_stats(&diff);
        ChangeSet {
            kind: ChangeKind::Tree,
            stats,
            ops: ChangeOps::Tree(diff),
            from_version: from.into(),
            to_version: to.into(),
            timestamp: now_unix(),
            author: None,
            note: None,
        }
    }

    /// Create a `ChangeSet` from a lossless patch.
    pub fn new_patch(patch: Patch, from: impl Into<String>, to: impl Into<String>) -> Self {
        let stats = patch_stats(&patch);
        ChangeSet {
            kind: ChangeKind::Patch,
            stats,
            ops: ChangeOps::Patch(patch),
            from_version: from.into(),
            to_version: to.into(),
            timestamp: now_unix(),
            author: None,
            note: None,
        }
    }

    /// Returns `true` when the diff contains no changes.
    pub fn is_clean(&self) -> bool {
        self.stats.is_clean()
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::diff as patch_diff;
    use crate::tree::{tree_diff, TreeNode};

    #[test]
    fn tree_changeset_basic() {
        let a = TreeNode::new("root").with_attr("version", "1");
        let b = TreeNode::new("root").with_attr("version", "2");
        let cs = ChangeSet::new_tree(tree_diff(&a, &b), "config-v1", "config-v2");
        assert_eq!(cs.kind, ChangeKind::Tree);
        assert_eq!(cs.from_version, "config-v1");
        assert!(!cs.is_clean());
        assert_eq!(cs.stats.modified, 1);
    }

    #[test]
    fn tree_changeset_clean() {
        let a = TreeNode::new("root").with_child(TreeNode::new("e").with_identity("u1"));
        let cs = ChangeSet::new_tree(tree_diff(&a, &a), "v1", "v1");
        assert!(cs.is_clean());
    }

    #[test]
    fn patch_changeset_counts_add_remove_modify() {
        let base = TreeNode::new("root")
            .with_child(TreeNode::new("a").with_identity("u1").with_attr("v", "1"))
            .with_child(TreeNode::new("b").with_identity("u2"));
        let target = TreeNode::new("root")
            .with_child(TreeNode::new("a").with_identity("u1").with_attr("v", "9")) // modify
            .with_child(TreeNode::new("c").with_identity("u3")); // u2 removed, u3 added
        let p = patch_diff(&base, &target);
        let cs = ChangeSet::new_patch(p, "v1", "v2");
        assert_eq!(cs.kind, ChangeKind::Patch);
        assert!(!cs.is_clean());
        assert!(cs.stats.additions >= 1 && cs.stats.deletions >= 1 && cs.stats.modified >= 1);
    }

    #[test]
    fn changeset_ops_kind_matches() {
        let tree_cs = ChangeSet::new_tree(TreeDiff::default(), "a", "b");
        assert_eq!(tree_cs.ops.kind(), ChangeKind::Tree);
        let patch_cs = ChangeSet::new_patch(Patch::empty(), "a", "b");
        assert_eq!(patch_cs.ops.kind(), ChangeKind::Patch);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn tree_changeset_serde_roundtrip() {
        let a = TreeNode::new("root").with_attr("version", "1");
        let b = TreeNode::new("root").with_attr("version", "2");
        let cs = ChangeSet::new_tree(tree_diff(&a, &b), "v1", "v2");
        let json = serde_json::to_string(&cs).unwrap();
        let back: ChangeSet = serde_json::from_str(&json).unwrap();
        assert_eq!(back.from_version, "v1");
        assert_eq!(back.kind, ChangeKind::Tree);
        assert_eq!(back.stats.modified, 1);
    }
}
