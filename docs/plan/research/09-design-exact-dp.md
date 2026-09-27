# Designer "exact": exact and near-exact dynamic programming for the Card Decoder solver

## 0. Thesis
- **What is exact.** The event is a finite belief-MDP. Within a puzzle, the belief is exactly the state (C, K, M), with posterior w restricted to C. Across puzzles, the event splits exactly into a table F_r(h,c) when assumptions A1–A6 hold (§1.3).
- **Final puzzle.** Exact solving is now cheap up to n≈16 at every resource level I tested. I measured two new admissible bounds (Lemma A and P̄₁, §2.3): with them, B&B keeps values identical in 260/260 cases, and the worst time for n≤16 drops from 7.4 s to 64 ms native.
- **Solver.** The core is a two-sided bounded expectimax. It always returns a proven interval [L,U] on the value, is anytime, and never throws work away. The UI shows the numeric gap U−L instead of a label.
- **Continuation.** A lower table F^lo is computed **exactly, with no sampling**, as the value of a concrete menu of per-puzzle policies. It is built from preset-independent per-policy distributions, so it can be recomputed in the browser in under 1 ms for any preset. For r≥2, "optimal" always means optimal relative to the continuation table in use.
- **Honest limit.** Proofs of true optimality early in the event are out of reach, because the upper table F^hi is nearly trivial when c is large. The second and third objective components (matches, −actions) mean something only where the first (expected solved) is provably tied.

## 1. Formal model
### 1.1 Puzzle layer (POMDP reduced to a belief-MDP)
- **State.** x = (C, K, M, h, c):
  - C ⊆ U is the candidate set (8023 groups);
  - K is the mask of known fields;
  - M is the mask of fields matched by challenges.
- **Belief.** b(t) = w_t·[t∈C]/w(C). This is exact because every observation is a deterministic function of the target plus randomness that does not depend on the target (the hint's field draw), so Bayes' rule reduces to restricting the set.
- **HINT** (h>0, K≠63). The field f is drawn uniformly from the unknown fields; the game shows its printed value v (level shows printed n). Next state: (C∩{v_f=v}, K|f, M, h−1, c). Reward (0, 0, −1).
- **CHALLENGE g** (c>0). Observation o = (m, frame id if m&1, printed n if m&8). Next state: (C∩{fb(g,·)=o}, K|m, M|m, h, c−1). Reward (1[m=63], |M|m|−|M|, −1). Full-match sets FM(g) have at most 3 members. FM is symmetric, so FM⁻¹ = FM.
- **Removed without loss:** the guessed set (a re-guess is a strictly dominated no-op), no-op guesses, and a "stop" action. Stop is never lexicographically optimal while c>0 and C≠∅, because challenging a candidate has P(solve)>0.

### 1.2 Event layer
- F_0 ≡ 0.
- F_r(h,c) = E_{t∼w, f₀∼U(6)}[ V_r(x₀(t,f₀); h, c) ].
- V_r is the in-puzzle optimum with terminal utility at a solve after using (H,J) resources: R_in + F_{r−1}(h−H, c−J). When c=0 without a solve, the value is 0. The event ending on failure is not an extra assumption: no solve is possible without challenges.
- There are 218 distinct root beliefs x₀ (13+7+25+13+83+77).
- **Lexicographic Bellman.** Lexicographic order on ℝ³ is a total order preserved by addition and positive scaling, so backward induction is exact in exact arithmetic (§2.7 covers floating point).
- **Objective structure.** Every solved puzzle contributes exactly 6 first-time matches, so E[M] = 6·E[S] + E[matches in the failed puzzle]. The third component (−E[actions]) decides only in subtrees where a solve is guaranteed.
- **Points variant.** Premium milestones are additive per puzzle and depend on (puzzle index k, |M|). Index F by k instead of r; nothing else changes.

### 1.3 When the decomposition is exact
It is exact if and only if the next puzzle's (target, reveal, hint draws), given the whole history, depends only on (h,c):
- **A1.** Targets are i.i.d. ∝ w, drawn with replacement, independent of play.
- **A2.** One initial reveal: a field uniform over the 6 (app.js `seedTestInitialReveal`; the dead `initialValue` code uses the same law).
- **A3.** A hint's field is uniform over the unknown fields, where known includes fields matched by challenges.
- **A4.** One pooled inventory. Daily grants that accumulate and can be saved for later are equivalent. Use-it-or-lose-it grants add a dimension F_r(h,c,day); the recursion is still exact, just larger.
- **A5.** Rewards are additive per puzzle.
- **A6.** The action set depends only on information within the puzzle (an R15 filter is fine).

Violations and their cost:
- **Without replacement at card level.** For puzzle k, the prior's total variation distance from w is at most (k−1)/(9059−k+1), so the sum over 9 puzzles is at most 36/9051 = 0.004. Coupling gives |ΔF| ≤ 0.036 as a crude worst case. Online, the current puzzle can use the exact posterior by subtracting already-drawn cards from w at no cost; only F̂ carries this error.
- **Different prior (excluded cards) or rule variant (R3/R5/R7/R8).** Rebuild the fb tables and recompute F. The pipeline takes minutes (§4).

## 2. Algorithm components
### 2.1 Kernels (measured earlier, reused)
- Factorized u16 feedback code: 2.2 ns per pair.
- Per-field histograms: about 0.5 ms.
- Effective action classes: 0.12–0.2 ms.
- Exact top-64 by entropy using the upper bound: about 1.3 ms at n=5000.

### 2.2 Exact in-puzzle expectimax
- **Memo keys.**
  - r=1: (C as u64 bitset, K, M, h_eff, c_eff) in a u128. The caps h_eff = min(h, #unknown) and c_eff = min(c, |C|+6−|M|) are valid only with zero continuation.
  - r≥2: (C, K, M, h, c) with no caps. Inside a subtree c spans at most |C|+6−|M| values, so cost matches the old uncapped v2.
- Child nodes inherit the parent's effective-class action list; no-ops are pruned.

```rust
pub struct Node { set: CandSet, known: u8, matched: u8, h: u8, c: u8 }
pub enum CandSet { Small(u64), Wide { zob: u128, ids: ArenaSlice } }
pub type Val = [f64; 3];
#[derive(Clone, Copy)] pub struct Bounds { lo: Val, hi: Val }
pub struct ContRow { gs: [f64; 38], gm: [f64; 38] } // g(j) for this (r,h,c); g(c+1)=0
```

### 2.3 Admissible upper bounds (proofs)
**Definition.** P*_j(x) is the maximum over policies of P(solve within j challenges).

**P̄₁(x; h), exact.** Use k = min(h, u) hints first (u = number of unknown fields). Then P̄₁ = E over a uniform random k-subset S of unknown fields of Σ_{cells of C split by S} max_g w(FM(g)∩cell)/w(C).
- Cost: O(3n) per subset via FM⁻¹ accumulation, with at most 10 subsets. Memoize on (set, K, k).
- Proof: the solve must happen on the next challenge. Information cannot hurt, so using every available hint first is optimal, and the revealed field set is a uniform k-subset whatever the order.
- **Caution:** for j≥2, "hints first" is **not** a valid relaxation, because a hint drawn after a challenge avoids fields the challenge already revealed. The valid j≥2 relaxation is chosen-field hints (the policy picks the field), where hints-first is without loss.

**Lemma S (continuation).**
- Let g(j) = 1 + F_S(h, c−j) for j≤c and g(c+1) = 0. F must be nondecreasing in (h,c), which holds for F* by free disposal and is enforced on tables with a running max.
- For any policy, with J the number of challenges at the solve:
  V_S ≤ E[g(J)] = Σ_j P(J≤j)(g(j)−g(j+1)) ≤ Σ_{j=1}^{c} P̄_j·(g(j)−g(j+1)) =: U_S.
- Every difference is ≥ 0 (Abel summation), and F(h−H, c−J) ≤ F(h, c−J). In the final puzzle U_S = P̄_c.
- Here P̄₁ is as above, P̄_{j≥2} = 1, and optionally at large n the count bound min(1, top-N mass) with N₂ = 3 + 3(B−1), where B is the maximum number of outcomes.

**Lemma M.** U_M = (6−|M|) + Σ_j P̄_j (g_M(j) − g_M(j+1)).

**Lemma A (actions, "hint charging").**
- If c ≥ 2, then E[#actions] ≥ 2 − P̄₁(x; **0**), using the no-hint P̄₁. If c = 1, E[#actions] ≥ 1.
- Proof, for a deterministic policy (randomized policies are mixtures):
  - If it challenges first: E[A] ≥ 1 + P(miss) ≥ 2 − P̄₁(x;0), since no stop exists and the cell after a miss is nonempty.
  - If it hints first: E[A] ≥ 1 + E[C] ≥ 2.
- U_A = −LB. Continuation actions are ≥ 0 and are dropped.

**Pruning.** Componentwise bounds imply lexicographic bounds. Prune action a if U(a) <lex L(b) for some b. This is valid because Q(a) ≤lex U(a) <lex L(b) ≤lex Q*(b).

**Why the old bound fails.** The top-(c·k_max) bound is not admissible: an adaptive tree can solve k_max·(1+K+…+K^{c−1}) targets, where K is outcomes per guess.

**O(1) bound for guesses not expanded.** For c ≥ 2: U_S(g) ≤ Ps(g)·g(1) + (1−Ps(g))·g(2), with Ps from the FM lists. Covering all 8023 guesses takes about 0.5 ms. This is what allows claiming optimality over the **full** action set rather than over a top-K subset.

**Measured, final puzzle** (`exact3`; "reference" is v3 = inherit + caps, no pruning; "B&B" adds P̄₁ + Lemma A with Star1 ordering by Ps):

| n (#classes) | h,c | reference ms (states) | B&B ms (states) |
|---|---|---|---|
| 15 (1101) | 4,6 | 4530 (81.9k) | **4.7** (1.9k) |
| 15 (1101) | 4,12 | 7425 (141k) | **4.6** |
| 16 (561) | 4,12 | 5531 (192k) | **3.7** |
| 20 (728) | 4,6 | 52169 (1.07M) | **0.9** |
| 27 (804) | 2,3 | 4508 | **7.5** |
| 24 (1740) | 0,3 | 11637 | 151 |
| 23 (555) | 4,6 | 13108 (341k) | **16932** (328k): the bound fails, E[A]≈2.89 |

- All 260 rows agree with the reference (0 mismatches).
- For n ≤ 16, the maximum time is 64 ms with B&B versus 7.4 s for the reference.
- A first variant that put the hint-using P̄₁ into Lemma A's action term gave no gain when resources were loose. The hint-charging form is what matters.

### 2.4 Feasible lower bounds
Any concrete policy's exact value is a lower bound L. Three are used:
1. **Closed-form one-step value:** [Ps·g(1), E[gain] + Ps·g_M(1), −1], computed from histograms.
2. **Exact subtree value of the base (menu) policy:** O(subtree nodes). The whole-prior measurement was 0.03–0.16 ms per node.
3. **At a puzzle root:** the menu policy's distribution P(H=i, J=j) gives L = Σ P(i,j)(1 + F^lo(h−i, c−j)). This can be computed lazily in a second Worker; a root with n=5000 takes about 1–2 s native (estimate).

### 2.5 Two-sided bounded expectimax (the solver)
```rust
fn bsearch(&mut self, x: &Node, d: u8, cont: &ContRow) -> Bounds {
  if x.c == 0 || x.set.is_empty() { return Bounds::exact(ZERO) }
  if let Some(b) = self.tt.get(x) { if b.closed() || b.depth >= d { return b } }
  let u_node = self.node_ub(x, cont);            // Lemmas S/M/A
  let mut lo = self.leaf_lb(x, cont);            // §2.4 (1) or (2)
  if d == 0 || lex_le(u_node, lo) { return self.store(x, lo, u_node, d) }
  let acts = self.select(x);                     // effective reps (small n); top-K entropy ∪ top-Ps ∪ cands (large n)
  let mut hi = self.onestep_ub_rest(x, &acts, cont); // O(1)/guess for every guess not in acts
  for a in self.order(x, &acts) {                // Ps desc, then #cells desc
    let (mut q, mut rest) = (Bounds::zero(), 1.0);
    for cell in self.partition(x, a) {           // Star1 chance-node cutoff (Ballard 1983)
      let b = if cell.solved { Bounds::exact(cell.r + cont.g1()) } else { cell.r + self.bsearch(&cell.child, d - 1, cont) };
      q.add(cell.p, b); rest -= cell.p;
      if lex_lt(q.hi_plus(rest, u_node), lo) { break }  // cannot beat lo: prune
    }
    lo = lex_max(lo, q.lo); hi = lex_max(hi, q.hi);
  }
  /* hint: chance node over (field, value), same pattern */
  self.store(x, lo, lex_min(hi, u_node), d)
}
```
- Iterative deepening is driven by a work budget. The transposition table (TT) keeps [L,U] plus depth, and closed nodes (L=U) are reused across calls, including after the user records an outcome.
- The root bounds are valid at every instant, so there is nothing to discard.
- **Recommendation:** the argmax of L̂ under F̂, with ties broken by Û.
- **Tie set:** every action a with U(a) ≥ L(best).

### 2.6 Collapsing a flat continuation
If F_{r−1} varies by at most ε over the reachable window, treat it as constant. The final-puzzle caps and speed then apply, and ε is added to the gap. My exact F^lo table is flat to three decimals for c ≥ 9r in the tested rows, e.g. F_2(11, c≥18) = 2.000.

### 2.7 Numerics
- **r=1:** exact integers. Q(x) = w(C)·60^{h_eff}·V fits in i64 (≤ 9059·60⁵·50 ≈ 3.5e14). Hint children get the factor 60/u, which is an integer because u ≤ 5. This gives exact tie sets with no epsilon.
- **r≥2:** f64. The tie set is defined through bounds with ε.

### 2.8 Continuation tables and error propagation
**F^lo (exact, proven lower bound).**
- Build a menu Π of per-puzzle policies π_{λ,k} that ignore the budget apart from a hint cap k ≤ 5. λ is a price on challenges and hints used inside the policy's own lookahead.
- Evaluate each policy once over the whole prior by exact tree traversal. Record the joint P(H=i, J=j), plus E[|M_j|; J>j] and E[H; J>j] for the M and A components.
- Outer DP: F^lo_r(h,c) = max_{π,k≤min(h,5)} Σ_{i,j≤c} P_π(i,j)(1 + F^lo_{r−1}(h−i, c−j)).
- The result is monotone by construction and is the exact value of a policy that adapts at puzzle boundaries.

**F̂ (the table decisions use).**
- v1: F^lo of a rich menu.
- Later: Monte Carlo fitting with common random numbers (CRN) of the full online policy, clamped at ≥ F^lo and made isotonic.

**F^hi.** Lemma S at the prior uses the prior-averaged P̄₁ (random hints, exact) and P̄₂ (exact without hints: 0.1225; with hints, use the chosen-field version), with P̄_{j≥3} = 1. It is informative only when c is below about 2r, i.e. in the last puzzle or two.

**Error propagation.**
1. **Value.** If |F̂_r − F*_r| ≤ δ_r, the policy that is greedy with respect to F̂ loses at most V* − V^π ≤ 2Σ_{r<R} δ_r. The per-stage error is ε_r ≤ 2δ_{r−1} + ε_{r−1}.
2. **Per decision.** Q_a(F̂) − Q_a(F*) = E_a[1{solve}(F̂−F*)(h',c')]. Let δ_win be the maximum of max(F^hi−F̂, F̂−F^lo) over the window W = {h−5 ≤ h' ≤ h, c−(n+6−|M|) ≤ c' ≤ c−1}. Action a is truly optimal if L̂(a) − δ_win ≥ Û(b) + δ_win for every b≠a.
3. **The lexicographic trap.** Any δ > 0 makes comparisons on the second and third components unreliable unless the first is provably tied. The M and A orderings are claimed only in the final puzzle or in subtrees where a solve is guaranteed.

## 3. What is proven and what is approximate; UI labels
| Condition | Proven statement | UI (units: expected puzzles solved) |
|---|---|---|
| r=1, root closed, integer arithmetic | Exact optimum and exact tie set, all 3 components | 「精确最优（已证明）」 + value + equivalent cards |
| r=1 or r≥2, root open | For the recommended card V ≥ L̂; for every action V ≤ Û (whole action set) | 「已证明差距 ≤ Û−L̂」 |
| r≥2, root closed | Optimal relative to the F̂ table | 「本题内最优（相对续值表）」 + 「续值误差 ≤ δ_win」 |
| margin > 2δ_win | Truly optimal under the model | 「真实最优（已证明）」: rare before the last 1–2 puzzles |
| Always | The model assumptions A1–A6 and R3/R5/R7/R8/R15 | Model footnote |

- The M and −A components are shown only when the S gap is 0.
- The legacy label 「可行下界」 for a heuristic value disappears: every number shown is either a feasible-policy value or a proven bound.

## 4. Offline pipeline (native, rayon)
1. **Menu evaluation, once per data/rules/prior hash.** Exact whole-prior trees.
   - Measured for a weak greedy menu (20 threads):

     | Hint cap | Decision nodes | Wall time |
     |---|---|---|
     | 0 | about 52k | 0.6–2.1 s |
     | 1 | about 250k | 2.3–8.2 s |
     | 2 | about 1M | 8.7–32 s |
     | 3 | 3.16M | 70–75 s |

   - Memoizing hint transpositions (different hint orders reach the same (C,K)) should cut caps 2 and 3 by up to 2×/6× (estimate).
   - Rich menu items with lookahead-quality decisions cost 10–50× more per node, so about 1–60 min per item (estimate). Plan for 5 prices × caps 0–5.
2. **Prior bounds:** P̄₁ and P̄₂ at the prior, 1.55 s.
3. **What ships:**

   | Asset | Raw size | Notes |
   |---|---|---|
   | Menu distributions: \|Π\|·6 caps·(i≤5 × j≤24) plus failure-path stats | ≈ 30 KB | f32; lets the browser compute F^lo for **any** custom preset in under 1 ms |
   | F̂ and F^lo tables, 9×12×37×3 | ≈ 48 KB | u16 fixed-point; F^lo rounded down, F^hi rounded up; about 10–20 KB brotli (estimate) |
   | Optional per-root solve-time table for the lower bound at puzzle start | ≈ 60 KB | u16 |

## 5. Validation plan
1. **Brute-force reference.** No memo, no dedup, no pruning, run on random small synthetic pools (about 200 groups) under every rule variant. Values must be equal as integers when r=1 and within 1e-12 when r≥2.
2. **Bound property tests.** Assert L ≤ V ≤ U componentwise on every node of the reference runs. Also check B&B against no-pruning; so far 260/260 agree.
3. **F^lo cross-check.** A CRN Monte Carlo of the menu policy with 10k activities must fall within its CI. Check monotonicity in h, c and r.
4. **Policy A/B.** At least 10k CRN activities against the native legacy port on 9/11/36 and on tighter presets (9/11/30, 9/11/27, 9/6/30), where policies actually separate.
5. **Latency.** Measure the WASM factor early. Cap work units, and track p50/p99 on exported state corpora.

## 6. Experiments run
All work was in `/tmp/claude-1000/-home-parry-gitRepos-md-card-decoder/cb7e6cca-357d-4211-9aca-7cbb5c5749b0/scratchpad/panel-exact/rs` (a copy of the prototype), built with `cargo build --release`. I added `src/ptree.rs` and `src/exact3.rs`. Runs were on 20 threads; about 60 min in total, which is over the 30–40 min time box.

- **`mdbench ptree 20 0 1.0`.** Exact evaluation over the whole prior of a greedy policy (argmax entropy + 1·Ps, with the argmax taken exactly via the entropy upper bound).
  - 52,093 nodes, 1.57M partitions, 1.44 s wall.
  - E[J] = 4.603; P(J≤1..8) = .0050 .0856 .2917 .5383 .7330 .8570 .9301 .9708.
- **`mdbench ptree 20 3 2.0`** (hints first, caps k = 0..3).
  - E[J] = 4.600 / 3.555 / 2.564 / 1.756 for k = 0..3.
  - Exact family DP on 9/11/36: **E[S] = 8.87286, P(all 9) = 0.8935**. With k=0 only: 7.467 / 0.184. With k=1 always: 8.736 / 0.811.
  - Other presets: 9/11/30 → 8.386 / 0.561; 9/11/27 → 7.920 / 0.325; 9/6/30 → 7.528 / 0.194.
  - So F*_9(11,36) ≥ 8.873 exactly, under the model. The legacy Monte Carlo gives 8.98, so this menu is too weak to serve as F̂.
- **β = 4 and `MODE2=1` runs** (score = entropy + β·P(solve within 2)): no improvement (8.849–8.872).
- **`mdbench pstar 20`.** Prior-averaged, no hints: **P*₁ = 0.01413, P*₂ = 0.12248**, 1.55 s. The greedy policy reaches only 0.0050 and 0.086–0.101.
- **`mdbench exact3 6000 > ../exact3b.log`.** The table in §2.3. The first run with the hint-using P̄₁ in the action term is in `../exact3.log`.
- **Timing noise.** The reference time for n=15, h4, c6 was 13.0 s in the first run versus 4.5 s in the second, so the machine was loaded during the first; the earlier report measured 4.1 s.

## 7. Risks
- A bound bug silently produces wrong actions. Mitigation: the reference solver plus property tests (§5).
- The r=1 exact band still has outliers (n=23, h4, c6 takes 16.9 s even with B&B) and needs strict work budgets with the bounded search as fallback.
- r≥2 exact solving with B&B under a continuation is **unmeasured**. Without B&B, the uncapped cost was n=12: 458 ms, n=15: 0.4–7 s, n=18: 1.4 s.
- A weak menu biases F̂ toward saving resources. It must be at least as good as the legacy policy in simulation before shipping.
- F^hi is close to trivial early, so true-optimality certificates will be rare; the UI must not overclaim.
- Everything depends on A1–A6 and on the rules R3/R5/R7/R8/R15 being right; any change means recomputing F.
- WASM speed is still an estimate (1.3–2× native).

## 8. v1 vs later
**v1**
- Bounded expectimax with the Lemma S/M/A bounds and O(1) bounds for guesses not expanded.
- Exact final puzzle (caps + B&B + i64), gated by a work budget.
- F^lo built in the browser from shipped menu distributions.
- F̂ = rich-menu F^lo.
- Flat-continuation collapse.
- UI shows the gap and δ_win.

**Later**
- A P̄₂ relaxation with chosen-field hints to close the n≈20–30 hard band.
- P̄₃ at the prior, which is what makes F^hi useful.
- Monte Carlo policy iteration for F̂.
- A day-schedule dimension, tracking the prior without replacement, and a scalar points objective.