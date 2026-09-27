# Reader report: algo

## Summary
I benchmarked the current JS solver on fixed-seed states: 12 openings plus 3 worst-case openings (up to 5000 candidates), 24 mid-game states and 39 endgames. I then built a native Rust prototype (cargo project in the scratchpad), and all 249 of its value vectors match the JS solver's: 93 exact cases and 156 depth-limited cases. The JS solver has three blowups. First, `solveExact` keys its memo on the set of already-guessed cards, so every run with c≥4 hits the 160k-state limit, even with only 2 candidates, after 7–20 s of main-thread time. Second, depth-3 `solveRestrictedHorizon` always hits its 24k-state limit at openings with ≥290 candidates (0.7–2.2 s wasted) and falls back to depth 2. Third, the first pass of `calculateRecommendations` costs O(8023·|C|) (0.25–0.53 s per call at openings), although the same numbers come from per-field histograms at O(|C|+8023) in 0.4 ms. With four exact changes, the prototype runs the recommendation stage at the worst opening in about 75 ms native, versus about 3.2 s in JS, and one simulated activity in 0.33 s, versus 20.7 s in JS. The changes are: drop the guessed set from memo keys and prune no-op guesses; merge equivalent guesses using an O(1)-per-guess signature; evaluate the last search level in O(1) from histograms; and use u8 per-field columns with a factorized outcome code. A tight entropy upper bound picks the exact top-64 most informative guesses with 84–206 full partitions instead of 8023. Better action sets chosen this way raise depth-3 solve value much more than extra depth does.

## Findings
- **solveExact state explosion is caused by `guessed` in the memo key (fixable with zero value change)**: The JS exact solver builds keys from the guessed set, so the same (candidates, masks, resources) state reached by different paths is re-expanded. Branching is roughly C(#actions, c-1). Every exact run with c>=4 hits EXACT_STATE_LIMIT (160k), even with 2 candidates, averaging 11.2-11.4 s of main-thread time. Dropping `guessed` and pruning no-op guesses fixes this: a no-op is a single-cell outcome with no new matched field and no solve, and it is strictly dominated because value is monotone in challenges and the -E[A] term breaks ties. Rust with this change reproduces all 93 JS-completed values exactly and solves the 56 JS-LIMIT cases in <=5 ms each (<=2094 states). [exact.log vs exact-rs.log (ab-cmp.js): same=93 diff=0 jsLimit=56; JS 739 s total vs Rust 51.7 ms; JS/Rust expanded-state ratio median 12x, max 887x; e.g. n=2 h=0 c=3: JS 5086 states/221 ms vs Rust 7 states]
- **Depth-3 restricted horizon never completes at real openings**: With the app's ~23-28-action set, depth 3 needs about 26k states at n=290, 30k at n=528, 40k at n=873, 58-61k at n=2179-2258 and 65k at n=5000. All exceed the 24k cap, so the app wastes 0.7-2.2 s, then silently uses depth 2 while the UI text still talks about depth 3. Mid-game and endgame states always complete (<=11k states, <=340 ms). [horizon.log; hexport-big.log (JS uncapped depth 3: n=5000 7.2 s / 65,411 states; n=2258 3.9 s / 58,431)]
- **First pass of calculateRecommendations is O(8023*|C|) but can be O(|C|+8023)**: P(solve), E[new matches] and infoApprox are linear in per-field match probabilities. They come from per-field weighted histograms plus static full-match lists (each guess fully matches at most 3 groups; 8149 pairs in total). Rust measures 0.36-0.50 ms for all 8023 guesses, versus 7-586 ms for the JS loop, with identical numbers. [ab-part.js quickLoop column (529 ms at n=5000, 249-261 ms at n~2200); rs `mdbench kernels` quickO1_ms 0.13-0.50]
- **Exact action-equivalence classes via an O(1)-per-guess projected signature**: Two guesses give identical outcomes on every candidate exactly when their per-field projections match: frame gives the set of compatible target frame-ids present, level gives compatible nm-ids present, and the equality fields give the value id if present, else MISS. The projected count equals the brute-force signature count in all 78 states. The effective variant drops already-matched fields and adds the guess's full-match set within C. It equals the brute-force effective count, e.g. endgame n=2 75->5, n=10 382->119, n=15 1355->1101. Openings barely merge (4713-7903 of 8023 guesses stay distinct); endgames merge heavily (median 537 full -> 220 effective). [ab-part.js (fullSigClasses == projSig in every row); ab-dom.js effClasses == Rust proj_classes(effective=true)]
- **Leaf-level O(1) evaluation plus dedup plus no-op pruning makes depth 3 cheap**: At depth-1 nodes every child is worth 0, so a challenge's value is [Ps*(1+contS), E[gain]+Ps*contM, -1], and a hint (worth [0,0,-1]) can never beat stopping. Both come from per-field histograms without partitioning. The faithful Rust port is 22x faster than JS; with these optimizations it is 116x faster on 156 cases and 100x on the large-opening set. All values are identical. The worst opening (n=5000, 26 actions) at depth 3 takes 72 ms, versus 7.2 s in JS. [rs `mdbench horizon ../hcases.txt`: TOTAL jsMs 11484, rsV0Ms 528.7, rsOptMs 98.6, mismatches 0; hcases-open: jsMs 48682 / v0 3034 / opt 487]
- **Root action-set quality matters more than depth**: Adding the top-K guesses by exact partition entropy to the app's 26-action root set raises depth-3 value substantially. At n=2258, E[S] goes 0.296 -> 0.368 -> 0.487 -> 0.611 for 26/46/76/140 actions, at a cost of 39/92/229/724 ms native. Depth 4 with 25 actions (n=873) gains less, 0.433 -> 0.457. [rs `mdbench hscale`]
- **Tight entropy upper bound makes exact top-K informative-guess selection nearly free**: By subadditivity, H(outcome) <= sum_f H(response_f), and each per-field response entropy is O(1) per guess from histograms. Frame and level responses are multi-valued (frame reveals frame bits, level reveals printed n). The bound held for every guess in every state (asserted). The exact top-64 by entropy needs only 84-206 full partitions: about 0.35 ms for the bounds plus about 1 ms of partitions at n=5000, versus 102 ms for all guesses. [rs `mdbench ubent`]
- **Level semantics trap: match uses the nm bitmask, hint/reveal uses the printed n**: My first Rust hint branch grouped the level field by nm-id and diverged from JS on 2/156 cases. Rank-0/1 Xyz groups with nm=3 and nm=2 both show n=1, and 7 groups share (b,a,r,n,atk,def) except nm. Fixed with a separate hint column. This must become a golden test in the rewrite. [horizon mismatch E[M] 11.4734 vs 11.4743 (1 weight unit of 1032) for opening n=528 depth 3; resolved by pool.hint_col]
- **The suggested E[S] bound using top-(c*kmax) masses is NOT admissible**: With c>=2 challenges, an adaptive policy can solve up to kmax*(1+K+K^2+...) targets, where K is outcomes per guess, not c*kmax. Valid per-puzzle bounds: when c=1 and h=0, the max single-guess solve mass; otherwise 1, or the counting bound. With only these trivial bounds, lexicographic B&B pruned actions but did not reduce expanded states. Real gains need Wordle-style lower bounds on E[actions]. [exact.rs ub() comment; exact-big.log bnb columns (same expanded counts)]
- **Exact resource caps for the final puzzle (zero continuation)**: c_eff = min(c, |S| + 6 - |matched|): every challenge in a no-op-free policy either splits the set, solves, or adds a matched field. h_eff = min(h, #unknown fields). Combined with inheriting parent action lists, large-resource exact solves speed up 6-30x with identical values. For example n=12 h11 c36 goes 929 -> 78 ms; n=15 h11 c36 goes 14.8 s -> 1.1 s. Remaining hard band: n~15 with h4 c6 still takes 2.4-4 s. [exact2.log (agree=true on all rows; v1ms vs v3ms)]
- **Simulation baseline and native port**: The real JS worker policy takes 20.7 s per 9-puzzle activity (160 solver calls, ~0.52 s each), so the UI maximum of 500 activities would take about 3 h. A native Rust port of the same policy structure (quick metrics -> action set -> uncapped depth-3 horizon) takes 0.33 s per activity on one thread, or 0.036 s at depth 2. With 20 threads, 1000 activities at depth 2 take 3.7 s wall and 400 at depth 3 take 12.8 s. Mean solved is about 8.98/9 with all-9 at 97.5-98.1%. That is near saturation, so telling policies apart needs about 10k activities, which argues for native rayon runs. [ab-sim.js output; rs `mdbench sim 1000 20 2`, `mdbench sim 400 20 3`]
- **Main-thread blocking in the current UI**: tryExactBellman and tryRestrictedPolicy run synchronously on the main thread after the chunked loops. The UI can freeze up to about 3 s at openings and 7-20 s on final-puzzle exact attempts that hit the limit. Separately, `await nextFrame()` is setTimeout(0); its roughly 111 yields per recommendation add about 0.45 s once browsers clamp nested timers to 4 ms. The WASM solver should run in a Worker. [dist/app.js:869,913,934-936,1221-1223]

## Spec
# Algorithm benchmark and optimization research: Card Decoder rewrite in Rust/WASM

Environment: i7-12700K (20 threads) under WSL2, Node v26.9.0, rustc 1.98.1 (`--release`, `lto`, `codegen-units=1`). No wasm32 target was installed, so all WASM figures are estimates.
Scratch code (none of it touches the repo) lives in `/tmp/claude-1000/-home-parry-gitRepos-md-card-decoder/cb7e6cca-357d-4211-9aca-7cbb5c5749b0/scratchpad/algo-bench/`:
- JS harness: `ab-load.js`, `ab-core.js`, `ab-gen.js` → `states.json`; `ab-part.js`, `ab-dom.js`, `ab-exact.js` → `exact.log`; `ab-horizon.js` → `horizon.log`; `ab-hexport.js` → `hcases*.txt`; `ab-sim.js`, `ab-export.js` → `data.txt`; `ab-cmp.js`.
- Rust prototype in `rs/`: `pool.rs`, `kernels.rs`, `exact.rs`, `exact2.rs`, `horizon.rs`, `sim.rs`. Modes: `kernels | exact | exact-big | exact2 | horizon | hscale | sim | ubent`.

States (fixed seeds, mulberry32):
- **opening**: weighted random target, one random field revealed. 12 states, n = 69…2258.
- **worst**: frame = effect (n = 5000), level = 4 (2258), attribute = DARK (2179).
- **mid1 / mid2**: after 1 or 2 entropy-best challenges. n = 7…585 and 1…286.
- **end**: played until n ≤ 30. 39 states, n = 2…30.

Weights are `wm`; the universe is 8023 groups.

---
## 1. Baseline measurements (current JS)

### 1a. Feedback partition of all 8023 guesses over the candidate set C (ms per full pass)

| state | n | JS string-key Map (solver style) | JS numeric-key Map (finalists) | JS quick loop (app pass 1) | JS typed arrays + histogram | **Rust factorized code + histogram** | Rust ns/pair | **Rust quick metrics O(\|C\|+U)** | avg / max outcomes per guess |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| worst (frame = effect) | 5000 | 3053 | 767 | 529 | 244 | **87.0** | 2.17 | **0.50** | 23.7 / 32 |
| opening | 2258 | 1378 | 344 | 261 | 115 | 44.7 | 2.47 | 0.41 | 23.8 / 49 |
| opening | 2179 | 1337 | 354 | 249 | 84 | 38.9 | 2.23 | 0.39 | 25.2 / 57 |
| opening | 873 | 559 | 152 | 100 | 34 | 16.7 | 2.38 | 0.41 | 19.1 / 35 |
| opening | 528 | 373–516 | 101–118 | 70 | 28 | 10.8 | 2.55 | 0.39 | 19.5 / 42 |
| opening | 290 | 209 | 60 | 38 | 17 | 6.6 | 2.82 | 0.43 | 16.5 / 35 |
| opening | 69 | 60 | 13 | 7 | 4.0 | 2.0 | 3.56 | 0.36 | 8.8 / 20 |
| mid1 | 585 | 324 | 82 | 58 | 22 | 10.6 | 2.25 | 0.36 | 7.6 / 16 |
| mid1 | 111 | 68 | 16 | 11 | 5.3 | 3.9 | 3.96 | 0.31 | 4.4 / 8 |
| end | 30 | 19 | 4 | 3 | 1.5 | 1.35 | 5.6 | 0.29 | 3.4 / 7 |
| end | 7 | 5 | 1 | 1 | 0.5 | 0.24 | 4.2 | 0.15 | 1.5 / 5 |

The string-map and numeric-map columns are extrapolated from 1/16 of the guesses.

Current app pipeline per recommendation at the worst opening: quick loop 586 ms + depth-3 attempt that hits the limit 2157 ms + depth-2 481 ms + 300 finalist partitions (~30 ms) + about 111 `setTimeout(0)` yields. That is **about 3.2 s**, mostly on the main thread. At n ≈ 2200 it is about 1.9 s; at n = 873, 1.1 s; at n = 69, 0.44 s.

### 1b. `solveExact` (the app's gate: final puzzle, n ≤ 7, h ≤ 4, c ≤ 6), 14 states with n = 2…5

| (h, c) | JS ms (completed) | JS states | JS runs hitting the 160k limit | avg ms wasted per limit hit | **Rust v1 max ms** | **Rust max states** |
|---|---:|---:|---:|---:|---:|---:|
| 0,1 | 0–1 | 1 | 0/14 | – | 0.01 | 1 |
| 0,2 | 6–91 | 119–586 | 0/14 | – | 0.17 | 64 |
| 1,1 | 0–2 | 3–12 | 0/14 | – | 0.05 | 12 |
| 1,2 | 12–405 | 269–3160 | 0/14 | – | 0.38 | 197 |
| 0,3 | 221–11639 | 5k–89k | 0/14 | – | 0.33 | 149 |
| 2,2 | 12–730 | 269–5834 | 0/14 | – | 0.68 | 382 |
| 2,3 | 472–13575 | 10.6k–153k | 4/13 | 15.5 s | 1.25 | 764 |
| 1,4 | – | – | **13/13** | 11.4 s | 1.18 | 663 |
| 4,4 | – | – | **13/13** | 11.2 s | 2.85 | 1656 |
| 4,6 | – | – | **13/13** | 11.4 s | 4.59 | 2094 |
| 3,6 | – | – | **13/13** | 11.4 s | 5.03 | 1938 |

Cause: the JS memo key includes the guessed set. For example, n = 2 with 75 representative actions at c = 4 explodes (JS 5086 states at c = 3; Rust 7). In practice the JS exact mode only works when c ≤ 3.

Rust exact on larger n. v1 = memo without `guessed` + dedup + no-op pruning; v3 = v1 + inherited action lists + resource caps:

| n (effective actions) | h, c = 0, 3 | 4, 6 | 11, 36 (v1 → v3) |
|---|---:|---:|---:|
| 7 (252) | 3.8 ms | 36 → 5 ms | 48 → 1.7 ms |
| 10 (223) | 16 ms | 105 → 53 ms | 160 → 17 ms |
| 12 (434) | 99 ms | 577 → 296 ms | 929 → 78 ms |
| 13 (279) | 72 ms | 412 → 273 ms | 648 → 74 ms |
| 15 (1101) | 1.08 s | 7.7 → 4.1 s | 14.8 → 1.1 s |
| 20 (1623) | 8.6 s | 175 s (v1) | – |

### 1c. `solveRestrictedHorizon` with app-like action sets

The action set is the top 18 by (solve, newMatches), plus the top 8 by infoApprox, plus the top 8 candidates by weight: 18–28 actions.

| state | n | depth-3 states needed (JS, uncapped) | JS depth 3 uncapped | app behaviour (cap 24k) | JS depth 2 | Rust port depth 3 | **Rust optimized depth 3** | Rust optimized depth 2 |
|---|---:|---:|---:|---|---:|---:|---:|---:|
| worst | 5000 | 65,411 | 7.2 s | limit after 2.16 s → depth 2 | 481 ms | 534 ms | **72 ms** | 2.5 ms |
| opening | 2258 | 58,431 | 3.9 s | limit after 1.40 s | 243 ms | 261 ms | **40 ms** | 1.5 ms |
| opening | 2179 | 60,947 | 3.4 s | limit after 1.16 s | 205 ms | 185 ms | 39 ms | 1.6 ms |
| opening | 873 | 39,511 | 1.6 s | limit after 0.86 s | 101 ms | 114 ms | 17 ms | 0.8 ms |
| opening | 528 | 29,662 | 0.90 s | limit after 0.76 s | 61 ms | 32 ms | 9 ms | 0.6 ms |
| opening | 290 | 26,285 | 0.75 s | limit after 0.73 s | 45 ms | 33 ms | 10 ms | 0.5 ms |
| opening | 69 | 15,785 | 0.43 s | completes | 22 ms | 11 ms | 3 ms | 0.4 ms |
| mid1 | 136 | 11,088 | 0.34 s | completes | 18 ms | 9.6 ms | 2.5 ms | 0.18 ms |
| mid2 | 47 | 7,383 | 0.20 s | completes | 10 ms | 4.3 ms | 1.7 ms | 0.14 ms |
| end | 15 | 4,005 | 72 ms | completes | 4 ms | 1.0 ms | 0.5 ms | 0.07 ms |

HORIZON_STATE_LIMIT is hit at depth 3 for every opening with n ≥ 290, and never for mid-game or endgame states. The "Rust port" column reproduces JS semantics with `guessed` dropped from the key. "Rust optimized" adds dedup, no-op pruning and the O(1) leaf evaluation. **All 156 + 42 value vectors match JS within 1e-9.**

### 1d. Whole-activity simulation (9 puzzles, 11 hints, 36 challenges)

- **JS worker policy:** 20.7 s per activity (4 activities: 160 solver calls, 125 at depth 3, 34 at depth 2, 1 exact).
- **Rust port of the same policy structure, depth-3 horizon uncapped:**
  - 1 thread: 0.33 s per activity at depth 3, 0.036 s at depth 2.
  - 20 threads: 1000 activities at depth 2 in 3.7 s wall; 400 at depth 3 in 12.8 s wall.
  - Mean solved about 8.98/9; all 9 solved in 97.5–98.1% of activities; hints 6.7–8.8; challenges 28.9–31.8.
  - The objective is near saturation, so separating policies needs 10k or more activities.

---
## 2. Distinct actions per state (measured)

Column definitions:
- **full**: distinct per-candidate outcome signatures. This equals the projected-signature count in all 78 states.
- **effective**: labels restricted to `mask & ~matched` plus a solved flag. This is exact: the same rewards and child states.
- **pareto**: the same partition with Pareto-maximal labels.
- **refine**: refinement dominance.
- **no-op**: guesses that are single-cell with a zero label.

| state | n | full | effective | same-partition Pareto | refinement Pareto | no-op guesses (of 8023) |
|---|---:|---:|---:|---:|---:|---:|
| openings (12) | 69–2258 | 4713–7631 (median ≈ 7400) | ≈ full | ≈ full | – | 0 |
| worst | 5000 | 7903 | 7903 | – | – | 0 |
| mid1 | 7 / 8 / 11 / 24 | 386 / 292 / 584 / 918 | 252 / 48 / 151 / 251 | 187 / 37 / 64 / 136 | 101 / 28 / 54 / 113 | 2149 / 1556 / 2559 / 148 |
| mid1 | 50 / 111 / 452 / 585 | 4211 / 3354 / 5869 / 7184 | 3429 / 2401 / 5251 / 6808 | 3230 / 1606 / 4899 / 6380 | 1763 / – | 93 / 77 / 40 / 7 |
| mid2 | 4 / 9 / 47 / 286 | 116 / 201 / 3700 / 6472 | 21 / 19 / 2713 / 5521 | 13 / 9 / 2644 / 5467 | 10 / 9 / 1322 / – | 6924 / 5440 / 174 / 38 |
| end, n ≤ 7 | 2–7 | 75–537 | 5–227 | 2–133 | 2–90 | 2400–7300 |
| end (39 states) | 2–30 | median 537 (75–3498) | median 220 (5–2766) | median 133 | median 90 | median ≈ 2800 |

Takeaways:
- Dedup is worth little at openings (≤ 1.7x).
- Effective dedup plus no-op pruning is worth about 2.5x median in endgames, and up to 15x (n = 2: 75 → 5).
- Refinement dominance gains another 1.2–2x, but carries proof risk.

---
## 3. Optimization research (concrete to this game)

1. **u8 SoA value-id columns and a factorized outcome code (implemented).**
   - Distinct values per field: frame 13, attribute 7, race 25, nm 15, ATK 83, DEF 77.
   - The code is `T_b[g][t]·240 + (a==)·120 + (r==)·60 + T_n[g][t]·4 + (atk==)·2 + (def==)`. The frame code is 1 + the target's frame id; the level code is 1 + the printed n. That gives 3360 codes, which fit a u16.
   - This is a factorized 8023² feedback matrix: tables of ≤ 16×16 plus equality tests, about 56 KB of data in total, versus 128 MB for a full u16 matrix.
   - Speed: 2.2 ns per pair native, which is 2.8x typed-array JS and 35x the solver's string-key Map. Partitioning uses a histogram with a touched-list (counting sort for child lists).
   - SIMD headroom is at most about 2x (pshufb / `i8x16.swizzle` for the 16-entry frame and level tables, interleaved sub-histograms). Low priority, because this kernel is no longer the bottleneck.
2. **Bitsets.**
   - Exact states with n ≤ 64 use a u64 over root-local indices (implemented). The key `(set, known, matched, h, c)` fits a u128 with no collisions.
   - For large sets, use 126-word universe bitsets for membership, hashing and hint partitions (AND with per-field value bitsets).
   - Partition by AND-tree with weighted popcount is estimated at parity with the lookup-table histogram at n ≥ 2000 and worse below. Weights (max 21; 7321 of 8023 groups have weight 1) would need 5 bit-planes. **Recommendation:** lookup-table histogram for partitions, bitsets for identity and membership.
3. **Action equivalence via projected signatures (implemented, exact).**
   - O(1) per guess from presence masks, plus a sort: 0.12–0.20 ms native for all 8023 guesses.
   - The effective variant drops fields already in `matched`, and adds the guess's full-match set ∩ C.
   - Per-node dedup uses a 128-bit rolling hash of (canonical cell id, effective label) per candidate.
   - Child nodes inherit the parent's representative list, because equivalence only coarsens on subsets.
4. **Dominance pruning.** Same-partition Pareto and refinement dominance (finer partition with labels ⊇) cut the effective count by a further 1.2–2x in endgames. Correctness needs a Blackwell argument, plus a coupling argument for the knownMask effect on hint distribution. Medium–high risk: keep it behind a flag and check against brute force on random small pools.
5. **Drop `guessed` from keys (implemented, verified).**
   - Every previously guessed card is a no-op on any descendant: all candidates share its outcome, and its mask is ⊆ matched.
   - No-ops are strictly dominated, because V is monotone in c on the first two components and -1 action on the third.
   - `known` must stay in the key, because it drives the hint distribution. `matched` must stay, because it drives rewards.
   - Keys: u128 for n ≤ 64; 2×64-bit Zobrist (per-group random) XOR-packed with (depth, h, c, known, matched) for large sets.
   - Transposition table: fixed-size open addressing (for example 2^20 entries, about 40 MB) with depth-preferred replacement. Eviction only costs recomputation, never correctness.
6. **Branch-and-bound and pruning.**
   - Admissible per-puzzle E[S] bound: when c = 1 and h = 0, max_g mass(FM(g)∩S)/mass(S); otherwise 1. The top-(c·kmax) bound is **not** admissible for adaptive policies.
   - E[M] ≤ 6 − |matched|, or 6 per solvable future puzzle.
   - With these bounds, B&B pruned actions but not states, so it was useless.
   - What is needed is a Wordle-style lower bound on expected challenges. With weights sorted descending, at most kmax targets can be solved at depth 1, K·kmax at depth 2, K²·kmax at depth 3, and so on, where K is the max number of cells. This makes the third component prunable once E[S] = 1 and M is maximal, which is exactly the n ≈ 15 band that is still slow.
   - Move ordering: sort by (P(solve), entropy).
   - Iterative deepening with a time budget, returning the best completed depth.
7. **Hint node (implemented).** Group by unknown field using the *printed* value (level → n), at 6·|S| per node. The per-field histograms are shared with the leaf evaluation.
8. **Leaf evaluation (implemented).** At depth 1 the value is closed-form from histograms, and the hint is never optimal there. This gives a 5x speedup over the faithful port and removes about 95% of partition work.
9. **Top-K informative guesses via an entropy upper bound (measured).**
   - The bound H ≤ Σ_f H(response_f) is O(1) per guess and costs 0.2–0.5 ms for all guesses.
   - Exact top-16/64/256 needs 21–106 / 84–206 / 307–512 full partitions instead of 8023.
   - Use it to build root action sets (effective reps ∪ top-K entropy ∪ top candidates ∪ top quick-lexicographic).
   - Measured at n = 2258: depth-3 E[S] goes 0.296 → 0.611 when the action set grows from 26 to 140, at a cost of 39 → 724 ms native.
10. **Cross-puzzle continuation table W[r][h][c] instead of min(r, c, (c + 0.61h)/3.9).**
    - Store vector entries (W_S, W_M), 9×12×37 = 3996 cells × 2 × f32 = 32 KB, embedded with `include_bytes!`.
    - Fitting is approximate policy iteration run offline natively:
      1. Seed with the current formula.
      2. For each cell, simulate N fresh puzzles with common random numbers under policy π_W (depth 2 by default). Record (solved, ΔM, h′, c′).
      3. Backward over r: W[r][h][c] = E[1{solved}·(1 + W[r−1][h′][c′])], with M likewise. An unsolved puzzle ends the activity.
      4. Apply isotonic regression in h, c, r and clamp to 0 ≤ W ≤ min(r, c).
      5. Repeat until max|ΔW| < 0.01 or root decisions on a validation set stop changing (expect 2–4 rounds).
    - Cost at depth 2 (about 4 ms per puzzle): 3996 × 200 × 4 ms ≈ 3200 CPU-s ≈ 3 min wall on 20 threads per round, or about half that if cells where c saturates are skipped. At depth 3 (about 37 ms per puzzle) it is about 25 min per round.
    - Honesty: this estimates the value of *this* policy, not the optimum. Label cross-puzzle numbers "近似". Report final performance from a fresh-seed simulation. Re-fit whenever the pool or weights change.
    - Optional follow-up with the same machinery: a learned in-puzzle leaf value V̂(entropy, n, h, c, unknown fields) to replace 0 at the depth cutoff. Medium risk.
11. **Exact resource caps, final puzzle only (implemented).** c_eff = min(c, |S| + 6 − |matched|) and h_eff = min(h, #unknown). Not valid when a continuation depends on (h, c).
12. **Parallelism.**
    - Native: rayon over activities, which is embarrassingly parallel with per-activity seeds (measured near-linear scaling to 20 threads, with HT/E-core losses).
    - Browser: N Workers each holding a WASM instance (data under 1 MB each), parallel over activities for simulation, and over root actions or speculative deeper searches for recommendations.
    - WASM threads need COOP/COEP headers. Message passing is simpler and sufficient.
13. **Known techniques applied.**
    - Knuth minimax (worst case) is not our objective.
    - Expected-size and entropy heuristics (Irving, Neuwirth) are used for action-set filtering. Greedy information gain is a near-optimal adaptive identification heuristic in theory (generalized binary search, adaptive submodularity), which justifies it as a filter but not as the objective.
    - Exact Mastermind and Wordle solvers (Koyama–Lai; Selby / Bertsimas–Paskov) rely on memoizing candidate-set subproblems, equivalence merging, and lower bounds on expected guesses. We adopt all three.
    - The precomputed feedback matrix is replaced by the factorized per-field tables above.

---
## 4. Ranked optimizations (impact vs risk)

| # | Optimization | Measured / estimated impact | Correctness risk |
|---|---|---|---|
| 1 | Drop `guessed` from memo keys + prune no-ops | exact: 56 limit failures (11 s each) → ≤ 5 ms; states up to 887x fewer | very low (proof; 249/249 values identical) |
| 2 | O(\|C\|+U) quick metrics (histograms + full-match lists) | 529 ms → 0.5 ms (about 1000x) | very low (identical numbers) |
| 3 | Leaf-level O(1) evaluation in the horizon | 5x over port; depth 3 at n = 5000 in 72 ms | low (verified) |
| 4 | Effective action classes (O(1) signature) + per-node dedup with inherited lists | endgame actions 2.5x (median) to 15x fewer; exact 1.5–2x | low (bijection verified) |
| 5 | SoA u8 columns + factorized code + histogram | 2.8x vs typed JS, 35x vs string Map; 56 KB instead of 128 MB | low (nm vs n trap) |
| 6 | u64/u128 keys, Zobrist 128-bit, bounded transposition table | removes O(n) string keys; bounded memory | low |
| 7 | Exact resource caps (final puzzle) | 6–30x at large c | low–medium (zero continuation only) |
| 8 | Entropy upper bound for top-K + better root action sets | 102 ms → about 1.3 ms selection; depth-3 E[S] up to about 2x | selection exact; policy change needs simulation validation |
| 9 | Iterative deepening + time budget + move ordering | latency guarantee | low |
| 10 | rayon / multi-Worker parallelism | 1000 activities at depth 2 in 3.7 s wall | low (seeded) |
| 11 | Fitted W[r][h][c] table | better cross-puzzle trade-offs; minutes offline | medium (approximation; labeling) |
| 12 | Lexicographic B&B with E[actions] lower bounds | expected large for exact n ≥ 12 (current bounds: about 0) | medium (bound bugs silently change results) |
| 13 | Refinement-dominance pruning | 1.2–2x fewer actions | medium–high (needs proof) |
| 14 | SIMD kernels (simd128 / AVX2) | ≤ 2x on the partition kernel only | low, but adds complexity |
| – | Full 8023² feedback matrix | rejected (128 MB) | – |

---
## 5. Performance estimates and target budgets

WASM is assumed at 1.3–2x native for this scalar integer / hash-map code (bounds checks, no SIMD by default). Native figures are measured.

| workload | ops model | native (measured) | WASM (estimated) |
|---|---|---:|---:|
| quick metrics, all guesses | 6\|C\| + 8023·~25 | 0.13–0.50 ms | 0.3–1 ms |
| effective-class dedup | 8023·~20 + sort | 0.12–0.20 ms | 0.3–0.4 ms |
| full partition, all guesses, n = 5000 | 40M pairs × 2.2 ns | 87 ms | 130–175 ms |
| top-64 entropy via upper bound, n = 5000 | 0.35 ms + 116 × 5000 × 2.2 ns | ≈ 1.3 ms | 2–3 ms |
| depth-3 horizon, 26 actions, worst / typical opening | ~30k nodes | 72 / 10–40 ms | 110–145 / 15–80 ms |
| depth-3 horizon, 76–80 actions, n ≈ 2200 | ~60k nodes | 230 ms | 350–460 ms |
| exact final puzzle, n ≤ 7, any resources | ≤ 12k states | ≤ 5 ms (v3) | ≤ 10 ms |
| exact final puzzle, n ≤ 13, any resources | ≤ 26k states | ≤ 350 ms (v3) | 0.5–0.7 s (needs item 12) |
| one activity, depth 2 / depth 3 | ~38 decisions | 0.036 / 0.33 s | 0.06 / 0.5–0.66 s |

Proposed budgets:
- **Recommendation in the browser** (WASM in a Worker, never the main thread): p50 ≤ 50 ms, p99 ≤ 250 ms, with a hard 300 ms anytime cutoff through iterative deepening. The default is depth 3 with 26–48 actions chosen by the entropy upper bound. Report the depth actually completed.
- **Exact final-puzzle solve:** enable when the estimated cost is within budget (roughly n ≤ 12 today, n ≤ 16 once item 12 lands). Target < 200 ms.
- **Native simulation:** 1000 activities at depth 2 in < 5 s and depth 3 in < 40 s on 20 threads (measured 3.7 s and about 32 s extrapolated). 10k-activity policy comparisons in < 1 min at depth 2.
- **Browser simulation:** 500 activities at depth 2 with 4 Workers in < 10 s (JS today: about 3 h).
- **W-table fit:** under 10 min wall per policy-iteration round, natively.

---
## 6. Semantics and golden tests the rewrite must keep

1. Level **match** uses the `nm` bitmask (4 groups have 2 bits: nm = 3 means rank 0/1; nm = 4097 means 12/0). Level **hint and reveal** use the printed `n`. 7 groups share (b, a, r, n, atk, def) and differ only in nm. My prototype broke this once, causing a 1/1032 E[M] error on 2 of 156 cases.
2. The frame feedback key includes the target's full frame bits when the frame matches (pendulum 129/130/132/136/144/192).
3. A hint picks uniformly among fields not yet known. Challenge matches add to both `known` and `matched`; hints add only to `known`.
4. A challenge solves iff the mask is 63. Several targets can be solved by one guess; full-match sets have ≤ 3 members.
5. Terminal rule: c = 0 or empty set → [0, 0, 0]. The horizon allows a hint at c = 0 (a harmless quirk).
6. Keep the export harness (`hcases.txt` format: state + actions + JS value) as golden tests with 1e-9 tolerance. Define a deterministic tie-break over `optimalActions`.

Verification tally so far:
- exact: 93/93 values identical to JS; 56 JS-limit cases solved.
- horizon: 156/156 plus 42/42 large openings identical.
- exact v3 (caps + inheritance): identical to v1 on every tested row.
- entropy upper bound: ≥ exact entropy for every guess in 37 states.


## Risks
- Semantic drift in the rewrite (nm vs n for level, frame reveal bits, hints over unknown fields, solve means mask==63). I hit the nm/n bug myself; golden tests exported from the JS solver (hcases format, 1e-9 tolerance) are mandatory.
- WASM figures are estimates (1.3-2x native): no wasm32 target, wasm-bindgen or wasm-pack is installed. Measure early once the toolchain is added (rustup target add wasm32-unknown-unknown; the wasm-bindgen-cli version must match the crate).
- The resource caps (c_eff, h_eff) are valid only with zero continuation (final puzzle). Using them inside the multi-puzzle horizon, where cont depends on (h,c), would silently change values.
- Refinement-dominance pruning and any B&B bounds need proofs; an inadmissible bound, like the suggested top-(c*kmax) E[S] bound, silently returns wrong actions. Always keep a no-pruning reference path for tests.
- Better root action sets and uncapped depth 3 change recommendations relative to the original tool. That is an improvement within the model, but users comparing against the old tool will see different advice; it needs simulation-based validation and release notes.
- The fitted W table estimates one specific policy's value under the simulated model: targets weighted by wm, and the real event may exclude cards. It is not optimal; convergence of approximate policy iteration with Monte-Carlo noise is not guaranteed; the UI must keep the '近似' label.
- Floating-point summation order differs between JS and Rust (Map insertion order vs counting sort), so ties within EPS may pick a different but equivalent action; the UI 'equivalent choices' list must be computed with an explicit tolerance.
- The u64 bitset exact states cap n at 64; a larger exact gate needs u128 or SmallVec bitsets. In the browser, memo/TT memory must be bounded (the exact v1 run at n=20 reached about 1M states).
- Policy differences are small (mean solved about 8.98/9, all-9 97.5-98.1%), so policy A/B conclusions need 10k+ simulated activities; small-sample browser simulations can mislead.

## Open questions
- Should the first Rust/WASM release reproduce the JS decisions (same 26-action root set, depth 2/3 fallback semantics) for parity, or adopt the improved action sets and uncapped depth 3 right away?
- Is E[S] -> E[first-time matches] -> -E[actions] still the objective the user wants? It is nearly saturated (8.98/9), and the config's milestone and solve points are ignored by rewardVector. Should P(all 9 solved) or reward points become part of the objective?
- Which policy should the W[r][h][c] table be fitted under (depth 2 is cheap, depth 3 matches the online solver), and should it carry W_M (expected matches) separately instead of 6*W_S?
- Browser parallelism: accept COOP/COEP headers on Cloudflare to enable WASM threads, or stay with multiple independent Workers? The latter is simpler and sufficient per these numbers.
- Confirm with the in-game UI that a hint on the level field shows the printed level/rank (n), and how the event handles the multi-level groups (nm=3, 4097); the current JS assumes printed n.
- Should the exact-solver gate be cost-based (estimated states from n, effective actions, h, c) instead of the fixed n<=7/h<=4/c<=6 rule, and should exact also run on non-final puzzles with the W continuation? Resource caps would then not apply.
