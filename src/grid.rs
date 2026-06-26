//! 2D grid alignment: take two tables of strings and produce one aligned grid
//! with cell-, row-, and column-level change status. Format-agnostic — it has
//! no knowledge of any particular file format; callers parse their format
//! (Excel, CSV, HTML tables, SQL result sets, …) into `&[Vec<String>]` and hand
//! the grid to [`grid_diff`].
//!
//! Algorithm:
//! 1. Detect the header row on each side (the first row that fills ≥
//!    [`GridOptions::header_fill_ratio`] of the width).
//! 2. Run an LCS diff over the header cells to align columns, producing a slot
//!    list mapping each output column to (a_col, b_col); a slot with one side
//!    missing means an added/removed column.
//! 3. Run an LCS diff over row signatures (aligned-column cells joined by a
//!    separator) to align rows, with an [`GridOptions::lcs_row_budget`] cap
//!    beyond which rows are aligned positionally.
//! 4. For each unpaired delete/insert pair left over, check row similarity and
//!    promote to `modified` when above threshold (the grid-level analogue of
//!    [`crate::inline::pair_replacements`]).
//! 5. **Iterative refinement:** re-align columns using the row-matched data
//!    (not just headers), then re-align rows with the improved column slots.
//!    Alternate until the alignment stabilises. This is coordinate descent:
//!    each step is exact (LCS), and the total cost monotonically decreases.
//! 6. For each row slot, render cells from each side through the column slots
//!    and tag `equal | modified | added | removed` per cell. Modified cells
//!    carry word-level inline segments via [`crate::inline::inline_segments`].

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::inline::OpType;
use crate::inline::{inline_segments, Seg, DEFAULT_SIMILARITY};
use crate::lcs::lcs_diff;

/// Status of one cell, row, or column in the aligned grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum Status {
    Equal,
    Modified,
    Added,
    Removed,
}

/// One diffed cell: status plus the old and new text (either may be empty when
/// the cell only exists on one side). For `Modified` cells, `old_segs` /
/// `new_segs` carry word-level inline segments for highlighting only the
/// changed words within the cell.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct CellChange {
    pub status: Status,
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "String::is_empty"))]
    pub old: String,
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "String::is_empty"))]
    pub new: String,
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Vec::is_empty"))]
    pub old_segs: Vec<Seg>,
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Vec::is_empty"))]
    pub new_segs: Vec<Seg>,
}

/// One aligned column slot: its display name and its status across the two
/// sides.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct GridColumn {
    pub name: String,
    pub status: Status,
}

/// One aligned row: its status and source-row pointers (1-based, 0 = absent).
/// `header` flags the detected header row for the UI.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct GridRow {
    pub status: Status,
    #[cfg_attr(feature = "serde", serde(rename = "rowA"))]
    pub row_a: usize,
    #[cfg_attr(feature = "serde", serde(rename = "rowB"))]
    pub row_b: usize,
    pub header: bool,
    pub cells: Vec<CellChange>,
}

/// The result of diffing two grids: aligned columns, rows, and per-cell status.
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct GridDiff {
    pub columns: Vec<GridColumn>,
    pub rows: Vec<GridRow>,
    #[cfg_attr(feature = "serde", serde(rename = "addedRows"))]
    pub added_rows: usize,
    #[cfg_attr(feature = "serde", serde(rename = "removedRows"))]
    pub removed_rows: usize,
    #[cfg_attr(feature = "serde", serde(rename = "modifiedRows"))]
    pub modified_rows: usize,
    #[cfg_attr(feature = "serde", serde(rename = "addedCols"))]
    pub added_cols: usize,
    #[cfg_attr(feature = "serde", serde(rename = "removedCols"))]
    pub removed_cols: usize,
    /// Operational notes surfaced to the UI (e.g. "row budget exceeded,
    /// positional alignment used").
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Vec::is_empty"))]
    pub notes: Vec<String>,
}

/// Configuration for [`grid_diff`]. Every knob is exposed so callers can adapt
/// the heuristics to non-table-shaped grids (sparse ledgers, wide pivots,
/// dense reports, …).
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct GridOptions {
    /// Row alignment is abandoned at this total row count (A or B) for
    /// performance; rows past the budget are paired positionally. Raise for
    /// server use, lower for interactive previews.
    pub lcs_row_budget: usize,
    /// A row is considered a header if it fills ≥ `header_fill_ratio` of the
    /// grid width. 0.8 matches a moderately dense CSV-with-title layout.
    pub header_fill_ratio: f64,
    /// Similarity threshold for promoting a leftover del+ins row pair to
    /// `Modified` (fraction of cells equal over the aligned common columns).
    /// 0.5 = "at least half the cells match".
    pub row_similarity_threshold: f64,
    /// When true, `GridColumn.name` uses the detected header text (e.g.
    /// "Variable Name"); when false, uses Excel-style letters (e.g. "A").
    pub use_header_names: bool,
    /// Maximum iterations of alternating column↔row refinement (coordinate
    /// descent). Set to 0 to disable (single-pass header→row pipeline only).
    /// Default: 2 (one refinement pass after the initial alignment).
    pub refinement_iters: usize,
}

impl Default for GridOptions {
    fn default() -> Self {
        GridOptions {
            lcs_row_budget: 4_000,
            header_fill_ratio: 0.8,
            row_similarity_threshold: 0.5,
            use_header_names: true,
            refinement_iters: 2,
        }
    }
}

/// Diff two string grids and produce one aligned grid.
///
/// `rows_a` / `rows_b` are arbitrary `&[Vec<String>]`; they typically come from
/// parsing xlsx/CSV/HTML/SQL/etc., but `grid_diff` doesn't know or care. Each
/// row is one record; each cell is already stringified.
pub fn grid_diff(
    rows_a: &[Vec<String>],
    rows_b: &[Vec<String>],
    opts: &GridOptions,
) -> GridDiff {
    let width_a = max_width(rows_a);
    let width_b = max_width(rows_b);

    let header_a = detect_header_row(rows_a, width_a, opts.header_fill_ratio);
    let header_b = detect_header_row(rows_b, width_b, opts.header_fill_ratio);

    // --- Initial column alignment via header LCS ---
    let mut slots = align_columns_by_header(rows_a, rows_b, width_a, width_b, header_a, header_b);

    // --- Initial row alignment ---
    let mut row_pairs = if rows_a.len() > opts.lcs_row_budget || rows_b.len() > opts.lcs_row_budget {
        align_rows_by_position(rows_a, rows_b)
    } else {
        align_rows_by_lcs(rows_a, rows_b, &slots, opts)
    };

    // --- Iterative refinement: coordinate descent on (columns, rows) ---
    for _ in 0..opts.refinement_iters {
        if rows_a.len() > opts.lcs_row_budget || rows_b.len() > opts.lcs_row_budget {
            break;
        }

        // Re-align columns using the row-matched data (not just headers).
        let new_slots = align_columns_by_data(rows_a, rows_b, &row_pairs, &slots, width_a, width_b);
        if new_slots == slots {
            break; // converged
        }
        slots = new_slots;

        // Re-align rows with the improved column slots.
        let new_pairs = align_rows_by_lcs(rows_a, rows_b, &slots, opts);
        if new_pairs == row_pairs {
            break; // converged
        }
        row_pairs = new_pairs;
    }

    // --- Render columns ---
    let head_a: &[String] = if header_a > 0 { &rows_a[header_a - 1] } else { &[] };
    let head_b: &[String] = if header_b > 0 { &rows_b[header_b - 1] } else { &[] };

    let mut gd = GridDiff {
        columns: Vec::with_capacity(slots.len()),
        ..Default::default()
    };
    for (k, slot) in slots.iter().enumerate() {
        let status = match (slot.a >= 0, slot.b >= 0) {
            (false, true) => { gd.added_cols += 1; Status::Added }
            (true, false) => { gd.removed_cols += 1; Status::Removed }
            _ => Status::Equal,
        };
        let name = if opts.use_header_names {
            column_display_name(k, slot, head_a, head_b)
        } else {
            col_letter(k)
        };
        gd.columns.push(GridColumn { name, status });
    }

    // --- Render rows ---
    for rp in &row_pairs {
        let mut gr = build_grid_row(*rp, rows_a, rows_b, &slots);
        if (gr.row_a != 0 && gr.row_a == header_a) || (gr.row_b != 0 && gr.row_b == header_b) {
            gr.header = true;
        }
        match gr.status {
            Status::Added => gd.added_rows += 1,
            Status::Removed => gd.removed_rows += 1,
            Status::Modified => gd.modified_rows += 1,
            _ => {}
        }
        gd.rows.push(gr);
    }

    if rows_a.len() > opts.lcs_row_budget || rows_b.len() > opts.lcs_row_budget {
        gd.notes.push(format!(
            "row count exceeds budget ({}); rows matched by position instead of LCS alignment",
            opts.lcs_row_budget
        ));
    }

    gd
}

fn max_width(rows: &[Vec<String>]) -> usize {
    rows.iter().map(|r| r.len()).max().unwrap_or(0)
}

/// detect_header_row returns the 1-based index of the row that most likely is
/// the table header: the first row filling ≥ `header_fill_ratio` of the width.
/// 0 if none.
fn detect_header_row(rows: &[Vec<String>], width: usize, ratio: f64) -> usize {
    if width < 2 {
        return 0;
    }
    let threshold = ((width as f64 * ratio) + 0.5) as usize;
    let mut best_row = 0;
    let mut best_filled = 0;
    for (i, r) in rows.iter().enumerate() {
        let filled = r.iter().filter(|c| !c.trim().is_empty()).count();
        if filled >= threshold && filled > best_filled {
            best_filled = filled;
            best_row = i + 1;
        }
    }
    best_row
}

/// Convert a 0-based column index to its Excel-style letter (0 -> A, 25 -> Z,
/// 26 -> AA). Used as a fallback display name when no header text is available.
pub fn col_letter(mut i: usize) -> String {
    let mut b: Vec<u8> = Vec::new();
    i += 1;
    while i > 0 {
        i -= 1;
        b.insert(0, b'A' + (i % 26) as u8);
        i /= 26;
    }
    String::from_utf8(b).unwrap()
}

/// Pick a human-readable name for an aligned column slot: prefer the source
/// header text (B's, then A's), falling back to the Excel-style letter.
fn column_display_name(k: usize, slot: &ColSlot, head_a: &[String], head_b: &[String]) -> String {
    let from_b = if slot.b >= 0 {
        head_b.get(slot.b as usize).map(|s| s.trim()).filter(|s| !s.is_empty())
    } else {
        None
    };
    let from_a = if slot.a >= 0 {
        head_a.get(slot.a as usize).map(|s| s.trim()).filter(|s| !s.is_empty())
    } else {
        None
    };
    from_b.or(from_a).map(|s| s.to_string()).unwrap_or_else(|| col_letter(k))
}

/// One column slot in the aligned grid. `a`/`b` are 0-based source column
/// indices, or -1 when the column exists only on the other side.
#[derive(Clone, Copy, PartialEq)]
struct ColSlot {
    a: isize,
    b: isize,
}

/// Re-align columns using row-matched data instead of headers. For each pair
/// of matched rows, compute a per-column similarity score (fraction of matched
/// rows where cells are equal), then greedily match columns by highest
/// similarity. This catches column correspondences that header-only matching
/// misses (renamed headers, missing headers, data-driven columns).
fn align_columns_by_data(
    rows_a: &[Vec<String>],
    rows_b: &[Vec<String>],
    row_pairs: &[RowPair],
    prev_slots: &[ColSlot],
    width_a: usize,
    width_b: usize,
) -> Vec<ColSlot> {
    let matched: Vec<(usize, usize)> = row_pairs
        .iter()
        .filter(|p| p.a >= 0 && p.b >= 0)
        .map(|p| (p.a as usize, p.b as usize))
        .collect();

    if matched.is_empty() {
        return prev_slots.to_vec();
    }

    // Compute per-column similarity matrix: sim[i][j] = fraction of matched
    // rows where A's column i equals B's column j.
    let col_sim = |ai: usize, bi: usize| -> f64 {
        let mut same = 0;
        for &(ra, rb) in &matched {
            let va = rows_a.get(ra).and_then(|r| r.get(ai)).map(|s| s.as_str()).unwrap_or("");
            let vb = rows_b.get(rb).and_then(|r| r.get(bi)).map(|s| s.as_str()).unwrap_or("");
            if va == vb {
                same += 1;
            }
        }
        same as f64 / matched.len() as f64
    };

    // Greedy assignment: repeatedly pick the highest-similarity (a_col, b_col)
    // pair, mark both as used, and create a matched slot. Remaining columns
    // become one-sided slots.
    let mut used_a = vec![false; width_a];
    let mut used_b = vec![false; width_b];
    let mut slots: Vec<ColSlot> = Vec::new();

    loop {
        let mut best_sim = 0.0f64;
        let mut best_ai: isize = -1;
        let mut best_bi: isize = -1;
        for (ai, &ua) in used_a.iter().enumerate().take(width_a) {
            if ua {
                continue;
            }
            for (bi, &ub) in used_b.iter().enumerate().take(width_b) {
                if ub {
                    continue;
                }
                let s = col_sim(ai, bi);
                if s > best_sim {
                    best_sim = s;
                    best_ai = ai as isize;
                    best_bi = bi as isize;
                }
            }
        }
        if best_ai < 0 || best_sim < 0.5 {
            break;
        }
        used_a[best_ai as usize] = true;
        used_b[best_bi as usize] = true;
        slots.push(ColSlot { a: best_ai, b: best_bi });
    }

    // Remaining unmatched columns → one-sided slots.
    for (ai, &used) in used_a.iter().enumerate().take(width_a) {
        if !used {
            slots.push(ColSlot { a: ai as isize, b: -1 });
        }
    }
    for (bi, &used) in used_b.iter().enumerate().take(width_b) {
        if !used {
            slots.push(ColSlot { a: -1, b: bi as isize });
        }
    }

    // Sort slots: matched first (in A order), then A-only, then B-only.
    slots.sort_by_key(|s| {
        let order = if s.a >= 0 && s.b >= 0 { 0 } else if s.a >= 0 { 1 } else { 2 };
        (order, s.a.max(0), s.b.max(0))
    });

    slots
}

/// align_columns_by_header matches A's columns to B's by LCS over the header-row
/// cells. When no usable header is found, falls back to positional 1:1 slots.
fn align_columns_by_header(
    rows_a: &[Vec<String>],
    rows_b: &[Vec<String>],
    width_a: usize,
    width_b: usize,
    header_a: usize,
    header_b: usize,
) -> Vec<ColSlot> {
    let head_a: &[String] = if header_a > 0 { &rows_a[header_a - 1] } else { &[] };
    let head_b: &[String] = if header_b > 0 { &rows_b[header_b - 1] } else { &[] };

    let usable = header_a > 0
        && header_b > 0
        && head_a.iter().filter(|c| !c.trim().is_empty()).count() >= 2
        && head_b.iter().filter(|c| !c.trim().is_empty()).count() >= 2;

    if !usable {
        let n = width_a.max(width_b);
        return (0..n)
            .map(|i| ColSlot {
                a: if i < width_a { i as isize } else { -1 },
                b: if i < width_b { i as isize } else { -1 },
            })
            .collect();
    }

    let norm = |s: &str| s.trim().to_lowercase();
    let ka: Vec<String> = (0..width_a)
        .map(|i| norm(head_a.get(i).map(|s| s.as_str()).unwrap_or("")))
        .collect();
    let kb: Vec<String> = (0..width_b)
        .map(|i| norm(head_b.get(i).map(|s| s.as_str()).unwrap_or("")))
        .collect();
    let ops = lcs_diff(&ka, &kb);

    let mut slots: Vec<ColSlot> = Vec::new();
    for op in &ops {
        match op.typ {
            OpType::Equal => slots.push(ColSlot { a: op.a as isize - 1, b: op.b as isize - 1 }),
            OpType::Delete => slots.push(ColSlot { a: op.a as isize - 1, b: -1 }),
            OpType::Insert => slots.push(ColSlot { a: -1, b: op.b as isize - 1 }),
            OpType::Replace => {}
        }
    }
    slots
}

#[derive(Clone, Copy, PartialEq)]
struct RowPair {
    a: isize,
    b: isize,
}

#[allow(dead_code)]
fn align_rows(
    rows_a: &[Vec<String>],
    rows_b: &[Vec<String>],
    slots: &[ColSlot],
    opts: &GridOptions,
) -> Vec<RowPair> {
    if rows_a.len() > opts.lcs_row_budget || rows_b.len() > opts.lcs_row_budget {
        return align_rows_by_position(rows_a, rows_b);
    }
    align_rows_by_lcs(rows_a, rows_b, slots, opts)
}

fn align_rows_by_position(rows_a: &[Vec<String>], rows_b: &[Vec<String>]) -> Vec<RowPair> {
    let n = rows_a.len().max(rows_b.len());
    let mut pairs = Vec::with_capacity(n);
    for i in 0..n {
        pairs.push(RowPair {
            a: if i < rows_a.len() { i as isize } else { -1 },
            b: if i < rows_b.len() { i as isize } else { -1 },
        });
    }
    pairs
}

fn align_rows_by_lcs(
    rows_a: &[Vec<String>],
    rows_b: &[Vec<String>],
    slots: &[ColSlot],
    opts: &GridOptions,
) -> Vec<RowPair> {
    let cols_a: Vec<usize> = slots.iter().filter(|s| s.a >= 0 && s.b >= 0).map(|s| s.a as usize).collect();
    let cols_b: Vec<usize> = slots.iter().filter(|s| s.a >= 0 && s.b >= 0).map(|s| s.b as usize).collect();
    let sig_a = signatures(rows_a, &cols_a);
    let sig_b = signatures(rows_b, &cols_b);
    let ops = lcs_diff(&sig_a, &sig_b);

    let mut pairs: Vec<RowPair> = Vec::new();
    let mut pending_del: Vec<usize> = Vec::new();
    let mut pending_ins: Vec<usize> = Vec::new();

    for op in &ops {
        match op.typ {
            OpType::Equal => {
                pairs.extend(repair_gap(&pending_del, &pending_ins, rows_a, rows_b, slots, opts));
                pending_del.clear();
                pending_ins.clear();
                pairs.push(RowPair { a: op.a as isize - 1, b: op.b as isize - 1 });
            }
            OpType::Delete => pending_del.push(op.a - 1),
            OpType::Insert => pending_ins.push(op.b - 1),
            OpType::Replace => {}
        }
    }
    pairs.extend(repair_gap(&pending_del, &pending_ins, rows_a, rows_b, slots, opts));
    pairs
}

/// repair_gap matches deleted rows to inserted rows by similarity, turning close
/// matches into modified pairs; leftovers stay pure delete/insert.
fn repair_gap(
    dels: &[usize],
    ins: &[usize],
    rows_a: &[Vec<String>],
    rows_b: &[Vec<String>],
    slots: &[ColSlot],
    opts: &GridOptions,
) -> Vec<RowPair> {
    let mut used_ins = vec![false; ins.len()];
    let mut match_of_del: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    for &ai in dels {
        let mut best_j: isize = -1;
        let mut best_sim = 0.0f64;
        for (j, &bi) in ins.iter().enumerate() {
            if used_ins[j] {
                continue;
            }
            let sim = row_similarity(&rows_a[ai], &rows_b[bi], slots);
            if sim > best_sim {
                best_sim = sim;
                best_j = j as isize;
            }
        }
        if best_j >= 0 && best_sim >= opts.row_similarity_threshold {
            used_ins[best_j as usize] = true;
            match_of_del.insert(ai, ins[best_j as usize]);
        }
    }

    let mut pairs: Vec<RowPair> = Vec::new();
    for &ai in dels {
        if let Some(&bi) = match_of_del.get(&ai) {
            pairs.push(RowPair { a: ai as isize, b: bi as isize });
        } else {
            pairs.push(RowPair { a: ai as isize, b: -1 });
        }
    }
    for (j, &bi) in ins.iter().enumerate() {
        if !used_ins[j] {
            pairs.push(RowPair { a: -1, b: bi as isize });
        }
    }
    pairs
}

/// signatures joins each row's cells at the given column indices (the columns
/// common to both sides), so rows are compared on aligned columns only.
fn signatures(rows: &[Vec<String>], cols: &[usize]) -> Vec<String> {
    rows.iter()
        .map(|r| {
            cols.iter()
                .map(|&c| r.get(c).map(|s| s.as_str()).unwrap_or(""))
                .collect::<Vec<_>>()
                .join("\u{0}")
        })
        .collect()
}

/// row_similarity compares two rows over the aligned common columns only.
fn row_similarity(ra: &[String], rb: &[String], slots: &[ColSlot]) -> f64 {
    let common: Vec<&ColSlot> = slots.iter().filter(|s| s.a >= 0 && s.b >= 0).collect();
    if common.is_empty() {
        return 1.0;
    }
    let mut same = 0;
    for s in &common {
        let va = ra.get(s.a as usize).map(|x| x.as_str()).unwrap_or("");
        let vb = rb.get(s.b as usize).map(|x| x.as_str()).unwrap_or("");
        if va == vb {
            same += 1;
        }
    }
    same as f64 / common.len() as f64
}

fn build_grid_row(rp: RowPair, rows_a: &[Vec<String>], rows_b: &[Vec<String>], slots: &[ColSlot]) -> GridRow {
    let ra: &[String] = if rp.a >= 0 { &rows_a[rp.a as usize] } else { &[] };
    let rb: &[String] = if rp.b >= 0 { &rows_b[rp.b as usize] } else { &[] };
    let mut gr = GridRow {
        status: Status::Equal,
        row_a: if rp.a >= 0 { rp.a as usize + 1 } else { 0 },
        row_b: if rp.b >= 0 { rp.b as usize + 1 } else { 0 },
        header: false,
        cells: Vec::with_capacity(slots.len()),
    };

    let row_status = if rp.a < 0 {
        Status::Added
    } else if rp.b < 0 {
        Status::Removed
    } else {
        Status::Equal
    };
    gr.status = row_status;

    let get = |row: &[String], idx: isize| -> String {
        if idx < 0 {
            String::new()
        } else {
            row.get(idx as usize).cloned().unwrap_or_default()
        }
    };

    let empty_segs = || (Vec::new(), Vec::new());

    let mut modified = false;
    for slot in slots {
        let va = get(ra, slot.a);
        let vb = get(rb, slot.b);
        let cc = if slot.b < 0 {
            let (s1, s2) = empty_segs();
            CellChange { status: Status::Removed, old: va, new: String::new(), old_segs: s1, new_segs: s2 }
        } else if slot.a < 0 {
            let (s1, s2) = empty_segs();
            CellChange { status: Status::Added, old: String::new(), new: vb, old_segs: s1, new_segs: s2 }
        } else {
            match row_status {
                Status::Added => {
                    let (s1, s2) = empty_segs();
                    CellChange { status: Status::Added, old: String::new(), new: vb, old_segs: s1, new_segs: s2 }
                }
                Status::Removed => {
                    let (s1, s2) = empty_segs();
                    CellChange { status: Status::Removed, old: va, new: String::new(), old_segs: s1, new_segs: s2 }
                }
                _ => {
                    if va != vb {
                        modified = true;
                        let (old_segs, new_segs) = inline_segments(&va, &vb, DEFAULT_SIMILARITY)
                            .unwrap_or_else(empty_segs);
                        CellChange { status: Status::Modified, old: va, new: vb, old_segs, new_segs }
                    } else {
                        let (s1, s2) = empty_segs();
                        CellChange { status: Status::Equal, old: String::new(), new: vb, old_segs: s1, new_segs: s2 }
                    }
                }
            }
        };
        gr.cells.push(cc);
    }
    if gr.status == Status::Equal && modified {
        gr.status = Status::Modified;
    }
    gr
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(rows: &[&[&str]]) -> Vec<Vec<String>> {
        rows.iter().map(|r| r.iter().map(|s| s.to_string()).collect()).collect()
    }

    fn counts(gd: &GridDiff) -> (usize, usize, usize, usize) {
        let (mut a, mut r, mut m, mut e) = (0, 0, 0, 0);
        for row in &gd.rows {
            match row.status {
                Status::Added => a += 1,
                Status::Removed => r += 1,
                Status::Modified => m += 1,
                Status::Equal => e += 1,
            }
        }
        (a, r, m, e)
    }

    #[test]
    fn col_letter_basic() {
        assert_eq!(col_letter(0), "A");
        assert_eq!(col_letter(25), "Z");
        assert_eq!(col_letter(26), "AA");
        assert_eq!(col_letter(27), "AB");
    }

    #[test]
    fn inserted_row_no_cascade() {
        let a = grid(&[&["name", "amount"], &["Alice", "100"], &["Bob", "200"], &["Carl", "300"], &["Dave", "400"]]);
        let b = grid(&[&["name", "amount"], &["Alice", "100"], &["Bob", "200"], &["NEW", "999"], &["Carl", "300"], &["Dave", "400"]]);
        let gd = grid_diff(&a, &b, &GridOptions::default());
        let (a_n, r, m, e) = counts(&gd);
        assert_eq!((a_n, m, r), (1, 0, 0), "inserted row cascaded");
        assert_eq!(e, 5, "want 5 equal rows");
    }

    #[test]
    fn single_cell_edit_is_modified() {
        let a = grid(&[&["id", "v"], &["1", "a"], &["2", "b"], &["3", "c"]]);
        let b = grid(&[&["id", "v"], &["1", "a"], &["2", "CHANGED"], &["3", "c"]]);
        let gd = grid_diff(&a, &b, &GridOptions::default());
        let (a_n, r, m, e) = counts(&gd);
        assert_eq!((m, a_n, r), (1, 0, 0));
        assert_eq!(e, 3);
    }

    #[test]
    fn sparse_header_layout_aligns_columns() {
        // A layout with a title block above the table header: the detector must
        // still find the real header row and align all four columns.
        let a = grid(&[
            &["Inventory Report"],
            &["Region:", "", "", "EMEA"],
            &[],
            &["Sku", "Description", "Category", "Stock"],
            &["A001", "Widget", "Tools", "20"],
            &["A002", "Gadget", "Toys", "2"],
        ]);
        let b = grid(&[
            &["Inventory Report"],
            &["Region:", "", "", "EMEA"],
            &[],
            &["Sku", "Description", "Category", "Stock"],
            &["A001", "Widget", "Tools", "21"],
            &["A002", "Gadget", "Toys", "2"],
        ]);
        let gd = grid_diff(&a, &b, &GridOptions::default());
        assert_eq!(gd.columns.len(), 4, "want 4 columns");
        assert_eq!(gd.columns[0].name, "Sku");
        assert_eq!(gd.columns[3].name, "Stock");
        let (a_n, r, m, e) = counts(&gd);
        assert_eq!((m, a_n, r), (1, 0, 0));
        assert_eq!(e, 5);
        for row in &gd.rows {
            if row.status == Status::Modified {
                assert_eq!(row.cells.len(), 4);
                assert_eq!(row.cells[3].status, Status::Modified);
                assert_eq!(row.cells[3].old, "20");
                assert_eq!(row.cells[3].new, "21");
            }
        }
    }

    #[test]
    fn appended_row_is_added() {
        let a = grid(&[&["Sku", "Category"], &["A001", "Tools"]]);
        let b = grid(&[&["Sku", "Category"], &["A001", "Tools"], &["A002", "Toys"]]);
        let gd = grid_diff(&a, &b, &GridOptions::default());
        let (a_n, r, m, _) = counts(&gd);
        assert_eq!((a_n, r, m), (1, 0, 0));
    }

    #[test]
    fn inserted_column_detected() {
        let a = grid(&[&["a", "c"], &["1", "3"], &["4", "6"]]);
        let b = grid(&[&["a", "b", "c"], &["1", "2", "3"], &["4", "5", "6"]]);
        let gd = grid_diff(&a, &b, &GridOptions::default());
        assert!(gd.added_cols >= 1, "inserted column should be detected");
    }

    #[test]
    fn empty_grids() {
        let a: Vec<Vec<String>> = Vec::new();
        let b: Vec<Vec<String>> = Vec::new();
        let gd = grid_diff(&a, &b, &GridOptions::default());
        assert!(gd.rows.is_empty());
        assert_eq!(gd.added_rows + gd.removed_rows + gd.modified_rows, 0);
    }

    #[test]
    fn identical_grids_all_equal() {
        let a = grid(&[&["h1", "h2"], &["x", "y"], &["1", "2"]]);
        let b = grid(&[&["h1", "h2"], &["x", "y"], &["1", "2"]]);
        let gd = grid_diff(&a, &b, &GridOptions::default());
        let (a_n, r, m, e) = counts(&gd);
        assert_eq!((a_n, r, m, e), (0, 0, 0, 3));
    }

    #[test]
    fn row_budget_falls_back_to_positional_with_note() {
        let mut big_a: Vec<Vec<String>> = Vec::new();
        let mut big_b: Vec<Vec<String>> = Vec::new();
        for i in 0..5 {
            big_a.push(vec![format!("r{i}"), "x".into()]);
            big_b.push(vec![format!("r{i}"), "x".into()]);
        }
        let opts = GridOptions { lcs_row_budget: 2, ..GridOptions::default() };
        let gd = grid_diff(&big_a, &big_b, &opts);
        assert!(!gd.notes.is_empty(), "expected a fallback note");
        assert!(gd.notes.iter().any(|n| n.contains("position")), "note should mention positional: {:?}", gd.notes);
    }

    #[test]
    fn asymmetric_widths_detect_header_independently() {
        let a = grid(&[&["h1", "h2", "h3", "h4"], &["1", "2", "3", "4"]]);
        let b = grid(&[&["h1"], &["1"]]);
        let gd = grid_diff(&a, &b, &GridOptions::default());
        assert!(gd.rows.iter().any(|r| r.header), "header should be detected on both sides independently");
    }

    #[test]
    fn column_names_use_header_text_when_available() {
        let a = grid(&[&["Name", "Value"], &["x", "1"]]);
        let b = grid(&[&["Name", "Value"], &["x", "2"]]);
        let gd = grid_diff(&a, &b, &GridOptions::default());
        assert_eq!(gd.columns[0].name, "Name");
        assert_eq!(gd.columns[1].name, "Value");
    }
}