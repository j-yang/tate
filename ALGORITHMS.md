# tate: Algorithm Design Document

## Overview

tate is a self-contained structured diff, patch, and merge library. It rests on one ontological commitment: **every structure is a tree**, and a tree is a *section* of a `location → value` sheaf (the flat map from each addressable location to the value living there). Diff, patch, and 3-way merge are defined **once**, on that object; sequence and grid data reach the same machinery by being *keyed into trees* — the alignment that LCS and coordinate descent compute becomes stable identities.

Concretely the crate has two layers:

- **The object and its algebra.** `section` defines the canonical `Section` (location → value). `tree` diffs and 3-way-merges it; `patch` is the lossless `diff / apply / invert / compose` algebra whose laws are proptest-verified. There is a **single merge** — `tree::tree_merge`.
- **Keying adapters** — the algorithms that turn un-keyed data into a stably-keyed tree, because for keyless data "figure out which row/line corresponds across versions" *is* the hard part and can only be answered by comparing versions. `lines` (patience + LCS + Hirschberg) keys sequences; `grid` (coordinate descent) keys 2D tables. These are irreducible algorithms, not parallel diff engines: their output feeds the one tree algebra. `inline` adds word-level highlighting; `lcs` is a private bounded-LCS primitive.

This document describes each algorithm, the single-merge design decision, current limitations, and theoretical gaps.

> **History.** Earlier versions carried three parallel 3-way merges (line-level `merge`, `grid_merge`, `tree_merge`). Once grids/sequences are keyed into trees, those first two are redundant — proven empirically by `tests/grid_stable_key_probe.rs`, which shows "grid → alignment-keyed Section → tree_merge → grid" reproduces the old `grid_merge` exactly (disjoint edits, same-cell conflicts, and row-insert shifts). tate 0.4 removed them. See *Module 6* below.

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

### 3-way merge: `tree_merge` — the single merge

`tree_merge(base, ours, theirs)` is tate's **only** 3-way merge. It is the sheaf *gluing* of two sections that both restrict to `base` on the untouched locus:

```
diff_o = tree_diff(base, ours)      # what ours changed
diff_t = tree_diff(base, theirs)    # what theirs changed
merged = ours, then apply theirs' changes where they do not collide with ours
conflicts = the collisions (gluing obstructions)
```

Merge is a **total function**: it always returns a tree carrying a best-effort value (favouring `ours`), and a non-empty conflict list flags that the value was forced. Four obstruction classes are recorded as `TreeConflict` (`ConflictKind`):

| Kind | Condition |
|------|-----------|
| `Attr` | both sides set the same node's same attribute to different values |
| `Text` | both sides set the same node's text to different values |
| `AddAdd` | both sides add a node at the same location with differing content |
| `ModifyDelete` | one side modifies a node the other side removes |

**Text is a first-class merge dimension.** A node's scalar payload (JSON value, XML text, a grid cell) lives in `text`, not in attributes. `tree_diff` records it as `changed_text`, and merge applies/conflicts on it exactly as it does for attributes — omitting this silently drops cell/scalar edits, which was a real bug fixed in 0.4.

**Theoretical properties.**
- *Totality:* always returns a result; conflicts are data, not failure.
- *Symmetry (pushout):* the conflict set is symmetric under swapping `ours`/`theirs` (proptest-verified).
- *Cleanliness:* disjoint changes glue with zero conflicts; identical changes on both sides are no-ops.

---

## Module 5: `lcs` (private) — Lightweight LCS for Word-Level Diff

A simple O(n·m) LCS DP with full matrix, used internally by `inline_segments` for word-level token alignment. Input is bounded (tokens of a single line, typically tens to hundreds), so the full matrix is appropriate — no patience anchoring or Hirschberg needed.

No theoretical gaps. This is the textbook LCS algorithm applied to a bounded input.

---

## Module 6: `section` + `patch` — The Object and Its Algebra

### The object: `Section`

A `TreeNode` is the nested, human-facing view. A `Section` is its flat, canonical form: a `BTreeMap<Location, Value>` where

- **`Location`** = the path of sibling *keys* from the root to a node. A key is the node's `identity` if set, else its `kind`.
- **`Value`** = everything intrinsic to a node *except* which children it has: kind, label, text, attributes, and `order` (its index among siblings).

Which children a node has is encoded structurally — by which *other* locations exist in the map — so it is not stored in the value. Two design consequences follow directly from the sheaf split:

- **Identity is the location; structural position is a value.** Renaming a node's kind, or moving it among siblings (its `order`), is a *value change at a stable location*, not a delete+add. This is what lets moves and renames merge cleanly.
- **`⊥` (absent) is "the location is not in the map."** A patch names the absent state explicitly with `None`.

`TreeNode::to_section` flattens; `Section::to_tree` rebuilds (children ordered by stored `order`). Round-tripping is the identity on trees whose siblings have distinct keys — the canonical case for identity-keyed data. Keyless siblings sharing a kind collide at one location; disambiguating them is the keying adapters' job.

### The algebra: `patch`

`patch` is the lossless `diff / apply / invert / compose` over sections — the morphisms of the versioned-structure category. Unlike `tree_diff` (a *display* diff that bubbles keyless changes up and drops add/remove payloads, so it cannot round-trip), `patch::diff` records exactly enough to reconstruct the target.

- `diff_sections(a, b)` — the unique patch `a → b`: for every location differing between the two sections, a `PointEdit { old, new }` (`None` = ⊥).
- `apply(p, a)` — transport; errors if `p`'s expected `old` does not match `a` (a patch applies only to the section it came from).
- `invert(p)` — swap `old`/`new` at every location.
- `compose(p, q)` — sequential `p` then `q`; edits that cancel drop out.

**Laws (proptest-verified, `tests/patch_laws.rs`, 2000 cases each):**

```
apply(diff(a, b), a) == b                          # diff/apply inverse
diff(a, a) is empty                                # identity
apply(invert(p), apply(p, a)) == a                 # invert undoes apply
apply(compose(p, q), a) == apply(q, apply(p, a))   # composition
(p ∘ q) ∘ r == p ∘ (q ∘ r)                         # associativity
compose(p, invert(p)) is empty                     # inverses cancel
tree_merge conflict set symmetric under swap       # pushout symmetry
```

Together with the identity patch and inverses, `compose`'s associativity makes patches a groupoid.

### Design decision: one merge, many keying adapters

The crate deliberately has **one** 3-way merge (`tree_merge`) rather than one per shape. The justification is empirical, not aesthetic:

- **What could be removed was removed.** `grid_merge` and the line-level `merge` duplicated 3-way logic. `tests/grid_stable_key_probe.rs` demonstrates that keying a grid by the alignment `grid_diff` already computes (base-anchored row/column identities) and running `tree_merge` reproduces `grid_merge` exactly — including cell-level conflicts and row-insert shifts that positional keying gets wrong. So they were redundant, and had zero product callers. Removed in 0.4.
- **What is irreducible stays.** LCS (`lines`) and coordinate-descent grid alignment (`grid`) are not "other diffs" — they are the translators that give un-keyed data stable identities so it *can* be a tree. Deleting them would make sequences and grids unable to become correct trees. They survive as keying adapters and produce their own display results (`GridDiff`).

The unification is at the level of the **object** (`Section`) and the **laws**, not at the level of the algorithms. Each surviving algorithm does a job no other does.

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
