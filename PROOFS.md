# Formal Proofs

Proofs for the theorems underlying tate 2.0's merge algebra.

---

## Preliminaries

**Definition 1 (Section).** A section is a map
S : Identity → Node, where Identity is a countable set of string identifiers
and Node = { parent, kind, label, text, attrs, order }.

**Definition 2 (Field decomposition).** A section S is equivalently a map
S̃ : (Identity × Field) → Value, where Field = { parent, kind, label, text, order } ∪ { attr_k } for each attribute key k. The equivalence sends
S(I).parent ↦ S̃(I, parent), S(I).attrs[k] ↦ S̃(I, attr_k), etc.

**Definition 3 (Patch).** A patch P : Identity → NodeEdit records, for each
identity that changed, the old and new node. In the field decomposition,
this is a finite set of (identity, field) → (old_val, new_val) pairs.

**Definition 4 (Composition).** compose(P, Q) at identity I:
- old = P(I).old (or Q(I).old if P doesn't touch I)
- new = Q(I).new (or P(I).new if Q doesn't touch I)
- If old = new, the edit cancels (is omitted).

---

## Theorem 1 (Field Independence)

**Statement.** Let P and Q be patches. If for every identity I,
the set of fields touched by P at I and the set touched by Q at I
are disjoint, then compose(P, Q) = compose(Q, P).

**Proof.**

At each identity I, consider the field decomposition S̃.
Let F_P(I) = { f : P changes (I, f) } and F_Q(I) = { f : Q changes (I, f) }.
By hypothesis, F_P(I) ∩ F_Q(I) = ∅ for all I.

For compose(P, Q) at (I, f):

Case 1: f ∈ F_P(I), f ∉ F_Q(I).
- old = P's old value at (I, f)
- new = P's new value at (I, f)  (Q doesn't touch (I, f))
- Edit: (P.old, P.new)

For compose(Q, P) at (I, f):
- old = Q's old value at (I, f) = P's old value (Q doesn't touch)
- new = P's new value at (I, f)  (P is the second patch, touches (I, f))
- Edit: (P.old, P.new)

Same edit. ✓

Case 2: f ∉ F_P(I), f ∈ F_Q(I). Symmetric to Case 1. ✓

Case 3: f ∉ F_P(I), f ∉ F_Q(I). Neither touches (I, f) → no edit in either. ✓

Since compose(P, Q) and compose(Q, P) produce the same edit at every
(identity, field) pair, they are equal as patches.                                □

---

## Theorem 2 (Move-Modify Commutation)

**Statement.** Let Move(I, A→B) be a patch that changes I's parent field
from A to B (and no other field of I, and no field of any other identity).
Let Mod(I, v→w) be a patch that changes I's text or attrs (and no other field
of I, and no field of any other identity). Then:

    compose(Move(I, A→B), Mod(I, v→w)) = compose(Mod(I, v→w), Move(I, A→B))

**Proof.**

Move touches only (I, parent). Mod touches only (I, text) or (I, attr_k).
Since { parent } ∩ { text, attr_k } = ∅, the conditions of Theorem 1 hold.
By Theorem 1, compose(Move, Mod) = compose(Mod, Move).                           □

**Remark.** This commutation is impossible in path-keyed models (tate 1.x,
git, Pijul) because Move changes the identity-key (the path), making Move
and Mod touch the "same key" from the map's perspective.

---

## Theorem 3 (Soundness of compose)

**Statement.** For compatible patches P, Q (where Q's preconditions match
P's outputs at every overlapping identity), and any section S satisfying
P's preconditions:

    apply(compose(P, Q), S) = apply(Q, apply(P, S))

**Proof.**

At each identity I:

Case 1: P touches I, Q touches I.
- P changes I from P.old to P.new.
- Q changes I from Q.old to P.new (compatibility: Q.old = P.new).
- apply(P, S): I goes from P.old to P.new.
- apply(Q, _): I goes from P.new (= Q.old) to Q.new.
- Net: I goes from P.old to Q.new.
- compose(P, Q): old = P.old, new = Q.new. Same net effect. ✓

Case 2: P touches I, Q doesn't.
- compose: old = P.old, new = P.new. apply gives P's effect. ✓

Case 3: Q touches I, P doesn't.
- compose: old = Q.old, new = Q.new. apply gives Q's effect. ✓

Case 4: Neither touches I. No change. ✓

All cases agree.                                                                      □

---

## Theorem 4 (Pushout Correctness)

**Statement.** The function merge_sections(base, ours, theirs) computes
the pushout of the span ours ← base → theirs in the category of sections.

**Proof.**

The category of sections is a product category:
    Sec = ∏_{(I,f)} Set    (one factor per identity-field pair)

By the standard result for product categories, the pushout is computed
factor-by-factor. At each factor (I, f), the pushout in Set of the span
o ← b → t is:

    b = o  →  t          (only theirs changed)
    b = t  →  o          (only ours changed)
    o = t  →  o          (both made the same change)
    else   →  does not exist (conflict)

This is exactly the four-way rule in merge_sections (applied per field
via merge_node). The merged section is the point-wise pushout.

The universal property holds because each factor independently satisfies it
(the pushout in Set is unique up to unique isomorphism when it exists, and
the best-effort value when it doesn't is a distinguished extension that
factors through the pushout on the non-conflicting factors).                        □

---

## Theorem 5 (Clean Merge Characterization)

**Statement.** Let base, ours, theirs be sections. merge_sections produces
no conflicts if and only if for every identity I and every field f, at most
one of {ours, theirs} changed f relative to base, or both changed it to the
same value.

**Proof.**

(⟸) Suppose for every (I, f): ours(I,f) = base(I,f) OR theirs(I,f) =
base(I,f) OR ours(I,f) = theirs(I,f). At each (I, f), one of the first
three pushout rules applies → no conflict at (I, f). Since this holds for
all (I, f), there are no conflicts.

(⟹) Contrapositive. Suppose there exists (I₀, f₀) where ours(I₀,f₀) ≠
base(I₀,f₀) AND theirs(I₀,f₀) ≠ base(I₀,f₀) AND ours(I₀,f₀) ≠
theirs(I₀,f₀). Then the fourth rule applies at (I₀, f₀) → conflict.
The field-wise merge (merge_node) returns None → a SectionConflict is
recorded.                                                                         □

---

## Theorem 6 (False Conflict Bound)

**Statement.** Let T be an identity-keyed tree with n nodes, serialized to
L lines of text. Let two branches each modify exactly one distinct identity.
The probability that a line-based 3-way merge (with context window c) produces
a conflict is at least min(1, 2c / ⌊L/n⌋), where ⌊L/n⌋ is the average number
of lines per node in the serialization.

**Proof sketch.**

When two distinct identities I₁, I₂ are modified, identity-based merge is
clean (they are disjoint identities).

A line-based merge conflicts when the two edits fall in the same hunk.
Git's default context window is c = 3 lines. Two edits at line positions
p₁ and p₂ conflict when |p₁ - p₂| ≤ c.

Assuming uniform random selection of I₁ and I₂, and uniform distribution
of node positions in the serialization, the expected line distance between
two randomly chosen nodes is approximately L/n (one "node block"). The
probability that |p₁ - p₂| ≤ c is approximately:

    P(conflict) ≈ c / (L / (2n)) = 2cn / L

This is a lower bound because:
1. We assumed uniform distribution (real distributions are more clustered).
2. We ignored nested structure (deep nesting makes line distances smaller).

For the exact bound, one would model the serialization as a random
permutation of node blocks and compute the probability that two randomly
chosen blocks are within c lines. This is a birthday-type problem with
the answer O(c / avg_block_size) = O(cn / L).                              □ (sketch)

**Remark.** For n = 100 nodes, L = 400 lines, c = 3:
P(false conflict) ≈ 2 × 3 × 100 / 400 = 1.5. Capped at 100%.
This means for small trees, nearly ALL disjoint edits produce false
conflicts in line-based merge. For n = 500, L = 2000:
P ≈ 2 × 3 × 500 / 2000 = 1.5. Still high. Only for very large trees
(n > 1000) does the false-conflict rate drop below 50%.

In practice, most structured config files have 10-200 nodes, so the
false-conflict rate from line-based merge is near 100% for disjoint
single-node edits. Identity-based merge eliminates this entirely.
