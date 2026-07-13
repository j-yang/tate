use crate::inline::Op;

pub(crate) fn lcs_diff(a: &[String], b: &[String]) -> Vec<Op> {
    let n = a.len();
    let m = b.len();
    if n == 0 {
        return b.iter().enumerate().map(|(j, s)| Op::insert(j + 1, s)).collect();
    }
    if m == 0 {
        return a.iter().enumerate().map(|(i, s)| Op::delete(i + 1, s)).collect();
    }
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in 1..=n {
        for j in 1..=m {
            if a[i - 1] == b[j - 1] {
                dp[i][j] = dp[i - 1][j - 1] + 1;
            } else {
                dp[i][j] = dp[i - 1][j].max(dp[i][j - 1]);
            }
        }
    }
    let mut ops: Vec<Op> = Vec::new();
    let mut i = n;
    let mut j = m;
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && a[i - 1] == b[j - 1] {
            ops.push(Op::equal(i, j, &a[i - 1], &b[j - 1]));
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || dp[i][j - 1] >= dp[i - 1][j]) {
            ops.push(Op::insert(j, &b[j - 1]));
            j -= 1;
        } else {
            ops.push(Op::delete(i, &a[i - 1]));
            i -= 1;
        }
    }
    ops.reverse();
    ops
}

pub(crate) fn tokenize(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_alnum: Option<bool> = None;
    for ch in s.chars() {
        let is_alnum = ch.is_alphanumeric();
        match cur_alnum {
            Some(prev) if prev == is_alnum => cur.push(ch),
            _ => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                cur.push(ch);
                cur_alnum = Some(is_alnum);
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub(crate) fn char_similarity(equal_chars: usize, a_len: usize, b_len: usize) -> f64 {
    let denom = (a_len + b_len).max(1) as f64;
    (2 * equal_chars) as f64 / denom
}
