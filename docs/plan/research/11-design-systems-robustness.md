# Card Decoder solver design, systems lens: anytime Worker engine, pondering, robustness

## 0. Summary

- **One pure function.** The engine exposes `recommend(state, budget_level) -> Recommendation`. It runs in a Web Worker and gives the same output on native and WASM, on any machine. Work is counted in units, not milliseconds, and wall-clock time is only a safety stop.
- **Pondering replaces transposition-table (TT) reuse across user moves.** Because the function is pure, a result computed while the user is playing in-game is exactly what a live call would return. After each recommendation, spare Workers work through the likely next states in probability order and cache their results.
  - Measured: pondering all outcomes of the recommended action takes p50 22 ms and max 563 ms native, on a heavily loaded machine.
  - The next query is therefore almost always a cache hit.
- **The biggest robustness risk is the rule assumptions, not the prior.** I measured this with 2000 common-random-number (CRN) activities per configuration.
  - If the game uses a different R3/R5 rule than the model, the naive filter drops the true card. E[solved] falls from 8.99 to 4.50–6.43.
  - Tracking all 4 rule variants and searching under the most likely one recovers 8.98, and hedging costs about 0 when the default rule is true.
  - A wrong prior shape costs only about 0.02 solved.
  - Restricting guesses to cards consistent with known fields (R15) costs about 0.005–0.012 solved.
- **The search itself:**
  - Iterative deepening with root widening, because action-set width matters more than depth (measured).
  - An exact, full-width depth-1 frontier.
  - A cost-gated exact tier.
  - A continuation table for each objective.

---

## 1. Formal model and assumptions

**Activity state**

σ = (k, h, c, B, π, D)

- k: puzzle index, 1..P. r = P−k+1 puzzles remain.
- h, c: hint and challenge inventory.
- B: puzzle belief.
- π: posterior over rule variants. It carries over from puzzle to puzzle, because the rules are global.
- D: day state. It is empty in the accumulating model (see §2.8).

**Puzzle belief**

B = ({C_ρ}ρ, K, M, ε⊥)

- C_ρ ⊆ U: the groups consistent with the logs under rule variant ρ, with weights w_t.
- K: known fields (6-bit). M: fields matched by challenges (6-bit).
- ε⊥: posterior mass on an "unknown card" (not in the dataset).
- ρ ranges over the 4 variants {R3: intersect | pendulum guess needs pendulum target} × {R5: numeric | same-kind}. R7/R8 affect ≤20 weighted pairs, so they are folded in later as extra variants.

**Actions**

- `Hint`, when h>0 and K≠63.
- `Challenge(g)`, when c>0 and g ∈ A(B).
  - Default: A(B) = all MD groups.
  - Under R15: only g with v_f(g) = known v_f for every f ∈ K.
  - Already-guessed cards are no-ops and strictly dominated, so they drop out of every key (proved and verified in the prior reports).

**Observations**

- A hint returns (f, v_f(t)), with f uniform over the unknown fields. The level field shows the printed n.
- A challenge returns code_ρ(g,t) = (mask, frame reveal if the frame matched, printed n if the level matched), as a u16 below 3360.
- The puzzle is solved iff mask = 63.

**Transitions.** Beliefs update by exact Bayes filtering on each C_ρ. The rule posterior updates as

π_ρ ∝ π⁰_ρ · Π_(solved puzzles) 1[target consistent under ρ] · (mass C_ρ / mass C⁰) for the current puzzle.

When a puzzle is solved its target is known, so replaying its logs checks every ρ exactly.

**Prior.** P(t) ∝ (1−λ)·w_t/Σw + λ/|U|, with λ = 0.1 as a hedge. The ⊥ mass starts at ε⊥⁰ = 0.005. For ⊥, the likelihood of an observation is the product of the per-field response marginals over U. ⊥ can never be solved.

**Initial reveal.** Exactly one field, uniform over the 6, from the target drawn by the prior (app.js `seedTestInitialReveal`).

**Budget.** Hints and challenges are shared across the whole event (legacy model). If challenges run out, the remaining puzzles stay unsolved.

**Objectives.** The user picks one; all three use the same recursion.

V_J(σ) = max_a Σ_o P(o|σ,a) [ r_J(σ,a,o) + V_J(σ') ]

and at a solve, V_J(σ') = W_J[k+1][h][c′].

| J | Per-step reward | Continuation at a solve | Notes |
|---|---|---|---|
| LEX | (1{solve}, Δ|M|, −1), compared lexicographically | (W_S, W_M, 0) | legacy objective; M≈6S, so ≈E[S] |
| PTS | thresholdGain(k, M, M∪m) + 1{solve}·solvePts(k) | W_PTS[k+1][h][c] | the table is indexed by k, because premium status depends on k |
| ALL | 0 | W_ALL[k+1][h][c] = P(all remaining solved) | no reward on failure; this is the tail objective |

---

## 2. Components

### 2.1 Kernel layer (measured in the prior reports)

- u8 struct-of-arrays columns and a factorized feedback code: 2.2 ns per pair.
- Per-field histograms: P(solve), E[new matches] and per-field response entropies for all 8023 guesses in 0.13–0.50 ms.
- Entropy upper bound H ≤ Σ_f H(resp_f): exact top-64 with 84–206 full partitions, about 1.3 ms at n=5000.
- Projected-signature dedup (exact): 0.12–0.20 ms.
- Rule variants only change the 16×16 frame table and a level table with kind-aware ids. They never touch the hot loop.

**New exact reduction: a full-width depth-1 frontier.** At a node with depth 1 left, each challenge is worth

[Ps(g)(1+W_S), E[gain(g)] + Ps(g)·W_M, −1].

The first component rises with Ps. Every candidate t is in its own full-match set FM(t), so max Ps > 0. That means every argmax lies in N(C) = ∪_{t∈C} FM(t), which has at most 3|C| members (8149 FM pairs in total).

- The exact maximum over **all 8023 guesses** therefore costs O(|C|) per frontier node, instead of O(|C|+A) over a restricted list.
- Frontier nodes use the best one-step continuation, not the root's 26 actions.
- This is exact and costs about the same as the current closed-form leaf. Estimated: at most about 1.3× the leaf cost.

```rust
pub struct Kernels<'a> { t: &'a CardTable, rules: &'a RuleTables /* per RuleId */ }
impl Kernels<'_> {
    fn field_hist(&self, c: &[Gid], out: &mut FieldHist);            // O(|C|)
    fn quick_all(&self, h: &FieldHist, fm_in_c: &BitSet129) -> QuickRow; // O(U) ≈0.5 ms
    fn frontier_best(&self, c: &[Gid], m: u8, cont: ContVal) -> (Val, Gid); // O(|C|) via N(C)
    fn partition(&self, g: Gid, c: &[Gid], rule: RuleId, arena: &mut Arena) -> Cells; // 2.2 ns/pair
    fn top_k_entropy(&self, c: &[Gid], k: usize) -> SmallVec<[Gid; 64]>; // UB-pruned
}
```

### 2.2 Belief layer: rule mixture, ⊥, input-error recovery

```rust
pub struct RuleSlice { rule: RuleId, logw: f64, cands: Vec<Gid>, zob: u128 } // zob = XOR of per-gid keys
pub struct Belief { slices: SmallVec<[RuleSlice; 4]>, known: u8, matched: u8, unknown_logw: f64 }
fn observe(&mut self, obs: &Obs);        // filters every slice, O(Σ|C_ρ|)
fn map_rule(&self) -> RuleId;            // v1 search rule
fn collapse(&mut self, thresh: f64);     // drop slices with π_ρ < 1e-4, permanently for the activity
fn relax1(&self, logs: &[Log]) -> Vec<(LogIdx, Vec<Gid>)>; // cands violating exactly one log
```

- **Search rule (v1):** filter under all 4 variants, but run the search on the MAP slice. This is what I measured (§6).
- **Search rule (v2):** a true mixture search. Each ρ-slice is partitioned with its own code, and the cells are merged by code, since the observation space is shared. Cost is ≤ Σ_ρ|C_ρ| ≤ 4|C|, and it falls back to 1× after collapse.
- **Contradictions.** A slice going empty eliminates that ρ, and so does a solved-puzzle replay. If every slice is empty, `relax1` runs in O(U·L), about 0.1 ms. It ranks log entries by how much candidate mass they would restore, and the UI asks whether that entry was mis-entered, e.g. "第3次挑战的反馈可能录入有误？". Mis-entry is the most likely real-world contradiction.
- **⊥ handling.** ⊥ is not used in the search, because an unknown card can never be solved and it lowers every challenge's Ps uniformly. It is shown on screen, and the tool warns when P(⊥) > 0.3.
- **R15.** R15 is an `ActionFilter` predicate inside `RulePolicy`, and it is part of every cache key.

### 2.3 Search: iterative deepening with root widening and a deterministic budget

**Base set B.** B is computed once per query and fixed for that query. It contains:

- the top-18 by (Ps, E[gain]),
- the top-16 by exact entropy (selected with the upper bound),
- the top-8 candidates by weight,
- all of it deduplicated with effective classes.

**Nodes.**

- Interior nodes inherit B, filtered and deduplicated. Dedup only gets coarser on subsets, so children inherit the parent's representatives.
- The TT key is (Zobrist128(C_ρ), rule, K, M, h, c, k, depth-left) together with B's fingerprint.
- Leaves use `frontier_best` (full width, exact).
- Hint nodes: 6|S| work per node, grouped by printed value.

**Rungs.** Each rung completes atomically and yields (action, Val, rung-id). The answer is the last rung that completed.

| rung | search | measured native cost for its class |
|---|---|---|
| R0 | quick_all + frontier_best at the root (depth 1, all guesses) | ≤0.5 ms |
| R1 | depth 2, B | 0.4–2.5 ms |
| R2 | depth 3, B (about 26–42 actions) | 10–72 ms at openings, ≤2.5 ms mid-game |
| R3 | depth 3, root widened to B ∪ top-48 entropy | 39→92 ms at n≈2258 (26→46 actions) |
| R4 | depth 3, root widened to 96/140 | 229/724 ms at n≈2258 |
| R5 | depth 4, when n is small | – |

- Widening only adds **root** actions. Child subtrees depend only on (child, B, depth), so the TT carries over across R2→R4 and only the new root children are searched.
- The measured payoff from width at n=2258: depth-3 E[S] goes 0.296→0.368→0.487→0.611 as the root grows 26→46→76→140 actions.

**Budget.**

- 1 unit = 1 feedback-code evaluation plus histogram add. There is also a fixed charge of 256 units per node visit (an estimate, to be calibrated).
- Levels:

| level | units | native (estimate) | WASM (estimate) |
|---|---|---|---|
| L1 interactive | 25M | about 55–60 ms | 75–120 ms |
| L2 | 50M | about 110 ms | 145–220 ms |

- The p50 target (≤50 ms) is met because most states are small: measured depth-3 decisions had p50 5.2 ms, even under load.
- A wall-clock stop at 240 ms returns the last completed rung and sets `truncated_by_wall=true`. Only in that case can results differ between machines.

```rust
pub struct Budget { units: u64, spent: u64, wall_ms: f64 }
pub enum Rung { Frontier, D2, D3 { width: u16 }, D4 { width: u16 } }
fn search(&mut self, s: &State, b: &mut Budget, tt: &mut Tt) -> SearchOut; // returns per-rung results
```

**Why no subtree TT reuse across user moves.** It would need action sets that depend only on the node's state. That means running quick_all at each interior node, 0.3–0.5 ms each. With about 520 interior nodes at depth 3, that adds about 200 ms. Pondering gets the same latency benefit at no extra cost (§2.5).

### 2.4 Exact tier

- In-puzzle exact Bellman: u64 bitsets for n ≤ 64, keys without the guessed set, effective dedup, resource caps.
- The caps are c_eff = min(c, |S|+6−|M|) and h_eff = min(h, #unknown). They apply **only** when the continuation is zero, i.e. on the final puzzle, and the type system enforces this.
- On non-final puzzles, the tier runs with W as the continuation and without caps, labelled "题内精确，跨题估计".
- Gate: predicted states ≈ f(n, effective actions, h_eff, c_eff), checked against the remaining budget. It runs after R2, and its result overrides the search only if it completes.
- Measured: ≤5 ms for n ≤ 7 at any resources; ≤350 ms for n ≤ 13; 2.4–4 s for n≈15 with h4/c6. The last case needs pondering or a later branch-and-bound on E[actions].
- State cap: 2^18 entries, about 10 MB.

### 2.5 Recommendation cache and pondering

**Cache key.** H128 of (engine and data version, preset hash, objective, R15, k, h, c, K, M, [(ρ, zob_ρ, round(logw_ρ, 0.01))], budget level).

**Ponder queue.** Priority = P(reach) × (expected live cost). Tasks come from four sources:

1. Every outcome cell of the recommended action.
   - A challenge gives its non-solving cells; the last challenge gives none.
   - A hint gives all (field, value) cells, with prob = mass/T/|unknown|.
   - Measured: the realized outcome is always in this set unless it was a solve.
2. Outcomes of the tied alternatives and of the second-best action, for users who deviate.
3. When P(solve now) > 0.3, the next puzzle's openings for (h, c−1, k+1): the 218 (field, value) pairs in order of (1/6)·mass(v)/Σw.
   - Estimated about 5 s native on one thread, because openings are the expensive states. With 3 ponder Workers, about 2–4 s in WASM (estimate).
4. Grandchildren of the most likely children.

**Preemption.** One task = one `recommend` call, ≤0.6 ms–0.6 s native. The Worker checks its message queue between tasks. A new observation cancels the queue, and a stale live query is dropped by request id; `terminate()` is the last resort. On battery or mobile, pondering is capped at 2 s of CPU per user action (a setting).

### 2.6 Worker layout and memory

- **Workers.** N = clamp(hardwareConcurrency − 1, 1, 4). All of them are instantiated from one compiled `WebAssembly.Module` that is posted to each; the data is ≤100 KB per instance.
  - W0 answers live queries.
  - W1..N−1 ponder.
  - There are no WASM threads, so no cross-origin isolation (COOP/COEP) headers are needed.
- **Root-parallel mode (v2)**, used when a live query misses the cache:
  - Every Worker builds the same B.
  - Worker j takes root actions i ≡ j (mod N) and returns per-action Val for each completed rung.
  - The coordinator merges at the deepest rung completed for every action, breaking ties by lowest gid. The result stays deterministic.
  - Estimated speedup: 1.5–2.5× on 4 Workers, with losses from duplicated transpositions (unmeasured).
- **Memory per Worker.**
  - WASM max memory is 64 MB (`--max-memory`).
  - TT: 2^17 entries × 48 B, about 6 MB, with depth-preferred replacement.
  - Recommendation cache: 2048 entries LRU, about 2 MB.
  - Per-query arenas, reset between queries.
  - Exact tier: 2^18 states.
  - On a memory trap, the Worker respawns and the budget drops one level.

### 2.7 Choosing an objective

- Every objective runs the same search. Only r_J and W_J change.
- Default preset: LEX, ALL and PTS mostly agree (P(all) ≈ 98%).
- Tight presets split them apart. At 6 hints / 27 challenges, E[S] = 7.31 and P(all) = 10.6% (measured).
  - PTS moves resources to the premium puzzles 1–3, which pay 100 points each versus 6.
  - ALL takes more risk once all-9 is unlikely.
- How the user chooses:
  1. **A "目标" selector**, defaulting to LEX for continuity.
  2. **A 3×3 table for each preset.** Rows: the policy optimized for X. Columns: its E[S], P(all) and E[points]. Built-in presets get the table precomputed natively; custom presets get it from a browser simulation.
  3. **An agreement badge.** After the main search, the top-3 root actions are re-scored under the other two objectives at R1 (depth 2, 0.4–2.5 ms). The UI then shows "三个目标推荐一致" or which objective disagrees.

### 2.8 Sketch: daily grants that do not accumulate

- The state gains D = (day d, c_today, h_today, banked hints).
- Puzzles carry over between days, so running out of challenges today is **not** terminal. The leaf becomes V(B, h′, g_c, d+1). Only running out on the last day ends the activity.
- Consequences:
  - If only the *challenges* expire and the player plays every day, the model is equivalent to the total-inventory model. Challenges are never worth saving across a day boundary when the puzzle carries over.
  - If *hints* expire, a hint's marginal cost drops to about 0 near the end of the day. Bellman then spends leftover hints before the day's last challenge.
- Engine changes: W_J[k][h][c_today][days_left], plus a synthetic "day ends" transition at c_today = 0. The search code stays the same; only the continuation and the resource transition change.

### 2.9 Explainability output

```rust
pub struct Recommendation {
  action: Action, ties: Vec<Gid>,                    // within explicit tol 1e-9 on Val
  label: ProofLabel, rung: Rung, units: u64, cache_hit: bool, truncated_by_wall: bool,
  stable: bool,                                      // same action in last two completed rungs
  hint_vs_challenge: Option<HvC>,                    // both computed at the same rung
  per_action: Vec<ActionStats>,                      // top 3–6
  posterior: Posterior,                              // top-N cards with probability, n_groups, mass
  rules: [(RuleId, f32); 4], unknown_mass: f32, contradiction: Option<Relax1Report>,
  objective_values: [ObjVal; 3],                     // chosen plan under LEX/PTS/ALL
}
pub struct HvC { v_hint: Val, v_chal: Val, d_solve_puzzle: f32, d_p_all: f32,
                 hint_in_challenges: f32 /* ΔW/Δh ÷ ΔW/Δc at (k,h,c) */ }
pub struct ActionStats { g: Gid, p_solve_now: f32, e_new_matches: f32, entropy: f32,
                         outcomes: SmallVec<[(Code, f32 /*prob*/, u16 /*cands*/); 8]>, worst_cands: u16 }
```

The UI explains a hint recommendation in these terms: "提示: 本题解出概率 +x%，全通概率 +y%；当前 1 提示 ≈ z 挑战".

---

## 3. What is optimal and what is approximate, with UI labels

| Claim | Status | UI label |
|---|---|---|
| Filtering, posteriors, P(solve now), E[new matches], entropies | exact under the model and the chosen ρ | none |
| Full-width depth-1 frontier (N(C) reduction) | exact | – |
| Final puzzle, exact tier completed | optimal within the model (prior, ρ) | 已证明最优（模型内） |
| Non-final puzzle, exact in-puzzle with W | optimal given W; W is fitted | 题内精确·跨题估计 |
| Search rungs | best among the restricted root set, depth d, fitted W | 搜索近似 d=3/K=48 · 稳定/变化中 |
| Rule posterior π_MAP < 0.99 | recommendation is conditional on the MAP rule | 规则未确认（依据 ρ̂） |
| Wall-clock truncation | may differ between machines | 已截断 |

The legacy "可行下界" label is dropped: the value includes a heuristic continuation, so it is not a lower bound.

---

## 4. Offline pipeline (native, rayon, 20 threads)

1. **Data:** `xtask data` builds `cards.bin`, about 22 KB brotli. Rule tables are built at runtime from `RulePolicy`, so nothing extra ships.
2. **Continuation tables:** W_LEX(S,M), W_PTS and W_ALL for each built-in preset, fitted by approximate policy iteration with CRN, backward over k, isotonic in h and c. Rows are per k so that premium puzzles are handled. There is also a generic grid for custom presets up to P ≤ 12, h ≤ 15, c ≤ 50.
   - Cost: about 3 min per round at depth 2 (prior report), so 3 objectives × 3 rounds ≈ 30 min.
   - Size: 9×12×37×4 f32 ≈ 64 KB for the default preset, and about 160 KB for the generic grid (raw).
3. **3×3 objective table** for each preset: 3 × 10k CRN activities, about 1 min at depth 2.
4. **Latency corpus:** 10k states sampled from simulated trajectories, for benchmarks.
5. **Golden fixtures:** see §5.

**Shipped:** wasm (estimated 150–250 KB brotli), `cards.bin`, W tables (≤100 KB brotli, estimate), and names loaded separately.

---

## 5. Validation plan

1. **Determinism:** the same (state, level) must give an identical `Recommendation` on native, WASM, and 1/4/20 threads. This needs libm, no FMA, and a fixed summation order.
2. **Exactness:** `frontier_best` against a brute-force max over 8023 guesses on 1000 random states. Exact tier against a naive reference on random small pools, under all 4 rules.
3. **Rules:** the official Zarc examples; self-guess gives 63 under every ρ; code_ρ partitions agree with the filter.
4. **Anytime quality:** per corpus state, the reference-evaluated value of the chosen action must not decrease from rung to rung (statistical check). Measure how often the answer is `stable`.
5. **Latency:** headless Chrome (Playwright) running WASM on the corpus, with 1× and 4× CPU throttling. Gates: p50 ≤ 50 ms and p99 ≤ 250 ms on live misses.
6. **Pondering:** simulated users with log-normal think time (median 6 s) and 10% deviation from the recommendation. Report the cache hit rate and ponder CPU.
7. **Policy quality:** 10k–20k CRN activities on the default preset, 6h/27c and 4h/24c, new engine versus the legacy port. Report paired differences in E[S], P(all) and E[points] with confidence intervals.
8. **Robustness:**
   - truth ρ ≠ model ρ;
   - uniform-truth prior;
   - 3% of groups excluded from the truth;
   - truth outside the dataset (tests ⊥);
   - 2% user mis-entry (tests `relax1` recovery).
9. **Field check:** an opt-in log export. Replaying solved puzzles eliminates ρ. Pooling about 5 users' logs should confirm R3 and R5 with about 97% probability (estimated from §6: about 50% per user per axis).

---

## 6. Experiments

**Setup**

- Copied the prototype to `/tmp/claude-1000/-home-parry-gitRepos-md-card-decoder/cb7e6cca-357d-4211-9aca-7cbb5c5749b0/scratchpad/panel-systems/rs`.
- Added `src/exp.rs` (rule-variant codes, a 4-slice mixture filter, R15 filter, pondering timer), an `exp` mode in `main.rs`, and `Pool.bvals`.
- Built with `cargo build --release`.
- Command: `./target/release/mdbench exp N THREADS DEPTH H0 C0 RESTRICT PONDER TRUTH MIXTURE [SEED=1000] [TRUTH_UNIFORM]`. The env variable `MODEL_UNIFORM=1` sets all model weights to 1.
- Truth bits: 1 = R5 same-kind, 2 = R3 strict pendulum, 3 = both.
- Batch file: `run_batch.sh`, output in `batch.log`.
- Policy: legacy-style action set at depth 2, legacy continuation, seeds 1000..2999, so the runs are paired.

**Results (N = 2000 per row)**

| Preset | Configuration | E[S] (se) | P(all) | Empty-set puzzles per activity |
|---|---|---|---|---|
| 11/36 | model = truth = default | 8.9870 (0.0026) | 98.75% | 0 |
| 11/36 | truth R5, naive | 6.4325 (0.0715) | 51.30% | 0.477 |
| 11/36 | truth R5, mixture | 8.9825 (0.0029) | 98.25% | 0 |
| 11/36 | truth R3, naive / mixture | 6.1410 / 8.9825 | 47.85% / 98.35% | 0.513 / 0 |
| 11/36 | truth R3+R5, naive / mixture | 4.4950 / 8.9815 | 23.75% / 98.20% | 0.754 / 0 |
| 11/36 | truth default, mixture (cost of hedging) | 8.9855 (0.0027) | 98.55% | 0 |
| 11/36 | R15 restricted guesses | 8.9815 (0.0032) | 98.25% | – |
| 11/36 | truth uniform over groups, model ∝ copies | 8.9840 (0.0030) | 98.50% | – |
| 6/27 | default | 7.3070 (0.0215) | 10.60% | – |
| 6/27 | R15 | 7.2950 (0.0219) | 11.15% | – |
| 6/27 | truth R3+R5, naive / mixture | 4.1890 / 7.2085 (0.0220) | 2.45% / 9.90% | 0.669 / 0 |
| 6/27 | truth uniform: model ∝ copies / model uniform (matched) | 7.2160 / 7.2385 | 9.20% / 9.20% | – |

**Rule discrimination along default trajectories**

- Share of puzzles with at least one challenge that discriminates the rules: R5 7.05%, R3 7.68%, either 14.69%.
- P(first discriminating observation by puzzle 1 / 3 / 9): R5 6.7 / 18.2 / 47.8%; R3 7.5 / 21.5 / 51.4%.
- Full identification of all 4 variants from one user's activity by puzzle 1 / 3 / 9: 0.1 / 3.5 / 23.1%.
- Conclusion: do not wait for identification. The mixture filter is what protects play.

**Latency and pondering** (`exp 24 4 3 11 36 0 1 0 0`, depth 3)

- Other jobs had the machine at load average 37–38 on 20 hardware threads, so treat these as upper bounds.
- 944 decisions: p50 5.2 ms, p90 91 ms, p99 294 ms, max 432 ms.
- Pondering, 791 events:
  - children per recommended action: p50 13, p90 35, max 113;
  - time to cover 90% of outcome mass: p50 17 ms, p90 138 ms, p99 348 ms;
  - time for all children: p50 22 ms, p90 171 ms, max 563 ms;
  - realized child ready after pondering starts: p50 12.8 ms, p90 229 ms.
- 92.2% of events had the realized child ready within 1 s. The other 7.8% were solve or last-challenge outcomes, which need no in-puzzle recommendation.
- Depth-2 decisions (N=2000, 20 threads, loaded machine): p50 1.18 ms, p99 33.5 ms, max 159 ms.

**Caveats.** All policies here are the legacy-style depth-2 policy. The mixture search used the default rule to partition. The R15 cost may grow with wider, more informative action sets. No WASM measurements were made.

---

## 7. Risks

- **WASM speed is unmeasured** (assumed 1.3–2× native), and my timings come from a loaded machine. Measure early with the latency corpus.
- **Determinism can break on floating point:** f64 summation order, libm versus `Math.log2`, and tie tolerance. A result truncated by the wall clock is not reproducible, so it is labelled.
- **W tables** only hold for the preset and policy they were fitted on. Custom presets fall back to the generic grid and get the "估计" label.
- **Rule prior π⁰ is subjective.** While the rules are unconfirmed, a wrong MAP choice early in the activity costs about 0.1 solved on tight presets (7.2085 vs 7.3070).
- **`relax1` may blame the wrong log.** Always show it as a question, never apply it automatically.
- **Pondering costs CPU and battery,** and its hit rate falls when users deviate. That is why tied and second-best actions are pondered too, with a CPU cap.
- **Opening pondering** (218 states) may not finish before the user enters the next reveal. Openings are the worst live-latency case (R2 costs 72 ms native at n=5000). A miss there falls back to the rung ladder, possibly with root-parallel search.
- **Model scope:** if the day structure or R15 behaves differently in-game, parts of the model change. Both are switches that enter the cache key.

---

## 8. v1 and later

**v1**

- Kernel layer and the full-width frontier.
- 4-variant rule filtering with MAP search and collapse.
- R15 toggle, on by default, since the measured cost is ≤0.012 solved and every recommendation is then selectable in-game.
- ⊥ display and `relax1` contradiction recovery.
- Rungs R0–R4 with deterministic units and a 240 ms wall-clock stop.
- Cost-gated final-puzzle exact tier.
- Recommendation cache and pondering of the recommended, tied and second-best actions' outcomes.
- One live Worker plus 1–3 ponder Workers, with bounded memory.
- LEX objective with a fitted W_LEX (legacy formula as a fallback).
- The full `Recommendation` explanation, with honest labels.

**Later**

- W_PTS and W_ALL with the objective selector, the 3×3 table and the agreement badge.
- Pondering next-puzzle openings.
- Root-parallel live search.
- True mixture search.
- In-puzzle exact tier with W on non-final puzzles, and branch-and-bound lower bounds on E[actions] for n≈15.
- A learned leaf V̂(entropy bucket, h, c, unknown fields, k).
- The day-structure model, if the in-game grants turn out not to accumulate.
- A log-replay service so users' pooled logs can confirm R3, R5 and R7.