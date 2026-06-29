# tate: Algorithm Design Document

## Overview

tate is a self-contained structured diff library with four algorithmic modules. Each module addresses a different granularity of structured comparison: lines, inline word-level highlighting, 2D grid alignment, and tree structure. This document describes the algorithm design, current limitations, and theoretical gaps for each module.

---

## Module 1: `lines` — Line-Level Diff

### Problem

Given two sequences of strings A = [a₁, ..., aₙ] and B = [b₁, ..., bₘ], produce an edit script of Equal/Delete/Insert operations that transforms A into B.

### Algorithm: Patience + LCS + Hirschberg Pipeline

The pipeline runs in three stages, choosing the appropriate algorithm by input size:

```
input → strip common prefix/suffix → patience_anchors → per-segment solver
```

**Stage 0: Common prefix/suffix stripping**

Before any diffing, matching prefix and suffix lines are stripped. This is O(min(n,m)) and handles the common case (small change in large file) almost entirely — the remaining "middle" is often tiny.

**Stage 1: Patience anchoring**

Patience diff (Bram Cohen, 2003; adopted by Git in 2009):

1. Find lines that appear exactly once in both A and B ("unique common lines").
2. These form a set of match pairs (aᵢ, bⱼ).
3. Compute the Longest Increasing Subsequence (LIS) of the b-coordinates, ordered by a-coordinate. This gives a set of anchor pairs that are in-order in both sequences.
4. Each anchor pair splits the problem into independent sub-problems (the region before the anchor, and after).

The LIS is computed via patience sorting: O(k log k) where k is the number of unique common lines.

**Why patience matters:** For structured documents (tables, code, reports), unique anchor lines (section headers, row IDs, fixed-format tokens) are abundant. Patience splits a large diff into many small independent sub-problems, each solved cheaply.

**Stage 2: Exact LCS via full DP**

For each patience-free segment where n·m ≤ 4,194,304 (4M cells):

- Build the full (n+1)×(m+1) DP table: O(n·m) time and space.
- Backtrack to recover the edit script.
- Produces a provably optimal (minimum-edit) script for the segment.

**Stage 3: Hirschberg linear-space LCS**

For segments where 4M < n·m ≤ 16,777,216 (16M cells):

- Hirschberg's algorithm (1975): O(n·m) time, O(min(n,m)) space.
- Recursively splits A in half, computes the optimal split point in B via two rolling-row LCS passes, recurses.
- Produces the same optimal result as full DP, just using less memory.

**Stage 4: Block replacement bailout**

For segments where n·m > 16M (e.g., two 4000-line files where almost every line differs):

- Stop trying to find an optimal alignment.
- Emit: delete all of A, insert all of B.
- This is correct (the script transforms A into B) but not minimal.
- Purpose: prevent UI freezes on pathological inputs. Without this, Hirschberg would take tens of seconds on adversarial inputs.

### Theoretical properties

- **Correctness:** The edit script always transforms A into B (verified by `validate` in tests).
- **Optimality:** Optimal (minimum edit distance) for segments ≤ 16M cells. Not guaranteed optimal for bailed-out segments.
- **Complexity:** O(n·m) worst case per segment, but patience anchoring typically reduces segment sizes dramatically. Space: O(n·m) for small segments, O(min(n,m)) for Hirschberg, O(1) for bailouts.
- **No known theoretical gap.** This pipeline is a standard, well-understood combination of textbook algorithms.

### Limitations (engineering, not theoretical)

- Patience anchoring degrades when few unique common lines exist (e.g., files of random UUIDs).
- The 4M/16M thresholds are hardcoded heuristics, not self-tuning.
- No deadline-based termination (Hirschberg's recursion depth is data-dependent).

---

## Module 2: `inline` — Word-Level Highlighting

### Problem

Given a raw edit script of Equal/Delete/Insert operations, merge adjacent delete+insert blocks into `Replace` operations carrying word-level inline segments, so a line like `"foo bar"` → `"foo baz"` shows as one modified row highlighting only `"baz"`.

### Algorithm: Positional Pairing + Word-Level LCS

**`pair_replacements(ops, threshold)`:**

1. Walk the edit script. When encountering a maximal block of Deletes followed by a maximal block of Inserts:
2. Pair them positionally: dels[0]↔inss[0], dels[1]↔inss[1], ...
3. For each pair, compute word-level similarity via `inline_segments`.
4. If similarity ≥ threshold (default 0.5): emit a `Replace` op with inline segments.
5. If below threshold: keep as separate Delete + Insert (too different to be a "modification").

**`inline_segments(a, b, threshold)`:**

1. Tokenize both lines into alphanumeric/non-alphanumeric runs (preserving the original text exactly when joined).
2. Run a small LCS DP over the token sequences.
3. Compute Sørensen–Dice similarity: `2 × |equal_chars| / (|a_chars| + |b_chars|)`.
4. If below threshold: return None (caller keeps them as separate del+ins).
5. Otherwise: build segment lists by coalescing adjacent same-tag runs.

### Theoretical properties

- **Correctness:** The output script is equivalent to the input (same transformation A→B).
- **Similarity metric:** Sørensen–Dice coefficient, a standard set-similarity measure.
- **Pairing strategy:** Positional (1st-with-1st). Not globally optimal — see Gap below.

### Gap: Positional pairing is suboptimal

When delete and insert block sizes differ (e.g., 3 deletes vs 2 inserts), positional pairing misses optimal pairings.

Example:
```
dels: ["foo bar",     "unrelated",     "foo baz"]
inss:                              ["foo baz", "foo bar"]
```
Positional: "foo bar"↔"foo baz" (similar), "unrelated"↔"foo bar" (dissimilar)
Optimal:    "foo bar"↔"foo bar" (identical), "foo baz"↔"foo baz" (identical), "unrelated"↔nothing

The optimal pairing is solvable via the assignment problem (Hungarian algorithm, O(n³)), but:
- In practice, diff blocks are small (1-10 lines), so the difference is negligible.
- The case (unequal del/ins counts) is uncommon in structured document diffs.
- **Not a priority for improvement.**

---

## Module 3: `grid` — 2D Grid Alignment

### Problem

Given two tables A (m₁ × n₁) and B (m₂ × n₂) of strings, produce one aligned grid with per-cell status (equal/modified/added/removed), detecting row and column insertions, deletions, and modifications.

### Algorithm: Coordinate Descent (v0.2.0)

The **table edit distance** problem is NP-hard (reduction from Maximum Biclique, see formalization below). tate approximates it via **coordinate descent** — alternating between two polynomial-time sub-problems until convergence.

**Step 1: Initialize alignment**

Default: **positional** (identity) alignment — row i ↔ row i, column j ↔ column j. Zero assumption.

When the caller provides `Init::Header { a, b }`: LCS on header text seeds the initial column alignment. The header is a **prior** (informed starting point), not a constraint — the algorithm still runs coordinate descent afterward and may override it.

**Step 2: Coordinate descent loop**

```
for _ in 0..max_iters:
    new_rows = align_1d(row_keys, row_similarity)    # fix columns, optimize rows
    new_cols = align_1d(col_keys, col_similarity)    # fix rows, optimize columns
    if new_cols == col_align && new_rows == row_align: break  # converged
```

Each `align_1d` call:
1. Compute hash keys per element (row or column). Two elements with the same key are equal across all aligned positions — LCS can compare keys in O(1).
2. Run LCS on keys → Equal / Delete / Insert.
3. **Repair gap**: for each Delete+Insert block, greedily match elements by similarity (fraction of equal cells). Pairs at or above the cost-derived threshold become Modified; leftovers stay pure delete/insert.

Rows and columns use the **same** `align_1d` subroutine — they are duals.

**Step 3: Render**

For each row pair, walk the column pairs and produce `CellChange` per cell:
- Both sides present, cells differ → Modified (with word-level inline segments via `inline_segments`)
- Both sides present, cells equal → Equal
- Only A → Removed
- Only B → Added

Tables exceeding `max_rows` (default 4000) skip coordinate descent entirely and use positional alignment only.

### Cost model

All thresholds are derived from three cost parameters — no magic numbers:

```rust
pub struct Cost {
    pub row: f64,    // α — insert/delete a row
    pub col: f64,    // β — insert/delete a column
    pub cell: f64,   // γ — modify a cell
}
```

**Modify threshold**: a Delete+Insert pair becomes Modified when modification is cheaper than delete+insert:

```
(1 − similarity) × n × γ < 2α
⟺  similarity > 1 − 2α / (nγ)
```

With default costs (α=γ=1):

| Aligned columns (n) | Threshold | Interpretation |
|---------------------|-----------|----------------|
| 1 | −1 | Always pair (modify always cheaper) |
| 2 | 0 | Pair if any cell matches |
| 4 | 0.5 | Pair if ≥50% cells match |
| 10 | 0.8 | Pair if ≥80% cells match |

### Theoretical properties

- **Each sub-problem is optimal**: given fixed column alignment, `align_1d` produces the minimum-edit row script (LCS is optimal for 1D). Symmetrically for columns.
- **Convergence**: coordinate descent on a finite space with monotonically decreasing cost → converges in ≤ `max_iters` steps to a local optimum.
- **No global optimality guarantee**: the joint problem is NP-hard. The local optimum quality depends on initialization (positional vs header).
- **Header is a prior, not a constraint**: the algorithm's correctness and convergence do not depend on header detection. A better prior merely accelerates convergence and avoids bad local optima.

---

### Formalization: Table Edit Distance

**Definition.** Given tables T₁=(R₁,C₁,f₁) and T₂=(R₂,C₂,f₂), the *table edit distance* τ(T₁,T₂) is the minimum cost of a sequence of operations transforming T₁ into T₂, with operations:

| Operation | Cost | Effect |
|-----------|------|--------|
| insert_row | α | Add a row |
| delete_row | α | Remove a row |
| insert_col | β | Add a column |
| delete_col | β | Remove a column |
| modify_cell | γ | Change a cell value |

**Metric axioms.** τ satisfies non-negativity, identity, symmetry, and the triangle inequality (standard concatenation argument).

**1D compatibility.** When tables have 1 column, τ reduces to Levenshtein distance.

**NP-hardness.** The basic (order-preserving) version is NP-hard via reduction from Maximum Biclique:

- Construct A = adjacency matrix of bipartite graph G=(U,V,E), B = k×k all-ones matrix.
- Set α=β=M (M > |U|·|V|), γ=1.
- Large M forces matching exactly k rows and k columns.
- Cell cost = zeros in the matched k×k submatrix of A.
- τ(A,B) ≤ M·(|U|-k) + M·(|V|-k) ⟺ G has a k×k biclique.

**FPT result.** When the column count is small (m₁,m₂ ≤ 20), the problem is fixed-parameter tractable: enumerate all O(2^(m₁+m₂)) column alignments, solve each with a 1D row DP, take the minimum. Time: O(2^(m₁+m₂) · n₁·n₂).

**Gap 1: No move detection (row/column reordering)**

**The problem:** LCS finds the longest common *subsequence*, not *subset*. If columns are reordered (same set, different order), LCS reports some as deleted and re-added.

**Concrete failure:**
```
A: | name | age  | role  |     B: | name | role  | age  |
```
LCS: [name] only (age and role are in reversed order).
Result: 2 columns deleted + 2 columns added = 4 column changes.
Reality: 0 changes (columns just reordered).

**Root cause:** The Table Edit Distance model has no "move" operation. Adding one would make the problem even harder (related to minimum common string partition). An order-free matching mode (e.g., Hungarian algorithm) could be added as an alternative to LCS in `align_1d`.

---

**Gap 2: Local optima in coordinate descent**

Coordinate descent converges to a **local** optimum, not necessarily global. With positional initialization, pathological tables (massive column reordering) may converge to a bad local optimum. The `Init::Header` prior mitigates this for tables with headers.

**Potential mitigation:** Multiple random restarts, or FPT exact solver for small column counts (see formalization above).

---

## Module 4: `tree` — Structural Tree Diff

### Problem

Given two tree nodes A and B (in the `TreeNode` intermediate representation), produce a list of changes (Added/Removed/Modified) that describes how A's tree differs from B's tree.

### Algorithm: Recursive Identity-Keyed Matching

```
diff_node(a, b):
    1. Compare tag names, attributes, text.
    2. Match children by key = kind + "#" + identity (or just kind if no identity).
    3. For matched children: recurse.
    4. For unmatched B-children: emit Added (and their locatable descendants).
    5. For unmatched A-children: emit Removed (and their locatable descendants).
    6. Changes in keyless descendants bubble up to nearest locatable ancestor.
```

### Key concepts

- **Locatable node:** has `identity` set (e.g., XML OID attr, JSON object key). Appears in the change list on its own.
- **Keyless node:** no identity. Matched positionally among siblings of the same kind. Changes bubble up.
- **Bubble-up:** if a keyless child changes, the parent (if locatable) is reported as Modified. The keyless child itself does not appear in the change list.

### Theoretical properties

- **Correctness:** Every difference between A and B is detected (the walk is exhaustive).
- **Matching quality:** Identity-keyed matching is optimal when identities are unique within siblings. Degrades to positional when identities are missing or duplicated.
- **Complexity:** O(|A| + |B|) per recursion level. Total: O(|A| · |B|) worst case (deeply nested keyless trees).

### Gap: No move detection

If a node moves from one parent to another (same identity, different location in the tree), the current algorithm reports it as Removed from the old location + Added at the new location. A "Moved" change type would be more accurate.

Move detection requires a second pass: after the initial diff, scan all Removed and Added changes for matching identities, and reclassify them as Moved. Cost: O(removed × added) identity comparisons.

This is a known limitation but low-priority: in the primary use case (CDISC define.xml version comparison), nodes rarely change parent.

---

## Module 5: `lcs` (private) — Lightweight LCS for Word-Level Diff

A simple O(n·m) LCS DP with full matrix, used internally by `inline_segments` for word-level token alignment. Input is bounded (tokens of a single line, typically tens to hundreds), so the full matrix is appropriate — no patience anchoring or Hirschberg needed.

No theoretical gaps. This is the textbook LCS algorithm applied to a bounded input.

---

## Summary of Gaps by Priority

| Gap | Module | Impact | Academic value | Practical value |
|-----|--------|--------|----------------|-----------------|
| No formal table edit distance | grid | Blocks all theoretical analysis | **High** (fills a literature gap) | Medium |
| Sequential column→row dependency | grid | Propagates column errors to rows | **Medium** (iterative refinement is novel) | **High** |
| Column reordering = delete+add | grid | Noisy diff on rearranged tables | Medium (permutation detection) | **High** |
| Header-only column matching | grid | Misses renamed columns | Low | Medium |
| Positional pairing in inline | inline | Suboptimal for unequal del/ins blocks | Low | Low |
| No move detection in tree | tree | Move = delete+add | Low | Low |
