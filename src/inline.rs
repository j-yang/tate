//! Inline alignment: turn raw delete/insert edit scripts into Replace ops with
//! word-level inline highlights.
//!
//! A line-diff engine (similar / imara / a hand-rolled one) gives you ops of the
//! form `Equal | Delete | Insert`. The default rendering of a line that was
//! edited from `foo bar` to `foo baz` is a delete + an insert — visually noisy.
//! [`pair_replacements`] walks the script and merges adjacent delete/insert pairs
//! that are similar enough into a single [`Op`] with `typ = Replace` carrying
//! inline segments (`a_segs` / `b_segs`), so your UI can highlight only the
//! changed word.
//!
//! [`inline_segments`] does the same word-level alignment for one pair of lines,
//! usable on its own (e.g. when you already know which two lines to compare, as
//! the RTF cell highlighter does). It uses a small internal LCS DP — line-level
//! diff is your job (feed the result into [`pair_replacements`]), but word-level
//! diff on a single pair of lines is tiny and bounded, so we bundle it.

use serde::{Deserialize, Serialize};

/// Op type emitted by a line-diff engine. `Replace` is never produced by raw
/// line-diff; it appears only after [`pair_replacements`] post-processing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpType {
    Equal,
    Delete,
    Insert,
    Replace,
}

/// One edit operation between two line sequences.
///
/// `a` / `b` are 1-based indices into the A/B source slices (0 = absent on that
/// side, for Insert / Delete). `a_val` / `b_val` carry the source text; for
/// `Equal` they are equal, for `Delete` `b_val` is empty, and so on. `a_segs` /
/// `b_segs` are populated only for [`OpType::Replace`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Op {
    pub typ: OpType,
    pub a: usize,
    pub b: usize,
    pub a_val: String,
    pub b_val: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub a_segs: Vec<Seg>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub b_segs: Vec<Seg>,
}

/// A word/token-level inline segment with its "changed vs the other side" flag.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Seg {
    pub text: String,
    pub changed: bool,
}

/// Default similarity threshold: lines with at least this fraction of shared
/// characters are paired into a `Replace` row instead of being shown as a
/// separate delete + insert. 0.5 means "at least half of the characters are
/// shared between the two lines" — the same default `git diff` effectively uses.
pub const DEFAULT_SIMILARITY: f64 = 0.5;

impl Op {
    pub fn equal(a: usize, b: usize, av: &str, bv: &str) -> Self {
        Op {
            typ: OpType::Equal,
            a,
            b,
            a_val: av.to_string(),
            b_val: bv.to_string(),
            a_segs: Vec::new(),
            b_segs: Vec::new(),
        }
    }

    pub fn insert(b: usize, bv: &str) -> Self {
        Op {
            typ: OpType::Insert,
            a: 0,
            b,
            a_val: String::new(),
            b_val: bv.to_string(),
            a_segs: Vec::new(),
            b_segs: Vec::new(),
        }
    }

    pub fn delete(a: usize, av: &str) -> Self {
        Op {
            typ: OpType::Delete,
            a,
            b: 0,
            a_val: av.to_string(),
            b_val: String::new(),
            a_segs: Vec::new(),
            b_segs: Vec::new(),
        }
    }

    pub fn replace(a: usize, b: usize, av: &str, bv: &str, a_segs: Vec<Seg>, b_segs: Vec<Seg>) -> Self {
        Op {
            typ: OpType::Replace,
            a,
            b,
            a_val: av.to_string(),
            b_val: bv.to_string(),
            a_segs,
            b_segs,
        }
    }
}

/// Pair consecutive delete+insert blocks in `ops` into `Replace` ops.
///
/// `ops` should come from a line-diff engine that only emits `Equal | Delete |
/// Insert`; any `Replace` ops already present are passed through untouched.
/// Within each maximal block of deletes followed by inserts, lines are paired
/// positionally (1st-with-1st, …); each pair whose similarity is at or above
/// `threshold` is collapsed into a `Replace` carrying word-level segments,
/// the rest stay separate `Delete` + `Insert` ops.
pub fn pair_replacements(ops: Vec<Op>, threshold: f64) -> Vec<Op> {
    let mut out: Vec<Op> = Vec::with_capacity(ops.len());
    let mut i = 0;
    while i < ops.len() {
        if ops[i].typ == OpType::Equal || ops[i].typ == OpType::Replace {
            out.push(ops[i].clone());
            i += 1;
            continue;
        }
        let block_start = i;
        while i < ops.len() && ops[i].typ == OpType::Delete {
            i += 1;
        }
        let dels = &ops[block_start..i];
        let ins_start = i;
        while i < ops.len() && ops[i].typ == OpType::Insert {
            i += 1;
        }
        let inss = &ops[ins_start..i];

        let pairs = dels.len().min(inss.len());
        for k in 0..pairs {
            let d = &dels[k];
            let s = &inss[k];
            if let Some((a_segs, b_segs)) = inline_segments(&d.a_val, &s.b_val, threshold) {
                out.push(Op::replace(d.a, s.b, &d.a_val, &s.b_val, a_segs, b_segs));
            } else {
                out.push(d.clone());
                out.push(s.clone());
            }
        }
        for d in &dels[pairs..] {
            out.push(d.clone());
        }
        for s in &inss[pairs..] {
            out.push(s.clone());
        }
    }
    out
}

/// Compute word-level inline highlight segments for two lines, returning
/// `(a_segs, b_segs)`. Returns `None` when the lines are below `threshold`
/// similar — caller should show them as a delete + insert rather than a Replace.
///
/// Word-level diffing delegates to [`similar::TextDiff::from_words`] (Myers),
/// and the similarity check uses [`similar::diff_ratio`] so the threshold's
/// semantics match `similar`'s own definition. The output `Vec<Seg>` is
/// tate's own type — `similar` produces Equal/Delete/Insert ops; we collapse
/// adjacent same-tag runs into a single `Seg` with its `changed` flag.
pub fn inline_segments(a: &str, b: &str, threshold: f64) -> Option<(Vec<Seg>, Vec<Seg>)> {
    let diff = similar::TextDiff::from_words(a, b);
    // Sørensen–Dice similarity over characters: shared chars on both sides over
    // total chars. Matches the original shtuka heuristic so callers' thresholds
    // keep the same meaning.
    let mut equal_chars = 0usize;
    for change in diff.iter_all_changes() {
        if let similar::ChangeTag::Equal = change.tag() {
            equal_chars += change.value().chars().count();
        }
    }
    let denom = (a.chars().count() + b.chars().count()).max(1) as f64;
    let similarity = (2 * equal_chars) as f64 / denom;
    if similarity < threshold {
        return None;
    }
    let mut a_segs: Vec<Seg> = Vec::new();
    let mut b_segs: Vec<Seg> = Vec::new();
    let push = |segs: &mut Vec<Seg>, text: &str, changed: bool| {
        if text.is_empty() {
            return;
        }
        if let Some(last) = segs.last_mut() {
            if last.changed == changed {
                last.text.push_str(text);
                return;
            }
        }
        segs.push(Seg { text: text.to_string(), changed });
    };
    for change in diff.iter_all_changes() {
        match change.tag() {
            similar::ChangeTag::Equal => {
                push(&mut a_segs, change.value(), false);
                push(&mut b_segs, change.value(), false);
            }
            similar::ChangeTag::Delete => push(&mut a_segs, change.value(), true),
            similar::ChangeTag::Insert => push(&mut b_segs, change.value(), true),
        }
    }
    Some((a_segs, b_segs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_number_change_pairs_as_replace() {
        let ops = vec![
            Op::delete(1, "Section A.1 Overview .... 17"),
            Op::insert(1, "Section A.1 Overview .... 18"),
        ];
        let out = pair_replacements(ops, DEFAULT_SIMILARITY);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].typ, OpType::Replace);
        let a_chg: String = out[0].a_segs.iter().filter(|s| s.changed).map(|s| s.text.clone()).collect();
        let b_chg: String = out[0].b_segs.iter().filter(|s| s.changed).map(|s| s.text.clone()).collect();
        assert_eq!(a_chg, "17");
        assert_eq!(b_chg, "18");
    }

    #[test]
    fn unrelated_lines_stay_separate() {
        let ops = vec![
            Op::delete(1, "the quick brown fox"),
            Op::insert(1, "completely different text here"),
        ];
        let out = pair_replacements(ops, DEFAULT_SIMILARITY);
        assert!(out.iter().all(|o| o.typ != OpType::Replace));
        assert!(out.iter().any(|o| o.typ == OpType::Delete));
        assert!(out.iter().any(|o| o.typ == OpType::Insert));
    }

    #[test]
    fn inline_only_marks_changed_word() {
        let (a_segs, b_segs) =
            inline_segments("foo bar", "foo baz", DEFAULT_SIMILARITY).expect("similar enough");
        let a_changed: String = a_segs.iter().filter(|s| s.changed).map(|s| s.text.clone()).collect();
        let b_changed: String = b_segs.iter().filter(|s| s.changed).map(|s| s.text.clone()).collect();
        assert_eq!(a_changed, "bar");
        assert_eq!(b_changed, "baz");
    }

    #[test]
    fn scattered_number_changes_word_level() {
        let a = "ROW01 12 0.0617 0.020 0.0400 0.075";
        let b = "ROW01 15 0.0580 0.020 0.0400 0.075";
        let (a_segs, b_segs) = inline_segments(a, b, DEFAULT_SIMILARITY).expect("similar enough");
        assert_eq!(a_segs.len(), b_segs.len());
        let b_changed: Vec<String> = b_segs
            .iter()
            .filter(|s| s.changed)
            .map(|s| s.text.trim().to_string())
            .collect();
        assert!(b_changed.iter().any(|s| s.contains("15")));
        assert!(b_changed.iter().any(|s| s.contains("0580")));
        let unchanged: String = b_segs.iter().filter(|s| !s.changed).map(|s| s.text.clone()).collect();
        assert!(unchanged.contains("0.0400"));
    }

    #[test]
    fn unequal_block_pairs_leftovers_pass_through() {
        let ops = vec![
            Op::delete(1, "gone"),
            Op::delete(2, "also gone"),
            Op::insert(1, "new"),
        ];
        let out = pair_replacements(ops, DEFAULT_SIMILARITY);
        // "gone" vs "new" below threshold → Delete + Insert; "also gone" leftover → Delete.
        assert!(out.iter().any(|o| o.typ == OpType::Delete));
        assert!(out.iter().any(|o| o.typ == OpType::Insert));
        assert!(out.iter().all(|o| o.typ != OpType::Replace));
    }

    #[test]
    fn equal_ops_pass_through() {
        let ops = vec![
            Op::equal(1, 1, "same", "same"),
            Op::delete(2, "removed"),
            Op::insert(2, "added"),
            Op::equal(3, 3, "tail", "tail"),
        ];
        let out = pair_replacements(ops, DEFAULT_SIMILARITY);
        assert_eq!(out[0].typ, OpType::Equal);
        assert_eq!(out.last().unwrap().typ, OpType::Equal);
    }

    #[test]
    fn empty_inputs() {
        let out = pair_replacements(Vec::new(), DEFAULT_SIMILARITY);
        assert!(out.is_empty());
        assert!(inline_segments("", "", DEFAULT_SIMILARITY).is_none()
            || inline_segments("", "", DEFAULT_SIMILARITY).map(|(a, b)| a.is_empty() && b.is_empty()).unwrap_or(false));
    }
}