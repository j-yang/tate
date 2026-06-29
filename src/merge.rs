//! 3-way merge: combine changes from two branches that diverged from a common
//! base.
//!
//! Given BASE, OURS, and THEIRS, produce a merged result. Regions changed by
//! only one side are taken automatically. Regions changed by both sides are
//! conflicts when the changes differ.
//!
//! ```
//! use tate::merge::{merge, MergeOutcome};
//!
//! let base = ["a", "b", "c"];
//! let ours = ["a", "X", "c"];
//! let theirs = ["a", "b", "c"];
//! let result = merge(&base, &ours, &theirs);
//! assert!(result.conflicts == 0);
//! assert_eq!(result.lines, vec!["a", "X", "c"]);
//! ```

use crate::inline::OpType;
use crate::lines::diff;

/// Result of a 3-way merge.
#[derive(Debug, Clone)]
pub struct MergeOutcome {
    /// Merged lines. Conflict regions contain [`ConflictMarker`] lines.
    pub lines: Vec<String>,
    /// Number of conflict regions found.
    pub conflicts: usize,
}

/// Conflict marker lines inserted into [`MergeOutcome::lines`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictMarker {
    /// `<<<<<<<` — start of "ours" section.
    OursStart,
    /// `=======` — separator between ours and theirs.
    Separator,
    /// `>>>>>>>` — end of "theirs" section.
    TheirsEnd,
}

impl ConflictMarker {
    pub fn render(self, label: &str) -> String {
        match self {
            ConflictMarker::OursStart => format!("<<<<<<< {label}"),
            ConflictMarker::Separator => "=======".to_string(),
            ConflictMarker::TheirsEnd => format!(">>>>>>> {label}"),
        }
    }
}

struct ChangeRegion {
    base_start: usize,
    base_end: usize,
    replacement: Vec<String>,
}

/// Perform a 3-way merge.
///
/// Uses the diff3 algorithm: lines common to all three sequences serve as
/// anchors, partitioning the input into chunks. Each chunk is resolved
/// independently:
/// - Only OURS changed → take OURS
/// - Only THEIRS changed → take THEIRS
/// - Both changed identically → take either
/// - Both changed differently → conflict
pub fn merge<B, O, T>(base: &[B], ours: &[O], theirs: &[T]) -> MergeOutcome
where
    B: AsRef<str>,
    O: AsRef<str>,
    T: AsRef<str>,
{
    let base: Vec<&str> = base.iter().map(AsRef::as_ref).collect();
    let ours: Vec<&str> = ours.iter().map(AsRef::as_ref).collect();
    let theirs: Vec<&str> = theirs.iter().map(AsRef::as_ref).collect();

    let regions_ours = change_regions(&base, &ours);
    let regions_theirs = change_regions(&base, &theirs);

    let mut lines = Vec::new();
    let mut conflicts = 0;
    let mut pos = 0;
    let mut io = 0;
    let mut it = 0;

    while pos < base.len() || io < regions_ours.len() || it < regions_theirs.len() {
        let ro = regions_ours.get(io);
        let rt = regions_theirs.get(it);

        let o_active = ro.is_some_and(|r| r.base_start <= pos);
        let t_active = rt.is_some_and(|r| r.base_start <= pos);

        if o_active && t_active {
            // Both sides changed overlapping regions → potential conflict.
            let o_end = ro.unwrap().base_end;
            let t_end = rt.unwrap().base_end;
            let region_end = o_end.max(t_end);

            // Extend to cover overlapping changes from both sides.
            let mut o_repl = ro.unwrap().replacement.clone();
            let mut t_repl = rt.unwrap().replacement.clone();
            let mut consumed_o = 1;
            let mut consumed_t = 1;

            // Absorb subsequent change regions that fall within the overlap.
            while io + consumed_o < regions_ours.len()
                && regions_ours[io + consumed_o].base_start <= region_end
            {
                let r = &regions_ours[io + consumed_o];
                o_repl.extend(base[r.base_start..r.base_end.min(base.len())].iter().map(|s| s.to_string()));
                o_repl.extend(r.replacement.iter().cloned());
                consumed_o += 1;
            }
            while it + consumed_t < regions_theirs.len()
                && regions_theirs[it + consumed_t].base_start <= region_end
            {
                let r = &regions_theirs[it + consumed_t];
                t_repl.extend(base[r.base_start..r.base_end.min(base.len())].iter().map(|s| s.to_string()));
                t_repl.extend(r.replacement.iter().cloned());
                consumed_t += 1;
            }

            // Also include base lines between the first change and the region end.
            let o_actual_end = if consumed_o > 0 {
                regions_ours[io + consumed_o - 1].base_end
            } else {
                o_end
            };
            let t_actual_end = if consumed_t > 0 {
                regions_theirs[it + consumed_t - 1].base_end
            } else {
                t_end
            };
            let full_end = o_actual_end.max(t_actual_end);

            if o_repl == t_repl {
                lines.extend(o_repl);
            } else {
                lines.push(ConflictMarker::OursStart.render("ours"));
                lines.extend(o_repl);
                lines.push(ConflictMarker::Separator.render(""));
                lines.extend(t_repl);
                lines.push(ConflictMarker::TheirsEnd.render("theirs"));
                conflicts += 1;
            }

            pos = full_end;
            io += consumed_o;
            it += consumed_t;
        } else if o_active {
            let r = &regions_ours[io];
            lines.extend(r.replacement.iter().cloned());
            pos = pos.max(r.base_end);
            io += 1;
        } else if t_active {
            let r = &regions_theirs[it];
            lines.extend(r.replacement.iter().cloned());
            pos = pos.max(r.base_end);
            it += 1;
        } else {
            // Stable region.
            lines.push(base[pos].to_string());
            pos += 1;
        }
    }

    MergeOutcome { lines, conflicts }
}

/// Extract change regions from diff(base, other). Each region describes where
/// `other` differs from `base`, with the replacement lines. `base_start` and
/// `base_end` are 0-based, with `base_start == base_end` meaning a pure
/// insertion at that position.
fn change_regions(base: &[&str], other: &[&str]) -> Vec<ChangeRegion> {
    let ops = diff(base, other);
    let mut regions = Vec::new();
    let mut base_pos = 0usize;

    let mut i = 0;
    while i < ops.len() {
        match ops[i].typ {
            OpType::Equal => {
                base_pos = ops[i].a + 1;
                i += 1;
            }
            _ => {
                let block_start = base_pos;
                let mut block_end = base_pos;
                let mut replacement = Vec::new();

                while i < ops.len() && ops[i].typ != OpType::Equal {
                    match ops[i].typ {
                        OpType::Delete => {
                            base_pos = ops[i].a + 1;
                            block_end = base_pos;
                        }
                        OpType::Insert => {
                            replacement.push(ops[i].new.clone());
                        }
                        OpType::Replace => {
                            base_pos = ops[i].a + 1;
                            block_end = base_pos;
                            replacement.push(ops[i].new.clone());
                        }
                        OpType::Equal => {}
                    }
                    i += 1;
                }

                regions.push(ChangeRegion {
                    base_start: block_start,
                    base_end: block_end,
                    replacement,
                });
            }
        }
    }

    regions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_changes() {
        let result = merge(&["a", "b", "c"], &["a", "b", "c"], &["a", "b", "c"]);
        assert_eq!(result.conflicts, 0);
        assert_eq!(result.lines, vec!["a", "b", "c"]);
    }

    #[test]
    fn only_ours_changed() {
        let result = merge(&["a", "b", "c"], &["a", "X", "c"], &["a", "b", "c"]);
        assert_eq!(result.conflicts, 0);
        assert_eq!(result.lines, vec!["a", "X", "c"]);
    }

    #[test]
    fn only_theirs_changed() {
        let result = merge(&["a", "b", "c"], &["a", "b", "c"], &["a", "Y", "c"]);
        assert_eq!(result.conflicts, 0);
        assert_eq!(result.lines, vec!["a", "Y", "c"]);
    }

    #[test]
    fn both_changed_same_way() {
        let result = merge(&["a", "b", "c"], &["a", "Z", "c"], &["a", "Z", "c"]);
        assert_eq!(result.conflicts, 0);
        assert_eq!(result.lines, vec!["a", "Z", "c"]);
    }

    #[test]
    fn both_changed_differently_conflict() {
        let result = merge(&["a", "b", "c"], &["a", "X", "c"], &["a", "Y", "c"]);
        assert_eq!(result.conflicts, 1);
        assert!(result.lines.iter().any(|l| l.contains("<<<<<<<")));
        assert!(result.lines.iter().any(|l| l.contains("=======")));
        assert!(result.lines.iter().any(|l| l.contains(">>>>>>>")));
        assert!(result.lines.iter().any(|l| l == "X"));
        assert!(result.lines.iter().any(|l| l == "Y"));
    }

    #[test]
    fn non_overlapping_changes_merge() {
        let result = merge(
            &["a", "b", "c", "d"],
            &["a", "X", "c", "d"],   // ours changes line 2
            &["a", "b", "c", "Y"],   // theirs changes line 4
        );
        assert_eq!(result.conflicts, 0);
        assert_eq!(result.lines, vec!["a", "X", "c", "Y"]);
    }

    #[test]
    fn pure_insertion_from_ours() {
        let result = merge(&["a", "c"], &["a", "b", "c"], &["a", "c"]);
        assert_eq!(result.conflicts, 0);
        assert_eq!(result.lines, vec!["a", "b", "c"]);
    }

    #[test]
    fn pure_insertion_from_both_different() {
        let result = merge(&["a", "c"], &["a", "X", "c"], &["a", "Y", "c"]);
        assert_eq!(result.conflicts, 1);
    }

    #[test]
    fn pure_deletion_from_ours() {
        let result = merge(&["a", "b", "c"], &["a", "c"], &["a", "b", "c"]);
        assert_eq!(result.conflicts, 0);
        assert_eq!(result.lines, vec!["a", "c"]);
    }

    #[test]
    fn both_delete_same_line() {
        let result = merge(&["a", "b", "c"], &["a", "c"], &["a", "c"]);
        assert_eq!(result.conflicts, 0);
        assert_eq!(result.lines, vec!["a", "c"]);
    }

    #[test]
    fn accepts_string_slices() {
        let base = vec!["a".to_string(), "b".to_string()];
        let ours = vec!["a".to_string(), "X".to_string()];
        let theirs = vec!["a".to_string(), "b".to_string()];
        let result = merge(&base, &ours, &theirs);
        assert_eq!(result.lines, vec!["a", "X"]);
    }
}
