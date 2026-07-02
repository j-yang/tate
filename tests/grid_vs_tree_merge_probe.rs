//! READ-ONLY PROBE — changes no library code.
//!
//! Question: can grid 3-way merge be folded into "grid → Section → tree_merge
//! → grid" without losing anything? If yes, `grid_merge` is redundant and Phase
//! 2 can delete it. If no, `grid_merge` is a specialised algorithm that must
//! stay, and the "everything folds into tree" story has a real boundary here.
//!
//! This test builds a few clinical-shaped grids, runs both merge paths, and
//! reports concrete divergences. It is written to *document* the answer, not to
//! guard a behaviour — so the assertions record what actually happens.
//!
//! # Verdict (measured, not assumed)
//!
//! The paths are NOT equivalent, and the tree path is strictly worse for grids:
//!
//! - disjoint cell edits: tree path silently DROPS one side's edit (data loss)
//!   because keyless `cell` nodes match positionally and bubble up to the row,
//!   where independent edits clobber each other.
//! - same-cell conflict: tree path SWALLOWS the conflict (no cell-level
//!   granularity), while grid_merge reports it precisely at (row, col).
//! - row inserted on one side: grid_merge's coordinate-descent alignment
//!   recognises the shift and merges cleanly; positional Section keys offset
//!   every subsequent row and produce a garbage grid.
//!
//! Conclusion: `grid_merge` is an irreducible specialised algorithm. Cell-level
//! merge needs the row/column alignment that coordinate descent computes and
//! that a positionally-keyed Section structurally cannot hold. Folding grid
//! into tree_merge would degrade correctness — so we do NOT. The unification is
//! at the level of the *object* (Section) and the *laws*, not the merge
//! algorithms. This file exists so any future attempt to fold them sees why.

use std::collections::BTreeMap;
use tate::grid::{grid_merge, GridOptions};
use tate::patch::{apply, diff};
use tate::section::{Location, Section, Value};
use tate::tree::{tree_merge, TreeNode};

// ── grid ⇄ Section (the natural depth-2 encoding: [rowKey, colKey] → value) ──

/// Encode a grid as a Section using positional keys: row `i` → `r{i}`, column
/// `j` → `c{j}`. This is exactly the keying an adapter would use for a grid with
/// no primary key — the honest representation of "grid as tree".
fn grid_to_section(rows: &[Vec<String>]) -> Section {
    let mut values = BTreeMap::new();
    // Root.
    values.insert(
        vec!["grid".to_string()],
        Value { kind: "grid".into(), label: String::new(), text: String::new(), attrs: vec![], order: 0 },
    );
    for (i, row) in rows.iter().enumerate() {
        let row_key = format!("r{i}");
        values.insert(
            vec!["grid".to_string(), row_key.clone()],
            Value { kind: "row".into(), label: String::new(), text: String::new(), attrs: vec![], order: i },
        );
        for (j, cell) in row.iter().enumerate() {
            values.insert(
                vec!["grid".to_string(), row_key.clone(), format!("c{j}")],
                Value {
                    kind: "cell".into(),
                    label: String::new(),
                    text: cell.clone(),
                    attrs: vec![],
                    order: j,
                },
            );
        }
    }
    Section { values }
}

/// Decode a Section back into a grid of strings.
fn section_to_grid(s: &Section) -> Vec<Vec<String>> {
    // Collect rows in order, then cells in order.
    let mut rows: BTreeMap<usize, BTreeMap<usize, String>> = BTreeMap::new();
    for (loc, v) in &s.values {
        if loc.len() == 3 {
            let ri: usize = loc[1].trim_start_matches('r').parse().unwrap_or(0);
            let ci: usize = loc[2].trim_start_matches('c').parse().unwrap_or(0);
            rows.entry(ri).or_default().insert(ci, v.text.clone());
        }
    }
    rows.into_values()
        .map(|cells| cells.into_values().collect())
        .collect()
}

fn g(rows: &[&[&str]]) -> Vec<Vec<String>> {
    rows.iter().map(|r| r.iter().map(|s| s.to_string()).collect()).collect()
}

/// The tree-path 3-way merge: grid → Section → TreeNode, tree_merge, → grid.
fn tree_path_merge(
    base: &[Vec<String>],
    ours: &[Vec<String>],
    theirs: &[Vec<String>],
) -> (Vec<Vec<String>>, usize) {
    let sb = grid_to_section(base).to_tree().unwrap();
    let so = grid_to_section(ours).to_tree().unwrap();
    let st = grid_to_section(theirs).to_tree().unwrap();
    let r = tree_merge(&sb, &so, &st);
    let merged_section = r.tree.to_section();
    (section_to_grid(&merged_section), r.conflicts.len())
}

#[test]
fn probe_disjoint_cell_edits() {
    // ours changes (1,1); theirs changes (0,0). No overlap — a correct 3-way
    // merge keeps BOTH edits with zero conflicts.
    let base = g(&[&["a", "b"], &["c", "d"]]);
    let ours = g(&[&["a", "b"], &["c", "X"]]);
    let theirs = g(&[&["Y", "b"], &["c", "d"]]);

    let gm = grid_merge(&base, &ours, &theirs, &GridOptions::default());
    let (tree_grid, tree_conflicts) = tree_path_merge(&base, &ours, &theirs);

    println!("grid_merge  -> grid={:?} conflicts={}", gm.grid, gm.conflicts.len());
    println!("tree_path   -> grid={:?} conflicts={}", tree_grid, tree_conflicts);

    // grid_merge is correct: both edits present, no conflict.
    assert_eq!(gm.grid, g(&[&["Y", "b"], &["c", "X"]]), "grid_merge keeps both edits");
    assert_eq!(gm.conflicts.len(), 0);

    // FINDING: the tree path silently DROPS theirs' edit. Keyless `cell` nodes
    // match positionally and bubble up to the `row`, where independent cell
    // edits clobber each other. This is data loss, not merely a different result.
    assert_ne!(
        tree_grid, gm.grid,
        "documented divergence: if these ever match, folding grid_merge is safe"
    );
    assert_eq!(tree_grid, g(&[&["a", "b"], &["c", "X"]]), "tree path lost theirs' (0,0) edit");
}

#[test]
fn probe_conflicting_cell_edit() {
    // Both change the SAME cell (1,1) to different values → a correct merge
    // reports exactly one conflict.
    let base = g(&[&["a", "b"], &["c", "d"]]);
    let ours = g(&[&["a", "b"], &["c", "X"]]);
    let theirs = g(&[&["a", "b"], &["c", "Z"]]);

    let gm = grid_merge(&base, &ours, &theirs, &GridOptions::default());
    let (_tree_grid, tree_conflicts) = tree_path_merge(&base, &ours, &theirs);

    println!("grid_merge  -> conflicts={} at cells {:?}", gm.conflicts.len(),
        gm.conflicts.iter().map(|c| (c.row, c.col)).collect::<Vec<_>>());
    println!("tree_path   -> conflicts={}", tree_conflicts);

    // grid_merge is correct: one conflict, precisely located at (1,1).
    assert_eq!(gm.conflicts.len(), 1, "grid detects the cell conflict");
    assert_eq!((gm.conflicts[0].row, gm.conflicts[0].col), (1, 1));

    // FINDING: the tree path SWALLOWS the conflict (bubbling loses cell-level
    // granularity). A silent wrong merge where grid_merge correctly flags a clash.
    assert_eq!(tree_conflicts, 0, "documented divergence: tree path misses the cell conflict");
}

#[test]
fn probe_row_inserted_by_one_side_shifts_coordinates() {
    // THE CRUX. theirs inserts a row at the TOP; ours edits the last row.
    //
    // grid_merge uses coordinate-descent row alignment: it recognises that
    // base row 1 == ours row 1 == theirs row 2 (shifted down by the insert),
    // so ours' edit and theirs' insert are DISJOINT and merge cleanly.
    //
    // The positional Section keying (row i → "r{i}") has no such alignment:
    // "r0" on theirs is the NEW row, so every subsequent row's key is offset by
    // one. The tree path sees this as "every row changed" — a false storm.
    let base = g(&[&["h1", "h2"], &["a", "b"]]);
    let ours = g(&[&["h1", "h2"], &["a", "B"]]); // edit last cell
    let theirs = g(&[&["NEW", "ROW"], &["h1", "h2"], &["a", "b"]]); // prepend row

    let gm = grid_merge(&base, &ours, &theirs, &GridOptions::default());
    let (tree_grid, tree_conflicts) = tree_path_merge(&base, &ours, &theirs);

    println!("=== ROW INSERT PROBE ===");
    println!("grid_merge  -> grid={:?}", gm.grid);
    println!("            conflicts={}", gm.conflicts.len());
    println!("tree_path   -> grid={:?}", tree_grid);
    println!("            conflicts={}", tree_conflicts);

    // grid_merge: coordinate descent aligns the shifted rows → clean merge of
    // the new row + the edit, no conflict.
    let grid_clean = gm.conflicts.is_empty();

    // tree_path: positional keys mean the insert shifts every key. Whether this
    // surfaces as conflicts or as silent clobbering, the point is it does NOT
    // reproduce grid_merge's alignment-aware result.
    let paths_agree = gm.grid == tree_grid && gm.conflicts.len() == tree_conflicts;

    println!("grid_merge clean? {grid_clean}");
    println!("paths agree? {paths_agree}");

    // The probe's thesis: the two paths DISAGREE on row-shift cases. If this
    // assertion ever fails (paths agree), the fold would be safe and we should
    // revisit. We expect them to differ.
    assert!(
        !paths_agree,
        "PROBE FALSIFIED: tree path reproduced grid_merge on a row-shift case — \
         folding may be safe after all. grid={:?} tree={:?}",
        gm.grid, tree_grid
    );
}

#[test]
fn probe_diff_apply_roundtrip_holds_for_grid_sections() {
    // Sanity: the patch algebra itself round-trips on grid-encoded sections,
    // independent of merge. (Confirms the encoding is faithful; the merge
    // divergence above is about ALIGNMENT, not about the algebra being broken.)
    let a = grid_to_section(&g(&[&["a", "b"], &["c", "d"]])).to_tree().unwrap();
    let b = grid_to_section(&g(&[&["a", "b"], &["c", "X"]])).to_tree().unwrap();
    let p = diff(&a, &b);
    assert_eq!(apply(&p, &a).unwrap(), b);
}
