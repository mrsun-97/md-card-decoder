# Card Decoder solver design: approximate DP with a learned evaluation

Designer "approx". This design replaces the legacy zero leaf and the continuation formula `min(r,c,(c+0.61h)/3.9)` with a value model fitted offline natively and shipped as two small tables. It runs a depth-2 expectimax with exact outcome enumeration and a learned leaf. I tested it against a native port of the legacy policy, using common random numbers (CRN):

- **Default 9/11/36, 10k activities:** P(all 9) goes from 98.51% to 99.84% (+1.33 ± 0.24 pts). Failed activities drop from 149 to 16.
- **Stress 9/11/28:** 65.25% → 84.90%.
- **Stress 9/4/30:** 14.37% → 23.17%, with E[S] +0.387 ± 0.033.

Those gains need both parts: the fitted table alone does nothing (dE[S] −0.001 ± 0.002), and the learned leaf with the legacy formula is worse than legacy. Section 6 has all the numbers.

---

## 1. Formal model and assumptions

**Pool.** U = 8023 behaviour groups with weights w (Σ = 9059). Each target is drawn i.i.d. ∝ w. Each puzzle starts with **one** revealed field, drawn uniformly from the 6 (checked in `seedTestInitialReveal`, app.js:1357).

**State** (the guessed set is left out; this is value-preserving and proven in report_algo):
```
s = (k, h, c, C, K, M)
  k = number of fresh puzzles still to come after the current one (k = P − j)
  h, c = shared hint and challenge inventory
  C ⊆ U = consistent candidates; the posterior is exactly w restricted to C
  K = known-field mask, M = mask of fields matched by challenges
```
The posterior stays ∝ w on C, because the hint's field choice is independent of the target and all feedback is deterministic.

**Actions.**
- HINT (h > 0, K ≠ 63): field f is uniform over the unknown fields; it reveals the printed value (level shows n, frame shows the full b).
- CHALLENGE g (c > 0): g ranges over effective equivalence classes. Feedback o = (6-bit mask, target b if frame matched, target n if level matched). Solved iff mask = 63.
- STOP (value 0). Only relevant when no action has positive value.

**Bellman equation** (vector values Val = (S, M, −A), compared lexicographically; expectation is linear, so lexicographic expectimax is consistent):
```
V(s)   = max{ 0,
              Σ_{f∉K} (1/|K̄|) Σ_v P(v|C,f)·V(k,h−1,c,C_fv,K|f,M) − e_A,
              max_g Σ_o P(o|C,g)·[ r(o,M) + (solved(o) ? W(k,h,c−1) : V(k,h,c−1,C_o,K|m,M|m)) ] }
r(o,M) = (1{m=63}, |M∪m| − |M|, −1)
W(k,h,c) = E_{t∼w, f0}[ V(k−1, h, c, C_{f0,t_f0}, 1<<f0, 0) ],   W(0,·,·) = 0
```
Two value functions get approximated: **W** (across puzzles) by the table F̂[k][h][c], and **V** at the search cutoff (within a puzzle) by the leaf V̂.

**Objective.** The legacy lexicographic (E[S], E[M], −E[A]). A solved puzzle ends with M = 63, so M = 6·S plus the partial matches in the failing puzzle. In practice the objective is E[S] with two tie-breakers. The machinery takes any per-puzzle reward vector: the table is indexed by k = P − j, so premium-puzzle milestone points only mean refitting it.

**Assumptions** (all parameters of `RulePolicy`; tables are fitted per policy):
- R3 (pendulum guess vs non-pendulum target), R5 (level/rank/link cross-match), R7/R8 (rule-level reveal) and R15 (unrestricted guess list) follow the legacy defaults.
- Budgets are a total inventory. If daily grants turn out not to accumulate, the table gains a day index; the method is unchanged.
- **Structural fact:** with K = 63, every remaining candidate shares all printed values, so any guess from C fully matches. Five hints plus one challenge therefore always solve a puzzle. The fitted table confirms this: F̂[1][11][1] = 1.000. This is why hints are worth far more than the legacy 0.61 challenges.

---

## 2. Algorithm components

### 2.1 Exact kernels (reused from report_algo, measured)
- Factorized u16 outcome code: 2.2 ns per pair.
- Per-field histograms giving P(solve), E[new matches] and response entropies in O(|C|+U): 0.13–0.50 ms.
- Effective equivalence classes through an O(1) signature: 0.12–0.20 ms.
- Exact top-K entropy using the bound H ≤ Σ_f H_f: 84–206 partitions for K = 64, about 1.3 ms at n = 5000.

### 2.2 Continuation table F̂ ≈ W
```rust
pub struct ContTable { kmax: u8, hmax: u8, cmax: u8, s: Box<[u16]>, m: Box<[u16]> } // fixed point /4096
impl ContTable {
    #[inline] pub fn get(&self, k: u8, h: u8, c: u8) -> [f64; 2]   // clamps h, c; k==0 || c==0 -> 0
}
```
It is fitted by backward induction over k with Monte-Carlo puzzles (section 4). Built-in constraints:
- monotone repair (running max in c, then in h);
- F̂ ≤ min(k, c), a true bound because every solve costs at least one challenge;
- CRN across cells, so differences between cells are low-noise.

The fitted values differ from the legacy formula in a structured way:

| cell (k=8) | F̂ | legacy |
|---|---:|---:|
| h0 c4 | 0.58 | 1.03 |
| h0 c20 | 4.04 | 5.13 |
| h11 c20 | 6.99 | 6.85 |
| h11 c4 | 2.33 | 2.75 |

From F̂[8][0][·] against F̂[8][11][·], one hint is worth about **1.1–1.4 challenges** at the margin, where the legacy formula uses 0.61.

### 2.3 Learned leaf V̂: a distributional cost-to-go model
Regressing V directly on (features, h, c, k) would have to relearn the composition with W in every cell. Instead the leaf learns the distribution of **resources still needed to solve this puzzle** and composes it with F̂ analytically:
```
φ(s)   = (b, flag)
b      = round(2·H(C)) in 0..27        // H = log2 m − Σ w·log2 w / m, computable per cell in O(1)
flag   = 1{h > 0}
P(J, E | φ)                            // J ∈ 1..14 challenges to solve, E ∈ 0..6 hints used
V̂_S(s) = Σ_{j,e} P(j,e|φ)·1{j≤c}·(1 + F̂_S[k][h−e][c−j])
V̂_M(s) = Σ P·1{j≤c}·((6 − |M|) + F̂_M[k][h−e][c−j])
V̂_A(s) = −Σ P·(min(j,c) + e)
```
```rust
pub struct CostToGo { nb: u8, jmax: u8, emax: u8, p: Box<[u16]> }        // [b][flag][j][e]
pub struct LeafCache { k: u8, rows: [[[f64; 4]; CMAX]; 3] }               // h, h−1, h−2; built once per decision
#[inline] fn vhat(&self, hbits: f64, h: u8, c: u8, matched: u8) -> Val    // O(1): one row lookup
```
- Each Σ_{j,e} is precomputed into `(PS, VS, VMcont, VA)` per (b, h, c) for the current k. That is 28·3·37 rows of 112 terms, well under 1 ms per decision (estimate).
- **Why this form:** the one thing a zero leaf cannot express is "how many resources remain when this puzzle ends". Here it enters through F̂, so the hint-versus-challenge trade-off falls out of the fitted table and not from a magic ratio.

### 2.4 Search: depth-2 expectimax with the learned leaf at depth-1 nodes
```
decide(s):
  A_root = effclasses( legacy(18 by (P_solve, E_new) ∪ 8 by infoApprox ∪ 8 heaviest candidates)
                       ∪ topK_entropy_UB(K) )                  // v1: K = 0; K = 16–32 after validation
  for a in {HINT} ∪ A_root:
    enumerate outcomes exactly (histogram)
    solved -> r + F̂(k, h, c−1)
    else   -> r + leaf_eval(child)
leaf_eval(x):                                                  // depth-1 node
  best = 0 (STOP)
  HINT:   Σ_f Σ_v p · vhat(H(C_fv), h−1, c, M) − e_A           // 6 partitions
  each a ∈ A_leaf, skipping no-ops:
          Σ_o p · (r + (solved ? F̂ : vhat(H(C_o), h, c−1, M|m)))
          // one pass per action: accumulate w and w·log2 w per outcome code
```
- **Cost:** O(|A_root|·|A_leaf|·n) partition work, about 800·n pairs for 26 actions.
- **Measured, native, one thread** (`adp-time`):
  - FV2 at n = 5000: 22.3 ms.
  - FV2 at n ≈ 2200: 10.3–12.5 ms.
  - FV2 at n ≤ 600: 3 ms or less.
  - Endgames: under 0.5 ms.
  - Legacy L2 costs 3.1 ms at n = 5000.
- **WASM estimate** (1.3–2× native): 29–45 ms worst case. That meets p50 ≤ 50 ms and p99 ≤ 250 ms with room to spare.
- **Estimated effect of a larger root set:** grow only the root and keep A_leaf at 26. Cost scales with |A_root|, so K = 32 gives about 2.2×, roughly 50 ms native worst case. Only run it when n < 2500, or behind the anytime budget.

### 2.5 Depth 3, outcome truncation, rollout, MCTS
- **Depth 3 with the learned leaf** (FV3): 299 ms native at n = 5000 and 105–150 ms at n ≈ 2000. It was **not better** than depth 2 (−0.033 ± 0.024 E[S] over 1000 activities). The legacy zero leaf at depth 3 is actually worse than at depth 2: 58.70% against 65.25% all-9 at 9/11/28. With a wrong leaf, extra depth hurts; with a good leaf, it adds nothing. v1 ships depth 2. Depth 3 stays an option for n ≤ 600 (about 34 ms native) once a better leaf exists.
- **Deterministic outcome truncation** (for any depth-3 use): expand only children with p ≥ τ (for example 0.02) and give the rest V̂ directly. This keeps results reproducible, unlike random sparse sampling. Not measured.
- **Bertsekas rollout** with FV2 as the base policy: for the top m = 8 root actions, run M = 32 CRN rollouts to the end of the puzzle (targets drawn from the posterior on C) and score the terminal state with F̂. Estimated cost 8·32·~4 decisions·~8 ms ≈ 8 s native, so it is **offline only**. Two uses:
  - as a teacher that labels root decisions for diagnosing where V̂ is miscalibrated;
  - as an optional "deep analysis" button that runs rollouts over several seconds in a Worker, with a progress bar.
- **MCTS: rejected.** Outcome distributions here are exact and cheap: O(n) per action, at most 57 cells. MCTS would sample what the expectimax already enumerates exactly. Its variance at 50 ms budgets is poor, and a deterministic depth-2 search with a calibrated leaf dominated in every test. The one idea worth keeping is progressive widening of the root set, which the top-K entropy step already covers.

### 2.6 Exact tiers
1. **Final puzzle (k = 0):** exact solver with resource caps c_eff and h_eff. Measured ≤ 5 ms for n ≤ 7 and ≤ 350 ms for n ≤ 13, at any resources. This is the provably optimal case.
2. **Non-final puzzle, n ≤ 7:** exact within the puzzle, using F̂ as the continuation at each solve. The caps are invalid here because the continuation depends on (h, c). Measured cost without caps: 48 ms at n = 7, h11 c36 (report_algo v1). Gate on estimated cost, and fall back to FV2 when the anytime budget runs out.

---

## 3. What is proven and what is approximate; UI labels

| situation | guarantee | UI label |
|---|---|---|
| final puzzle, exact solve completes | optimal under the model and RulePolicy | 已证明最优 |
| non-final puzzle, exact within the puzzle | optimal for this puzzle, given the approximate continuation | 本题精确（后续价值为模拟估计） |
| FV2 / FV3 | approximate: learned value at depth 2 | 近似最优（学习估值·深度2） |
| budget exhausted before depth 2 | depth-1 greedy using V̂ | 快速近似 |

**Exact** (bit-identical to reference solvers): kernels, dedup, no-op pruning, the entropy bound, and F̂ ≤ min(k, c).

**Not guaranteed:** F̂ and V̂ are Monte-Carlo estimates of one policy's value, not bounds. The legacy "当前可行下界" (feasible lower bound) label is dropped.

**Forecasts the UI can show, both labelled 模拟估计:**
- "本题预计还需 E[J] 次挑战 / E[E] 次提示，P(本题可解) = PS", from the leaf row.
- "本活动预计可解 X 题", from F̂.

**Ties:** actions within 1e-9 on all three components are shown as 任选其一 (pick any one), with a deterministic tie-break on the lowest group id.

---

## 4. Offline pipeline (native, deterministic, `mdd fit`)

1. **Leaf data.** Simulate N activities under policy π_i. At every decision, log (b, flag). When the puzzle is solved, log the realized (J, E) from that point on. Failed puzzles are dropped as censored. Pool sparse bins: first with the other flag, then with neighbouring bins, down to at least 40 samples.
   - Measured: 4000 activities give 150k samples in 13.6 s wall under L, and 41 s under FV.
   - **Lesson from the policy-iteration round:** fit on a mixture of presets (9/11/36, 9/11/28, 9/4/30, 9/0/36). A single preset under a hint-rich policy leaves almost no no-hint states at high entropy (2–6 samples per bin).
2. **Table F̂.** For k = 1..K, for every (h, c) with c ≤ min(cmax, 5k+6), run n = 64 fresh puzzles with CRN across cells, using the policy that consults F̂[<k]. Then set F̂[k][h][c] = mean(1{solved}·(1 + F̂[k−1][h'][c'])), and do the same for M. Apply the monotone repair and the clamps.
   - Measured: 71.1 s wall (1078 CPU-s, 20 threads) for the 9×12×37 grid with the depth-2 zero-leaf policy.
   - Estimated: 3–4× that under the FV policy. For a grid that covers custom presets (k ≤ 15, h ≤ 20, c ≤ 60, about 14.6k cells), about 6.5 min with zero leaf and about 20 min with FV.
3. **Approximate policy iteration.** π_{i+1} = FV(F̂_i, leaf_i). Stop when a 10k-activity CRN evaluation on held-out seeds shows no significant change (expect 1–2 rounds; round 1 showed none, see section 6).
4. **Reproducibility.** Seeds come from `mix(seed, activity, puzzle, stream)`. Results are gathered in index order, so tables are identical at any thread count. Fit seeds (777, 4242) are disjoint from evaluation seeds (99).

**What ships** (`include_bytes!`, versioned header):

| table | contents | size |
|---|---|---|
| F̂ (default grid) | 3996 cells × (S, M) as u16 | 16 KB |
| F̂ (general grid) | 20.5k cells | about 82 KB, roughly 30 KB after brotli (estimate) |
| CostToGo | 28 × 2 × 16 × 7 u16 | 12.5 KB |

The header carries `data_hash`, `rules_hash`, `objective_id`, the grid and the fit seed.
- If the rules, pool or objective hash differs from the current state, the engine falls back to FV with a renewal-approximated F̂ (below) and shows 估值表与当前设置不匹配.
- Outside the grid, values are clamped and labelled 外推 (extrapolated).
- Refit in CI whenever the card data changes, in about 10 min.

**Later: renewal kernel.** Instead of F̂, ship the single-puzzle consumption kernel Q(solved, Δh, Δc | h-bucket, c-bucket). Then build F̂ for any custom preset in the browser: F[k] = Q ⊛ (1 + F[k−1]). That is about 20k cells × ~40 support points, around 1 ms (estimate).

---

## 5. Validation plan

- **CRN.** The target and initial field for each (activity, puzzle) come from stream 0, and hint-field draws from stream 1, so every policy faces the same targets. Report paired differences in S and in 1{all solved}, with 95% intervals (1.96·sd/√n) and the number of activities that differ.
- **Sample sizes.** 10k activities at the default preset: the paired SE on P(all) is about 0.12 pts. 2k–3k is enough for stress presets, where effects are 5–20 pts.
- **Presets:**
  - default 9/11/36;
  - stress 9/11/28, 9/4/30, 9/0/36, 9/11/24, 12/14/45;
  - a custom preset that is **not** in the fit mixture, to test for overfitting;
  - rule variants R3, R5 and R15 (the table fitted under default rules, evaluated on sims with alternative rules, and refitted per rule).
- **Calibration.**
  - Leaf: predicted P(J ≤ j | φ) against realized, on held-out trajectories (Brier score per bin).
  - F̂: prediction at each puzzle start against realized future solves, residual per (k, c-band). Target |bias| < 0.02 solves.
- **Acceptance for v1:**
  - FV ≥ L on every preset, with the lower 95% bound ≥ 0 for E[S];
  - on the default preset, P(all) at least as good as L with a 0.3-pt non-inferiority margin (achieved +1.33 ± 0.24);
  - WASM latency p99 ≤ 250 ms on the 78 exported states plus 1000 sampled decisions;
  - native and WASM choose the same action on fixtures (libm-only math).
- **Baselines:** L (legacy, depth 2), L3 (legacy, depth 3 uncapped), and a Wordle-style greedy policy that maximizes entropy.

---

## 6. Experiments run

**Setup.** A copy of the prototype lives at `/tmp/claude-1000/-home-parry-gitRepos-md-card-decoder/cb7e6cca-357d-4211-9aca-7cbb5c5749b0/scratchpad/panel-approx/rs`. I added `src/adp.rs`, and `src/horizon.rs` gained a pluggable continuation plus `leaf_eval`.
- Build with `cargo build --release`. Run everything from that `rs/` directory with the `adp-*` modes of `./target/release/mdbench`, 20 threads.
- The policy L is a native port of the legacy restricted horizon: the 18/8/8 action set, legacy continuation, zero leaf, no exact tier.

**Policies:**
- **F** = fitted F̂, zero leaf;
- **LV** = learned leaf composed with the legacy formula;
- **FV** = F̂ plus the learned leaf;
- **FVi** = FV with the leaf refitted under FV;
- the digit is the search depth (default 2).

**Commands:**
```
adp-leaf 4000 9/11/36 2 ../leaf_default.txt                  # 150,056 samples, 13.6 s
adp-fit 64 2 ../F_d2.txt                                     # 71.1 s wall
adp-eval 10000 9/11/36 2 ../F_d2.txt ../leaf_default.txt L F FV
adp-eval 2000 9/11/28 2 ../F_d2.txt ../leaf_default.txt L F LV FV
adp-eval 2000 9/11/28 2 ../F_d2.txt ../leaf_default.txt L3 FV L
adp-eval 3000 9/4/30 2 ../F_d2.txt ../leaf_default.txt L F FV
adp-time ../F_d2.txt ../leaf_default.txt
adp-leaf 4000 9/11/36 2 ../leaf_fv.txt ../F_d2.txt ../leaf_default.txt
LEAF2=../leaf_fv.txt adp-eval 2000 9/11/28 2 ... FV FV2      # (same for 3000 9/4/30)
adp-eval 1000 9/11/28 2 ../F_d2.txt ../leaf_default.txt FV FV3
```

**Results** (E[S], P(all), hints/challenges used; paired against the first policy listed in each block):

| preset | policy | E[S] | P(all) | hints / chal | paired Δ vs first (95%) |
|---|---|---:|---:|---|---|
| 9/11/36, n=10k | L | 8.9846 | 98.51% | 8.85 / 28.80 | – |
| | F | 8.9835 | 98.37% | 8.55 / 29.23 | ΔS −0.0011 ± 0.0023; ΔP −0.14 ± 0.22 |
| | **FV** | **8.9984** | **99.84%** | 8.09 / 28.60 | **ΔS +0.0138 ± 0.0025; ΔP +1.33 ± 0.24** |
| 9/11/28, n=2k | L | 8.578 | 65.25% | 9.66 / 27.02 | – |
| | F | 8.594 | 65.65% | 9.44 / 27.15 | ΔS +0.016 ± 0.018 |
| | LV | 8.484 | 60.40% | 10.82 / 26.19 | ΔS −0.095 ± 0.034 (worse) |
| | **FV** | **8.835** | **84.90%** | 9.88 / 26.02 | **ΔS +0.257 ± 0.026; ΔP +19.65 ± 2.08** |
| | L3 | 8.505 | 58.70% | 9.66 / 27.66 | FV−L3: ΔP +26.2 ± 2.2; L−L3: +6.55 ± 2.2 |
| 9/4/30, n=3k | L | 7.352 | 14.37% | 3.99 / 29.78 | – |
| | F | 7.389 | 14.63% | | ΔS +0.037 ± 0.017 |
| | **FV** | **7.739** | **23.17%** | 3.90 / 29.75 | **ΔS +0.387 ± 0.033; ΔP +8.80 ± 1.48** |
| policy iteration | FVi vs FV, 9/11/28 | | | | ΔS −0.0075 ± 0.015 (none) |
| | FVi vs FV, 9/4/30 | | | | ΔS −0.033 ± 0.023 (leaf coverage, see risks) |
| depth | FV3 vs FV, 9/11/28, n=1k | | | | ΔS −0.033 ± 0.024; 10× the cost |

**Throughput** (20 threads, ms per activity per thread): L 55–103, FV 176–348, FV3 1784.

**Latency** (single thread, per decision, `adp-time` with h=11, c=30):

| n | L2 | L3 | FV2 | FV3 |
|---|---:|---:|---:|---:|
| 5000 | 3.1 ms | 72.9 ms | 22.3 ms | 299 ms |
| 2179–2258 | – | – | 10.3–12.5 ms | 108–152 ms |
| ≤ 600 | – | – | ≤ 3.1 ms | ≤ 34 ms |

FV2 picks the same action as L2 on roughly half of the 78 states.

**Takeaways:**
1. The leaf is what gains, but only when composed with a calibrated continuation. The legacy ratio (1 hint = 0.61 challenges) makes the leaf overspend hints: LV uses 10.8 hints against L's 9.7.
2. Extra depth with a bad leaf hurts (L3 < L2).
3. Most of the default-preset headroom is already recovered: residual failures are 0.16%.

---

## 7. Risks

- **Model misspecification** (R3/R5/R7/R8/R15, the target prior, whether daily grants accumulate) biases both tables. Mitigation: tables fitted per `RulePolicy`, a hash check with an explicit fallback, and a log-replay consistency check on real play.
- **Leaf state coverage.** The failed policy-iteration round showed that data from a hint-rich policy leaves no-hint bins empty (2–6 samples), and pooling then contaminates them. Mitigation: mixed-preset fitting, ε-exploration in hint use while logging, and minimum-count checks in CI.
- **Policy mismatch.** F̂ was fitted under the zero-leaf policy but is consumed by FV. It worked, but convergence of approximate policy iteration under Monte-Carlo noise is not guaranteed. Freeze a version only after the held-out CRN evaluation.
- **Features too coarse** (entropy plus hint flag). They ignore top mass, the number of unknown fields (which limits hints) and matched fields. Candidate v2 features, accepted only if calibration improves and CRN shows a gain.
- **Stale tables** when the Master Duel pool changes. Entropy bins transfer across pools; F̂ needs a CI refit of about 10 min, tied to the data hash.
- **Objective drift.** If the user wants reward points or P(all 9), the tables must be refitted for that objective; they are not interchangeable.
- **Changed advice.** FV disagrees with legacy on about 50% of decisions (for example it holds hints longer). Users need release notes and the simulation evidence.
- **Simulation fidelity.** Neither simulated policy includes the exact final-puzzle tier, and the real app mixes depth 3 and depth 2. Absolute rates may shift a little; the paired comparisons stand.
- **WASM latency** is an estimate; no wasm32 build exists yet. Measure early (M0).

---

## 8. v1 and later

**v1:**
- the exact kernels;
- FV2 with the legacy root set (optionally plus top-16 entropy, if CRN confirms), depth 2, and an anytime fallback to depth-1 greedy using V̂;
- the exact final-puzzle tier (proven label);
- F̂ for the default grid plus a general grid (k ≤ 15, h ≤ 20, c ≤ 60) with clamping;
- the CostToGo leaf fitted on a mixture of presets;
- `mdd fit` / `mdd eval` natively with CRN and a 10k-activity gate;
- the labels and forecasts from section 3;
- about 30–100 KB of shipped tables.

**Later:**
- one or two more policy-iteration rounds with ε-exploration;
- richer leaf features (top mass, unknown count, n) or a tiny GBDT, calibrated;
- the renewal kernel, so any custom preset gets an exact F̂ in the browser;
- exact-within-puzzle with F̂ for n ≤ 7 on non-final puzzles;
- an offline rollout teacher plus a "deep analysis" button;
- top-K root expansion within the anytime budget;
- outcome-truncated depth 3, only if a better leaf makes depth pay off;
- alternative objectives (points, P(all)) as separately fitted table sets.