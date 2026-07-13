use crate::inline::Op;

const MAX_LCS_CELLS: usize = 4 << 20;
const MAX_HIRSCHBERG_CELLS: usize = 16 << 20;

pub fn diff<A: AsRef<str>, B: AsRef<str>>(a: &[A], b: &[B]) -> Vec<Op> {
    let a: Vec<&str> = a.iter().map(AsRef::as_ref).collect();
    let b: Vec<&str> = b.iter().map(AsRef::as_ref).collect();
    let mut out: Vec<Op> = Vec::with_capacity(a.len() + b.len());
    diff_into(&a, &b, 0, 0, &mut out);
    out
}

fn diff_into(a: &[&str], b: &[&str], mut off_a: usize, mut off_b: usize, out: &mut Vec<Op>) {
    let mut p = 0;
    while p < a.len() && p < b.len() && a[p] == b[p] {
        out.push(Op::equal(off_a + p, off_b + p, a[p], b[p]));
        p += 1;
    }
    off_a += p;
    off_b += p;
    let a = &a[p..];
    let b = &b[p..];

    let mut s = 0;
    while s < a.len() && s < b.len() && a[a.len() - 1 - s] == b[b.len() - 1 - s] {
        s += 1;
    }
    let a_mid = &a[..a.len() - s];
    let b_mid = &b[..b.len() - s];

    diff_middle(a_mid, b_mid, off_a, off_b, out);

    for t in 0..s {
        let ai = a_mid.len() + t;
        let bi = b_mid.len() + t;
        out.push(Op::equal(off_a + ai, off_b + bi, a[ai], b[bi]));
    }
}

fn diff_middle(a: &[&str], b: &[&str], off_a: usize, off_b: usize, out: &mut Vec<Op>) {
    if a.is_empty() {
        for (j, bv) in b.iter().copied().enumerate() {
            out.push(Op::insert(off_b + j, bv));
        }
        return;
    }
    if b.is_empty() {
        for (i, av) in a.iter().copied().enumerate() {
            out.push(Op::delete(off_a + i, av));
        }
        return;
    }

    let anchors = patience_anchors(a, b);
    if anchors.is_empty() {
        solve_exact(a, b, off_a, off_b, out);
        return;
    }

    let (mut prev_a, mut prev_b) = (0usize, 0usize);
    for an in &anchors {
        diff_into(&a[prev_a..an.a], &b[prev_b..an.b], off_a + prev_a, off_b + prev_b, out);
        out.push(Op::equal(off_a + an.a, off_b + an.b, a[an.a], b[an.b]));
        prev_a = an.a + 1;
        prev_b = an.b + 1;
    }
    diff_into(&a[prev_a..], &b[prev_b..], off_a + prev_a, off_b + prev_b, out);
}

fn solve_exact(a: &[&str], b: &[&str], off_a: usize, off_b: usize, out: &mut Vec<Op>) {
    let cells = a.len().saturating_mul(b.len());
    if cells <= MAX_LCS_CELLS {
        lcs_full(a, b, off_a, off_b, out);
    } else if cells <= MAX_HIRSCHBERG_CELLS {
        hirschberg(a, b, off_a, off_b, out);
    } else {
        for (i, av) in a.iter().copied().enumerate() {
            out.push(Op::delete(off_a + i, av));
        }
        for (j, bv) in b.iter().copied().enumerate() {
            out.push(Op::insert(off_b + j, bv));
        }
    }
}

#[derive(Clone, Copy)]
struct AnchorPair {
    a: usize,
    b: usize,
}

fn patience_anchors(a: &[&str], b: &[&str]) -> Vec<AnchorPair> {
    use std::collections::HashMap;
    let mut count_a: HashMap<&str, i32> = HashMap::with_capacity(a.len());
    for x in a {
        *count_a.entry(*x).or_insert(0) += 1;
    }
    let mut count_b: HashMap<&str, i32> = HashMap::with_capacity(b.len());
    for x in b {
        *count_b.entry(*x).or_insert(0) += 1;
    }
    let mut pos_b: HashMap<&str, usize> = HashMap::with_capacity(b.len());
    for (j, x) in b.iter().enumerate() {
        if *count_b.get(*x).unwrap_or(&0) == 1 {
            pos_b.insert(*x, j);
        }
    }

    let mut seq: Vec<AnchorPair> = Vec::new();
    for (i, x) in a.iter().enumerate() {
        if *count_a.get(*x).unwrap_or(&0) != 1 {
            continue;
        }
        if let Some(&j) = pos_b.get(*x) {
            seq.push(AnchorPair { a: i, b: j });
        }
    }
    if seq.is_empty() {
        return Vec::new();
    }

    let mut piles: Vec<usize> = Vec::with_capacity(seq.len());
    let mut prev: Vec<isize> = vec![-1; seq.len()];
    for i in 0..seq.len() {
        let (mut lo, mut hi) = (0usize, piles.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            if seq[piles[mid]].b < seq[i].b {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        prev[i] = if lo > 0 { piles[lo - 1] as isize } else { -1 };
        if lo == piles.len() {
            piles.push(i);
        } else {
            piles[lo] = i;
        }
    }

    let mut out: Vec<AnchorPair> = Vec::with_capacity(piles.len());
    let mut k = *piles.last().unwrap() as isize;
    while k >= 0 {
        out.push(seq[k as usize]);
        k = prev[k as usize];
    }
    out.reverse();
    out
}

fn lcs_full(a: &[&str], b: &[&str], off_a: usize, off_b: usize, out: &mut Vec<Op>) {
    let (n, m) = (a.len(), b.len());
    let stride = m + 1;
    let mut dp = vec![0i32; (n + 1) * stride];
    for i in 1..=n {
        let ai = a[i - 1];
        let (prev_part, cur_part) = dp.split_at_mut(i * stride);
        let prev_row = &prev_part[(i - 1) * stride..(i - 1) * stride + stride];
        let row = &mut cur_part[..stride];
        for j in 1..=m {
            if ai == b[j - 1] {
                row[j] = prev_row[j - 1] + 1;
            } else if prev_row[j] >= row[j - 1] {
                row[j] = prev_row[j];
            } else {
                row[j] = row[j - 1];
            }
        }
    }

    let mut tmp: Vec<Op> = Vec::with_capacity(n + m);
    let (mut i, mut j) = (n, m);
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && a[i - 1] == b[j - 1] {
            tmp.push(Op::equal(off_a + i - 1, off_b + j - 1, a[i - 1], b[j - 1]));
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || dp[i * stride + j - 1] >= dp[(i - 1) * stride + j]) {
            tmp.push(Op::insert(off_b + j - 1, b[j - 1]));
            j -= 1;
        } else {
            tmp.push(Op::delete(off_a + i - 1, a[i - 1]));
            i -= 1;
        }
    }
    out.extend(tmp.into_iter().rev());
}

fn hirschberg(a: &[&str], b: &[&str], off_a: usize, off_b: usize, out: &mut Vec<Op>) {
    if a.is_empty() {
        for (j, bv) in b.iter().copied().enumerate() {
            out.push(Op::insert(off_b + j, bv));
        }
        return;
    }
    if b.is_empty() {
        for (i, av) in a.iter().copied().enumerate() {
            out.push(Op::delete(off_a + i, av));
        }
        return;
    }
    if a.len() == 1 {
        let mut idx: isize = -1;
        for (j, bv) in b.iter().enumerate() {
            if *bv == a[0] {
                idx = j as isize;
                break;
            }
        }
        if idx < 0 {
            out.push(Op::delete(off_a, a[0]));
            for (j, bv) in b.iter().copied().enumerate() {
                out.push(Op::insert(off_b + j, bv));
            }
            return;
        }
        let idx = idx as usize;
        for (j, bv) in b[..idx].iter().copied().enumerate() {
            out.push(Op::insert(off_b + j, bv));
        }
        out.push(Op::equal(off_a, off_b + idx, a[0], b[idx]));
        for (j, bv) in b[idx + 1..].iter().copied().enumerate() {
            out.push(Op::insert(off_b + idx + 1 + j, bv));
        }
        return;
    }

    let mid = a.len() / 2;
    let score_l = lcs_row(&a[..mid], b, false);
    let score_r = lcs_row(&a[mid..], b, true);

    let mut best: i32 = -1;
    let mut best_k = 0usize;
    for k in 0..=b.len() {
        let s = score_l[k] + score_r[b.len() - k];
        if s > best {
            best = s;
            best_k = k;
        }
    }

    hirschberg(&a[..mid], &b[..best_k], off_a, off_b, out);
    hirschberg(&a[mid..], &b[best_k..], off_a + mid, off_b + best_k, out);
}

fn lcs_row(a: &[&str], b: &[&str], rev: bool) -> Vec<i32> {
    let mut prev = vec![0i32; b.len() + 1];
    let mut cur = vec![0i32; b.len() + 1];
    let at = |s: &[&str], i: usize| -> usize {
        if rev { s.len() - 1 - i } else { i }
    };
    for i in 0..a.len() {
        let ai = a[at(a, i)];
        for j in 1..=b.len() {
            let bj = b[at(b, j - 1)];
            if ai == bj {
                cur[j] = prev[j - 1] + 1;
            } else if prev[j] >= cur[j - 1] {
                cur[j] = prev[j];
            } else {
                cur[j] = cur[j - 1];
            }
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev
}
