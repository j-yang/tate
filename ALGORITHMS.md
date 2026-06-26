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

Given two tables (2D grids of strings) A = [n×k] and B = [m×l], produce one aligned grid with per-cell status (equal/modified/added/removed), detecting row insertions/deletions/modifications and column insertions/deletions.

### Algorithm: Sequential Column-Then-Row LCS

```
detect_header → align_columns (header LCS) → align_rows (signature LCS) → repair_gap → render
```

**Step 1: Header detection**

Find the first row that fills ≥ `header_fill_ratio` (default 0.8) of the grid width. This row's cells serve as column identity tokens.

Each side detects its header independently using its own width (not a shared max width).

**Step 2: Column alignment via header LCS**

- Normalize header cells: trim + lowercase.
- Run LCS on the two header sequences.
- LCS Equal → column slot with both a_col and b_col.
- LCS Delete → column exists only in A (removed).
- LCS Insert → column exists only in B (added).

If no usable header on either side: fall back to positional 1:1 column slots.

**Step 3: Row signature construction**

- For each row, extract cells at the aligned (common) column indices only.
- Join them with `\u{0}` (NUL) separator to form a signature string.
- This makes row comparison insensitive to inserted/removed columns.

**Step 4: Row alignment via signature LCS**

- Run LCS on the signature sequences.
- LCS Equal → rows matched (may still have cell-level differences in non-common columns).
- LCS Delete + Insert → collected into pending_del / pending_ins buffers.

**Step 5: Repair gap (greedy similarity pairing)**

For each pending delete/insert block:
- For each deleted row, find the best-matching unused inserted row by cell similarity.
- If best similarity ≥ `row_similarity_threshold` (default 0.5): pair as Modified.
- Leftover deletes: marked as Removed. Leftover inserts: marked as Added.

Cell similarity = fraction of common-column cells that are equal.

**Step 6: Render**

For each row pair, walk the column slots and produce CellChange per cell:
- Both sides present, cells differ → Modified
- Both sides present, cells equal → Equal
- Only A → Removed
- Only B → Added

If a row budget (`lcs_row_budget`, default 4000) is exceeded, skip LCS and align rows positionally.

### Theoretical properties

- **Row alignment is optimal** given fixed column alignment: LCS on signatures produces the minimum-edit row script.
- **Column alignment is greedy**: header LCS is not globally optimal for column matching.
- **No approximation guarantee**: the sequential approach (column → row) has no provable bound on how close the result is to an optimal joint alignment.

---

### Gap 1: Column reordering is reported as delete+add

**The problem:** LCS finds the longest common *subsequence* of header cells. If columns are reordered (same set, different order), LCS reports some as deleted and re-added.

**Concrete failure:**
```
A: | name | age  | role  |     B: | name | role  | age  |
```
Header LCS: [name] only (age and role are in reversed order, not a common subsequence).
Result: 2 columns deleted + 2 columns added = 4 column changes.
Reality: 0 changes (columns just reordered).

**Root cause:** LCS models "common subsequence" not "common set" or "permutation". It cannot detect that the same columns exist in a different order.

**Impact:** Any table where columns are rearranged between versions produces a noisy diff with many spurious column add/delete operations.

---

### Gap 2: No formal definition of "optimal table diff"

**The problem:** There is no accepted formal definition of table edit distance analogous to:
- Levenshtein distance for sequences (insert/delete/substitute, cost 1 each)
- Zhang-Shasha for trees (insert/delete/relable nodes)

Without a formal cost model, we cannot:
- Prove that grid_diff's output is within a factor of k of optimal.
- Compare different algorithms on a theoretical basis.
- Reason about which operations should be "cheap" vs "expensive".

**Proposed formalization:**

Define table edit distance τ(A, B) as the minimum cost of a sequence of operations transforming A into B, where the operation set is:

| Operation | Cost | Effect |
|-----------|------|--------|
| insert_row(i, values) | 1 | Add a row at position i |
| delete_row(i) | 1 | Remove row i |
| modify_cell(i, j, v) | 1 | Change cell (i,j) to v |
| insert_col(j, values) | 1 | Add a column at position j |
| delete_col(j) | 1 | Remove column j |
| swap_rows(i, i') | 1 | Exchange rows i and i' |
| swap_cols(j, j') | 1 | Exchange columns j and j' |

**Complexity:** Without swap operations, the problem decomposes into independent row and column alignment (each solvable by LCS in polynomial time). With swaps, the problem likely becomes NP-hard (related to the minimum common string partition problem). The exact complexity is an open question.

**Current algorithm's relationship to this definition:** grid_diff solves a restricted version (no swaps, column alignment via header LCS, row alignment via signature LCS). Its cost is an upper bound on τ(A, B), but the gap between grid_diff's cost and τ(A, B) is unbounded in the worst case (column reorder case above produces O(k) spurious changes when τ = 0).

---

### Gap 3: Sequential dependency propagates column errors to rows

**The problem:** Row alignment depends on column alignment. If columns are misaligned, row signatures are computed on wrong columns, causing row pairing errors.

**Concrete failure:**
```
A: | id | name   |     B: | id | label  |
   | 1  | Alice  |        | 1  | Alice  |
   | 2  | Bob    |        | 2  | Bobby  |
```
If header LCS fails to match "name"↔"label" (different text), column alignment treats them as delete+add.
Row signatures use only [id] → "1" and "2" match on both sides → row 2 appears Equal.
But "Bob" → "Bobby" should be a Modified cell — it's invisible because the column was dropped.

**This is the most impactful gap for practical diff quality.**

---

### Proposed Improvement: Iterative Refinement

Instead of the current one-pass column→row pipeline:

```
align_columns (using headers)
    ↓
align_rows (using column-aligned signatures)
    ↓ DONE
```

Use alternating optimization:

```
C₀ = align_columns(headers)
R₀ = align_rows(C₀, row_signatures)
loop {
    C_{i+1} = align_columns(R_i, column_signatures)  // re-align using row-matched data
    R_{i+1} = align_rows(C_{i+1}, row_signatures)    // re-align using updated columns
    if C_{i+1} == C_i && R_{i+1} == R_i { break }     // converged
}
```

**Intuition:** After the first row alignment, we know which rows correspond. We can use this correspondence to build better column signatures (comparing cells across matched rows, not just headers). This may detect column renames or reorderings that header-only LCS missed.

**Convergence:** Each iteration can only improve or maintain the alignment quality (monotone non-increasing cost). Since there are finitely many possible alignments, the algorithm converges in finite steps. Whether convergence is fast (poly iterations) is an open question.

**This approach is unpublished in the table/spreadsheet diff literature to our knowledge.**

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
