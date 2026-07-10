# Mathematical Foundation

tate 2.0 models structured data as **identity-keyed sections**. This document
makes the design precise and connects it to the code.

---

## 1. The identity-section model

### 1.1 Identity ≠ Location

In tate 2.0, a **section** is a map:

```
Section = Identity → Node
```

where `Identity` is a string (the node's stable identifier) and `Node` stores
both position and content:

```
Node = {
    parent: Option<Identity>,   // structural position
    kind: String,               // element type
    label: String,              // human-readable name
    text: String,               // text content
    attrs: Vec<(String, String)>, // key-value attributes
    order: usize,               // index among siblings
}
```

The **key design decision**: identity is the map key, and position (`parent`,
`order`) is a *value* stored inside the node. This means:

- A **Move** changes `parent` at a stable identity key — not a delete+insert
  at two different path keys.
- A **Modify** changes `text`/`attrs` at a stable identity key.
- **Move and Modify touch different fields of the same node** → they are
  independent operations → they commute.

### 1.2 Why this matters

In tate 1.x (and in all path-keyed diff systems, including git), a node's key
is its path from the root. Moving a node changes its path → the key changes →
the diff sees delete+insert. This makes Move + Modify impossible to merge
cleanly: the modify targets a node that was "deleted."

In tate 2.0, the key is stable (identity). Move changes a field (`parent`),
not the key. Two branches that touch different fields of the same node
merge cleanly via the field-wise pushout.

### 1.3 Equivalence: (Identity, Field) pairs

A section is equivalently a flat map:

```
Section ≅ BTreeMap<(Identity, Field), Value>
```

where `Field ∈ {parent, kind, label, text, order, attr₁, attr₂, …}`.

This is the **sheaf on a discrete space** — the space of (identity, field)
pairs. Each pair maps to a scalar value.

---

## 2. Diff as section difference

Given two sections `S₁` and `S₂`, the diff is the set of (identity, field)
pairs where values differ. At the node level, this means: for each identity,
compare every field. If any field differs, emit a `NodeEdit` recording the
old and new node.

This is complete: every change is captured, no heuristic is involved.

---

## 3. Merge as field-wise pushout

### 3.1 The categorical setup

The category of sections `Sec` is a **product category**:

```
Sec = ∏_{(I, f)} Set    (one factor per identity-field pair)
```

Pushouts in a product category are computed **point-wise** (one factor at a
time). This is the standard result from category theory.

### 3.2 The point-wise rule

At each (identity, field) pair, with base `b`, ours `o`, theirs `t`:

```
o = b  →  take t          (only theirs moved this field)
t = b  →  take o          (only ours moved this field)
o = t  →  take it         (both made the same change — glues)
o ≠ t, both ≠ b  →  conflict (the pushout does not exist at this pair)
```

### 3.3 Clean merge

When all three sides exist at an identity but differ as whole nodes, the
field-wise merge tries **each field independently**:

- `parent`: if only one side changed it → take that side's parent
- `kind`: same logic
- `text`: same logic
- `attrs`: each attribute key merged independently
- `order`: same logic

If all fields merge cleanly → the node merges cleanly, even though the whole
nodes differed. This is what enables **Move + Modify clean merge**: Move
changes `parent`, Modify changes `text`/`attrs` — different fields, same node.

### 3.4 Conflict = field-level obstruction

A conflict occurs at an (identity, field) pair when both sides changed that
specific field to incompatible values. The conflict set is the set of such
pairs. On a discrete (identity, field) space, this is the first Čech
cohomology H¹ of the cover {U_ours, U_theirs}.

---

## 4. The patch groupoid

### 4.1 Patches as morphisms

A patch is an identity-keyed map of node edits:

```
Patch = BTreeMap<Identity, NodeEdit>
NodeEdit = { old: Option<Node>, new: Option<Node> }
```

### 4.2 Groupoid structure

Patches form a groupoid (verified by proptest, 2000 random cases each):

| Law | Statement |
|---|---|
| Diff/apply inverse | `apply(diff(a, b), a) == b` |
| Identity | `diff(a, a)` is empty; `apply(empty, a) == a` |
| Inverse | `apply(invert(p), apply(p, a)) == a` |
| Composition | `apply(compose(p, q), a) == apply(q, apply(p, a))` |
| Associativity | `compose(compose(p, q), r) == compose(p, compose(q, r))` |
| Cancellation | `compose(p, invert(p))` is empty |
| Left/right identity | `compose(p, empty) == p == compose(empty, p)` |

### 4.3 Commutation via field independence

**Theorem (Field Independence):** Patches that touch different fields of the
same identity commute.

*Proof:* Each field edit is an independent entry in the equivalent
`BTreeMap<(Identity, Field), Value>`. Edits at different keys are
independent → commute. QED.

**Corollary (Move-Modify Commutation):**
`Move(I, A→B) ∘ Modify(I, v→w) = Modify(I, v→w) ∘ Move(I, A→B)`

*Proof:* Move touches `(I, parent)`. Modify touches `(I, text)` or
`(I, attrs)`. Different fields → by Field Independence, they commute. QED.

This commutation is **impossible in path-keyed models** (including tate 1.x
and Pijul's line-based theory), because Move changes the key, making the two
operations interact.

---

## 5. Comparison with Pijul

| | Pijul | tate 2.0 |
|---|---|---|
| Data model | Text lines (position-based) | Tree nodes (identity-based) |
| Patch commutation | Line-level (adjacent lines) | Field-level (different fields of same node) |
| Move handling | Position shift | `parent` field change |
| Move + Modify | May conflict (position changes) | Always commutes (different fields) |
| Merge algorithm | Commutation + composition | Field-wise pushout |
| Merge base | Patch dependency graph | Commit DAG (LCA) |
| Conflict representation | Text markers / patch sets | (Identity, field) pairs |

tate's merge is **snapshot-based** (pushout on sections), not
**patch-based** (commutation on operations). This means:
- Merge does not require commutation — it works on any three sections.
- Cherry-pick and rebase are `apply`-based (simpler, less flexible than
  Pijul's commutation-based versions).
- For structured data (where changes are usually disjoint by identity),
  this is sufficient.

---

## 6. Design consequences

1. **Identity as key.** A node's identity is its map key; position is a value.
   Move is a parent-field change, not a delete+insert.

2. **Field-wise merge.** Each field is merged independently. This resolves
   Move + Modify, reorder + modify, and different-attribute changes
   automatically — all are "different fields, same node."

3. **Field-wise pushout.** The merge is the categorical pushout in the
   product category ∏(identity, field) Set. Proptest checks the pushout
   property at every (identity, field) pair.

4. **Patch groupoid with field-level commutation.** The groupoid laws are
   verified. The Field Independence theorem gives a sound basis for
   commutation — patches on different fields always commute.
