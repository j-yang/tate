# tate: Algorithm Design Document

## Overview

tate is a pure structured diff, patch, and merge algebra. It rests on one ontological commitment: **every structure is a tree**, and a tree is a *section* of a `location → value` sheaf (the flat map from each addressable location to the value living there). Diff, 3-way merge, and a lossless patch algebra are defined **once**, on that single object.

tate deliberately contains no format parsing and no sequence/grid alignment. Turning bytes into a tree — and, for un-keyed data, computing the alignment (LCS, coordinate descent) that gives it stable identities so it *can* be a tree — is a separate concern that lives in the **mumford** crate (see `mumford/ALGORITHMS.md`). The split is the whole point: seq / grid / table are not parallel diff problems here, they are trees once keyed, and the unification is at the level of the object (`Section`) and the laws, not a pile of per-shape algorithms.

This document describes tate's algebra — the object (`section`), structural tree diff and the single 3-way merge (`tree`), and the lossless patch algebra (`patch`).

> **History.** Earlier versions of tate also carried the alignment engines (`lines`, `grid`, `inline`, `unified`) and three parallel 3-way merges (line-level, grid, tree). Two findings collapsed this: (1) once grids/sequences are keyed into trees, `grid_merge` and the line merge are reproduced exactly by `tree_merge` — proven by `mumford/tests/grid_stable_key_probe.rs` — so they were removed in 0.4; (2) the alignment engines are format/keying concerns, not tree algebra, so they moved to mumford. What remains here is only the tree algebra.

---

## Module 1: `tree` — Structural Tree Diff

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


---

## Module 2: `section` + `patch` — The Object and Its Algebra

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


---

## Summary of Gaps by Priority

| Gap | Area | Impact | Priority |
|-----|------|--------|----------|
| No move detection (same identity, new parent) | tree | A move is reported as delete+add | Low — rare in the primary use case (CDISC define.xml) |
| Positional matching when sibling keys collide | tree / patch | Keyless siblings sharing a kind land at one location | Handled upstream by the keying adapters (mumford) |

Alignment-layer gaps — the table-edit-distance formalization, coordinate-descent local optima, column reordering, and inline pairing — now live with those algorithms in `mumford/ALGORITHMS.md`.
