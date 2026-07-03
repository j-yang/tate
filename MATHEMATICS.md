# Mathematical Foundation

tate models structured data as **sections of a location→value sheaf**. This
document makes that statement precise and connects it to the code.

---

## 1. The sheaf model

### 1.1 Trees as sheaves

A tree is a sheaf on the **Alexandrov topology** of its prefix poset: the set
of locations (paths from the root), ordered by the prefix relation `≤` (a
location is ≤ its extensions). The open sets are downward-closed sets of
locations (subtrees rooted at some node).

A **section** of this sheaf assigns to each location a **value**:

```
Section = Location → Value
```

where:

- **Location** = the sequence of sibling keys from the root to the node.
  A key is the node's **identity** (if it has one) or its **kind** (positional).
  Identity-as-key is the load-bearing choice: a node keeps its location when
  its content changes, so a moved or renamed node is a *value change* at a
  stable location, not a delete+add.

- **Value** = everything intrinsic to a node except which children it has:
  `kind`, `label`, `text`, `attributes`, and `order` (index among siblings).
  Structural position (`order`) is part of the value, not the location — this
  is the sheaf split that makes moves detectable as value-level edits.

- **⊥** (the absent value) = a location not present in the map. Patches use
  `Option<Value>` to talk about absence explicitly (`None` = ⊥).

**The gluing axiom holds automatically.** Trees are contractible in the
Alexandrov topology (they have a unique root = initial element), so any
locally consistent assignment of values to locations extends uniquely to a
global section. This means the sheaf is well-defined and the algebra on it
is exact.

### 1.2 The two views

tate provides two interconvertible views of the same data:

- **`TreeNode`** — the nested view. Format parsers produce it, UIs consume
  it, humans read it.
- **`Section`** — the flat view: `BTreeMap<Location, Value>`. This is the
  object the algebra runs on, because on the flat view an edit is a point
  change and the laws are clean.

Round-tripping (`to_section` / `to_tree`) is the identity on trees whose
siblings have distinct keys.

---

## 2. Diff as section difference

Given two sections `S₁` and `S₂`, the **diff** is the set of locations where
values differ:

```
diff(S₁, S₂) = { (loc, S₁(loc), S₂(loc)) | S₁(loc) ≠ S₂(loc) }
```

This is a point-wise comparison on a discrete base space. It is complete:
every difference is captured, no heuristic is involved.

`tate::tree::tree_diff` is the display-layer version (it summarises changes
for humans, bubbling keyless descendants to their nearest identity-bearing
ancestor). `tate::patch::diff` is the lossless version (it records exactly
enough to reconstruct the target from the source).

---

## 3. Merge as pushout

### 3.1 The categorical setup

Define the **category of sections** `Sec`:

- **Objects**: sections (`BTreeMap<Location, Value>`)
- **Morphisms**: patches (location-keyed point edits, see §4)
- **Composition**: patch composition
- **Identities**: empty patches

Given a base section `B` and two branches `O` (ours) and `T` (theirs), each
related to `B` by a patch:

```
    O
   ↑
   | f = patch(B → O)
   |
   B
   |
   | g = patch(B → T)
   ↓
   T
```

### 3.2 Clean merge = pushout

When `O` and `T` change **disjoint** sets of locations, the **pushout** of
`f` and `g` exists. It is the section `M` obtained by applying both change
sets to `B`:

```
M = B with O's changes applied and T's changes applied
```

The universal property: any other section `M'` compatible with both `O` and
`T` factors uniquely through `M`. This is computed **element-wise** — the
correct way to compute a pushout in a product category, one factor per
location. Concretely, at each location `ℓ` with `b = B(ℓ)`, `o = O(ℓ)`,
`t = T(ℓ)`:

```
o = b            → take t   (only theirs moved)
t = b            → take o   (only ours moved, or neither)
o = t            → take it  (both made the same move — glues)
o ≠ t, both ≠ b  → conflict (see §3.3)
```

**This is exactly `tate::patch::merge_sections`.** It is not a metaphor laid
over a heuristic: the function runs the four-way point-wise rule above on the
lossless `Section`, and a proptest law (`law_merge_is_pointwise_pushout`,
2000 cases) checks the merged section against this pushout definition at
*every* location, and checks that the conflict set is *precisely* the set of
divergent locations — no more, no less.

### 3.3 Obstruction = conflict

When `O` and `T` change the **same** location to **different** values, no
section extends both. The pushout does not exist at `ℓ`.

`merge_sections` is a **total function**: it always returns a best-effort
section (carrying `ours`'s value at conflicting locations) and records every
obstruction as a `SectionConflict { location, base, ours, theirs }`. The
conflict set is the H¹ of §5.

### 3.4 Two merges: the algebra and the display

`merge_sections` (above) is the exact pushout on `Section`. The tree-facing
`tree::tree_merge` is a **display-oriented** wrapper: it drives off the lossy,
human-readable `tree_diff` so its conflicts carry UI-level detail — *which*
attribute clashed, old/ours/theirs text, and the classification below. The two
agree on **where** conflicts occur; they differ only in how much detail each
attaches. Use `merge_sections` for the algebra, `tree_merge` to show a human
what clashed.

`tree_merge`'s four display obstruction classes:

| Conflict kind | Sheaf interpretation |
|---|---|
| `Attr` | Both sides set the same attribute to different values |
| `Text` | Both sides changed the same node's text content differently |
| `AddAdd` | Both sides added a node at the same path with differing content |
| `ModifyDelete` | One side modified a node the other removed |

---

## 4. The patch groupoid

### 4.1 Patches as morphisms

A **patch** is a location-keyed map of point edits:

```
Patch = BTreeMap<Location, PointEdit>
PointEdit = { old: Option<Value>, new: Option<Value> }
```

`None` represents ⊥ (absent). The invariant `old ≠ new` holds for every
edit.

### 4.2 Groupoid structure

Patches form a **groupoid** (a category in which every morphism is
invertible):

- **Objects**: sections
- **Morphisms**: patches
- **Identity**: `Patch::empty()` (no edits)
- **Composition**: `compose(p, q)` — element-wise, with cancellation
- **Inverse**: `invert(p)` — swap `old` and `new` in every edit

### 4.3 Verified laws

The groupoid axioms are verified by proptest (2000 random cases each):

| Law | Statement |
|---|---|
| Diff/apply inverse | `apply(diff(a, b), a) == b` |
| Identity | `diff(a, a)` is empty; `apply(empty, a) == a` |
| Inverse | `apply(invert(p), apply(p, a)) == a` |
| Composition | `apply(compose(p, q), a) == apply(q, apply(p, a))` |
| Associativity | `compose(compose(p, q), r) == compose(p, compose(q, r))` |
| Cancellation | `compose(p, invert(p))` is empty |
| Merge symmetry | Swapping ours/theirs yields the same conflict set |
| Pushout construction | `merge_sections` equals the point-wise pushout at every location, and its conflict set is exactly the divergent locations |
| Identical branches | `merge_sections(b, x, x)` is conflict-free and returns `x` |

### 4.4 Commutativity

On a discrete location space, patches at different locations **trivially
commute**: `compose(p, q) == compose(q, p)` when `p` and `q` touch disjoint
locations. This makes the groupoid **abelian** on non-overlapping patches.

Non-trivial commutativity (patches that interact at the same location but
in compatible ways) is the subject of future structural-operation work.

---

## 5. Cohomological interpretation of conflicts

### 5.1 Čech complex for merge

Given base `B` and branches `O`, `T`, define the cover:

- `U_O` = locations changed by `O` (relative to `B`)
- `U_T` = locations changed by `T`

The **Čech complex** for this cover with values in the section sheaf:

- `C⁰` = assignments to each open set
- `C¹` = assignments to the intersection `U_O ∩ U_T`
- `δ⁰: C⁰ → C¹` = restriction to the intersection (check agreement)

### 5.2 Cohomology groups

- **H⁰ = ker(δ⁰)** = assignments that agree on the intersection = **clean
  merges** (no conflicts).

- **H¹ = coker(δ⁰)** = disagreements on the intersection = **the conflict
  set**. Each conflicting location contributes one generator to H¹.

- **H^k = 0 for k ≥ 2**: with a two-set cover, the Čech complex has no
  higher simplices.

### 5.3 Multi-way merge

For `n` branches, the cover has `n` open sets. The Čech complex has
non-trivial higher structure:

- **H¹** captures pairwise conflicts (two branches disagree at a location).
- **H²** captures triple inconsistencies: three branches where every pair
  agrees, but the triple is inconsistent. These are invisible to sequential
  pairwise merging.

On a discrete location space, H^k = 0 for k ≥ 2 regardless of cover size
(local sections always glue point-wise). The interesting cohomology arises
from the **cover topology** (how branches overlap), not the data topology.

---

## 6. Design consequences

The sheaf perspective is not merely descriptive — it drove several
load-bearing design decisions:

1. **Identity as location.** A node's identity is its address; structural
   position is a value. This makes moves and renames value-level edits,
   not delete+add pairs. (§1.1)

2. **Explicit absence (⊥).** Absent locations are first-class (`None` in
   `PointEdit`), not hidden inside `Option<Value>` ad-hoc. This gives
   additions and deletions the same algebraic status as modifications. (§1.1)

3. **Total merge as an honest pushout.** `merge_sections` *is* the point-wise
   pushout on `Section` — proptest-checked against the pushout definition, not
   a heuristic wearing categorical language. It always returns a section +
   obstruction list, so downstream code never panics on conflicts, and the
   conflict set *is* the cohomological obstruction: exactly where the pushout
   fails. The tree-facing `tree_merge` is a display wrapper over the same
   obstruction locus. (§3.2–§3.4, §5.2)

4. **Patch groupoid, not monoid.** Every patch has an inverse, verified by
   proptest. This enables bidirectional patch pipelines (undo, rollback,
   replay). (§4)

5. **Section as the canonical object.** diff, patch, *and* merge operate on
   `Section`, not `TreeNode`. The flat view makes the laws clean; the nested
   view is for I/O and display only. (§1.2)
