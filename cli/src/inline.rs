use serde::{Deserialize, Serialize};

use crate::lcs::{char_similarity, lcs_diff, tokenize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpType {
    Equal,
    Delete,
    Insert,
    Replace,
}

#[derive(Debug, Clone)]
#[derive(Serialize, Deserialize)]
pub struct Op {
    #[serde(rename = "type")]
    pub typ: OpType,
    pub a: usize,
    pub b: usize,
    pub old: String,
    pub new: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub old_segs: Vec<Seg>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub new_segs: Vec<Seg>,
}

#[derive(Debug, Clone)]
#[derive(Serialize, Deserialize)]
pub struct Seg {
    pub text: String,
    pub changed: bool,
}

pub const DEFAULT_SIMILARITY: f64 = 0.5;

impl Op {
    pub fn equal(a: usize, b: usize, old: &str, new: &str) -> Self {
        Op { typ: OpType::Equal, a, b, old: old.to_string(), new: new.to_string(), old_segs: Vec::new(), new_segs: Vec::new() }
    }

    pub fn insert(b: usize, new: &str) -> Self {
        Op { typ: OpType::Insert, a: 0, b, old: String::new(), new: new.to_string(), old_segs: Vec::new(), new_segs: Vec::new() }
    }

    pub fn delete(a: usize, old: &str) -> Self {
        Op { typ: OpType::Delete, a, b: 0, old: old.to_string(), new: String::new(), old_segs: Vec::new(), new_segs: Vec::new() }
    }

    pub fn replace(a: usize, b: usize, old: &str, new: &str, old_segs: Vec<Seg>, new_segs: Vec<Seg>) -> Self {
        Op { typ: OpType::Replace, a, b, old: old.to_string(), new: new.to_string(), old_segs, new_segs }
    }
}

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
            if let Some((old_segs, new_segs)) = inline_segments(&d.old, &s.new, threshold) {
                out.push(Op::replace(d.a, s.b, &d.old, &s.new, old_segs, new_segs));
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

pub fn inline_segments(a: &str, b: &str, threshold: f64) -> Option<(Vec<Seg>, Vec<Seg>)> {
    let ta = tokenize(a);
    let tb = tokenize(b);
    let ops = lcs_diff(&ta, &tb);

    let mut equal_chars = 0usize;
    for op in &ops {
        if op.typ == OpType::Equal {
            equal_chars += op.old.chars().count();
        }
    }
    let similarity = char_similarity(equal_chars, a.chars().count(), b.chars().count());
    if similarity < threshold {
        return None;
    }

    let mut old_segs: Vec<Seg> = Vec::new();
    let mut new_segs: Vec<Seg> = Vec::new();
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
    for op in &ops {
        match op.typ {
            OpType::Equal => {
                push(&mut old_segs, &op.old, false);
                push(&mut new_segs, &op.new, false);
            }
            OpType::Delete => push(&mut old_segs, &op.old, true),
            OpType::Insert => push(&mut new_segs, &op.new, true),
            OpType::Replace => {}
        }
    }
    Some((old_segs, new_segs))
}
