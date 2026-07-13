use crate::inline::{Op, OpType};

#[derive(Debug, Clone)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct HunkLine {
    pub kind: LineKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LineKind {
    Context,
    Delete,
    Insert,
}

#[derive(Debug, Clone)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Hunk {
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    pub lines: Vec<HunkLine>,
}

pub fn hunks(ops: &[Op], context: usize) -> Vec<Hunk> {
    let change_indices: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter(|(_, o)| o.typ != OpType::Equal)
        .map(|(i, _)| i)
        .collect();

    if change_indices.is_empty() {
        return Vec::new();
    }

    let mut groups: Vec<(usize, usize)> = Vec::new();
    for &ci in &change_indices {
        if let Some(last) = groups.last_mut() {
            if ci <= last.1 + 2 * context {
                last.1 = ci;
                continue;
            }
        }
        groups.push((ci, ci));
    }

    let mut result = Vec::with_capacity(groups.len());
    for (g_start, g_end) in groups {
        let hunk_start = g_start.saturating_sub(context);
        let hunk_end = (g_end + context + 1).min(ops.len());

        let mut old_start = 1usize;
        let mut new_start = 1usize;
        if let Some(first) = ops.get(hunk_start) {
            old_start = first.a + 1;
            new_start = first.b + 1;
        }

        let mut old_count = 0;
        let mut new_count = 0;
        let mut lines = Vec::with_capacity(hunk_end - hunk_start);

        for op in &ops[hunk_start..hunk_end] {
            match op.typ {
                OpType::Equal => {
                    old_count += 1;
                    new_count += 1;
                    lines.push(HunkLine { kind: LineKind::Context, text: op.old.clone() });
                }
                OpType::Delete => {
                    old_count += 1;
                    lines.push(HunkLine { kind: LineKind::Delete, text: op.old.clone() });
                }
                OpType::Insert => {
                    new_count += 1;
                    lines.push(HunkLine { kind: LineKind::Insert, text: op.new.clone() });
                }
                OpType::Replace => {
                    old_count += 1;
                    new_count += 1;
                    lines.push(HunkLine { kind: LineKind::Delete, text: op.old.clone() });
                    lines.push(HunkLine { kind: LineKind::Insert, text: op.new.clone() });
                }
            }
        }

        result.push(Hunk { old_start, old_count, new_start, new_count, lines });
    }
    result
}

pub fn to_unified(ops: &[Op], context: usize) -> String {
    let hunks = hunks(ops, context);
    let mut out = String::new();
    for h in &hunks {
        out.push_str(&format_hunk_header(h));
        for line in &h.lines {
            let prefix = match line.kind {
                LineKind::Context => ' ',
                LineKind::Delete => '-',
                LineKind::Insert => '+',
            };
            out.push(prefix);
            out.push_str(&line.text);
            out.push('\n');
        }
    }
    out
}

fn format_hunk_header(h: &Hunk) -> String {
    let old_s = if h.old_count == 0 { h.old_start.saturating_sub(1) } else { h.old_start };
    let new_s = if h.new_count == 0 { h.new_start.saturating_sub(1) } else { h.new_start };
    format!("@@ -{},{} +{},{} @@\n", old_s, h.old_count, new_s, h.new_count)
}
