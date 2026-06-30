//! Versioned change sets — diff results with metadata for audit and tracking.
//!
//! A [`ChangeSet`] wraps a tate diff result with version metadata (source
//! hashes, timestamp, author, note). It is a plain data structure — no I/O,
//! no history management, no state. Callers are responsible for storing
//! snapshots and managing version history; tate only answers "what changed
//! between these two inputs?"
//!
//! With the `serde` feature, `ChangeSet` serializes to JSON, making it suitable
//! for cross-language pipelines (Python, R, CLI tools).
//!
//! ```
//! use tate::change::ChangeSet;
//! use tate::lines::diff;
//!
//! let ops = diff(&["a", "b", "c"], &["a", "x", "c"]);
//! let cs = ChangeSet::new_lines(ops, "v1", "v2");
//! assert_eq!(cs.stats.additions + cs.stats.deletions, 2);
//! assert_eq!(cs.from_version, "v1");
//! ```

use crate::grid::GridDiff;
use crate::inline::{Op, Stats, stats};
use crate::tree::TreeDiff;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Which tate algorithm produced this change set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum ChangeKind {
    Line,
    Grid,
    Tree,
}

/// The diff payload — one variant per tate algorithm family.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ChangeOps {
    Lines(Vec<Op>),
    Grid(GridDiff),
    Tree(TreeDiff),
}

impl ChangeOps {
    pub fn kind(&self) -> ChangeKind {
        match self {
            ChangeOps::Lines(_) => ChangeKind::Line,
            ChangeOps::Grid(_) => ChangeKind::Grid,
            ChangeOps::Tree(_) => ChangeKind::Tree,
        }
    }

    pub fn stats(&self) -> Stats {
        match self {
            ChangeOps::Lines(ops) => stats(ops),
            ChangeOps::Grid(gd) => Stats {
                additions: gd.added_rows,
                deletions: gd.removed_rows,
                modified: gd.modified_rows,
                unchanged: gd.rows.iter().filter(|r| r.status == crate::grid::Status::Equal).count(),
            },
            ChangeOps::Tree(td) => {
                use crate::tree::ChangeKind;
                let mut s = Stats::default();
                for c in &td.changes {
                    match c.kind {
                        ChangeKind::Added => s.additions += 1,
                        ChangeKind::Removed => s.deletions += 1,
                        ChangeKind::Modified => s.modified += 1,
                    }
                }
                s
            }
        }
    }
}

/// A versioned diff result with metadata for audit and tracking.
///
/// Plain data — no I/O, no state. Callers store snapshots and manage history;
/// `ChangeSet` is the answer to "what changed between these two versions?"
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ChangeSet {
    /// Which algorithm produced the diff payload.
    pub kind: ChangeKind,
    /// The diff result itself.
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
    /// Create a `ChangeSet` from a line-sequence diff.
    pub fn new_lines(ops: Vec<Op>, from: impl Into<String>, to: impl Into<String>) -> Self {
        let stats = stats(&ops);
        ChangeSet {
            kind: ChangeKind::Line,
            stats,
            ops: ChangeOps::Lines(ops),
            from_version: from.into(),
            to_version: to.into(),
            timestamp: now_unix(),
            author: None,
            note: None,
        }
    }

    /// Create a `ChangeSet` from a grid (2D table) diff.
    pub fn new_grid(diff: GridDiff, from: impl Into<String>, to: impl Into<String>) -> Self {
        let stats = ChangeOps::Grid(diff.clone()).stats();
        ChangeSet {
            kind: ChangeKind::Grid,
            stats,
            ops: ChangeOps::Grid(diff),
            from_version: from.into(),
            to_version: to.into(),
            timestamp: now_unix(),
            author: None,
            note: None,
        }
    }

    /// Create a `ChangeSet` from a tree (structural) diff.
    pub fn new_tree(diff: TreeDiff, from: impl Into<String>, to: impl Into<String>) -> Self {
        let stats = ChangeOps::Tree(diff.clone()).stats();
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
    use crate::lines::diff;
    use crate::inline::pair_replacements;

    #[test]
    fn line_changeset_basic() {
        let ops = diff(&["a", "b", "c"], &["a", "x", "c"]);
        let cs = ChangeSet::new_lines(ops, "v1", "v2");
        assert_eq!(cs.kind, ChangeKind::Line);
        assert_eq!(cs.from_version, "v1");
        assert_eq!(cs.to_version, "v2");
        assert!(!cs.is_clean());
        assert_eq!(cs.stats.deletions, 1);
        assert_eq!(cs.stats.additions, 1);
    }

    #[test]
    fn line_changeset_clean() {
        let ops = diff(&["a", "b"], &["a", "b"]);
        let cs = ChangeSet::new_lines(ops, "abc123", "def456");
        assert!(cs.is_clean());
        assert_eq!(cs.stats.unchanged, 2);
    }

    #[test]
    fn line_changeset_with_replace() {
        let ops = pair_replacements(diff(&["foo bar"], &["foo baz"]), 0.5);
        let cs = ChangeSet::new_lines(ops, "r1", "r2");
        assert_eq!(cs.stats.modified, 1);
        assert!(!cs.is_clean());
    }

    #[test]
    fn grid_changeset_basic() {
        use crate::grid::{grid_diff, GridOptions};
        let a = vec![vec!["A".into(), "1".into()], vec!["B".into(), "2".into()]];
        let b = vec![vec!["A".into(), "1".into()], vec!["B".into(), "3".into()]];
        let gd = grid_diff(&a, &b, &GridOptions::default());
        let cs = ChangeSet::new_grid(gd, "sheet-v1", "sheet-v2");
        assert_eq!(cs.kind, ChangeKind::Grid);
        assert_eq!(cs.from_version, "sheet-v1");
        assert!(!cs.is_clean());
        assert_eq!(cs.stats.modified, 1);
    }

    #[test]
    fn tree_changeset_basic() {
        use crate::tree::{tree_diff, TreeNode};
        let a = TreeNode::new("root").with_attr("version", "1");
        let b = TreeNode::new("root").with_attr("version", "2");
        let td = tree_diff(&a, &b);
        let cs = ChangeSet::new_tree(td, "config-v1", "config-v2");
        assert_eq!(cs.kind, ChangeKind::Tree);
        assert!(!cs.is_clean());
        assert_eq!(cs.stats.modified, 1);
    }

    #[test]
    fn changeset_ops_kind_matches() {
        let line_cs = ChangeSet::new_lines(vec![], "a", "b");
        assert_eq!(line_cs.ops.kind(), ChangeKind::Line);

        let grid_cs = ChangeSet::new_grid(GridDiff::default(), "a", "b");
        assert_eq!(grid_cs.ops.kind(), ChangeKind::Grid);

        let tree_cs = ChangeSet::new_tree(TreeDiff::default(), "a", "b");
        assert_eq!(tree_cs.ops.kind(), ChangeKind::Tree);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn line_changeset_serde_roundtrip() {
        let ops = diff(&["a", "b", "c"], &["a", "x", "c"]);
        let cs = ChangeSet::new_lines(ops, "v1", "v2");
        let json = serde_json::to_string(&cs).unwrap();
        let back: ChangeSet = serde_json::from_str(&json).unwrap();
        assert_eq!(back.from_version, "v1");
        assert_eq!(back.to_version, "v2");
        assert_eq!(back.kind, ChangeKind::Line);
        assert_eq!(back.stats.additions, 1);
        assert_eq!(back.stats.deletions, 1);
    }
}