# Reader report: engine

## Summary
I read all of dist/solver-core.js and the engine-related parts of dist/app.js (candidateIndices through recommendationReason, plus the simulation code, the worker and the WebMCP summary). I also rebuilt the balanced pipeline headless in Node against the real card data to measure it. The engine has four stages: a candidate filter, an O(A·N) 'quick' scoring pass over every legal guess (about 8023 cards), a refinement pass on up to 340 finalists that computes the exact feedback entropy, and a policy tier. The policy tier is tried in order: exact Bellman (final puzzle only, N≤7, hints≤4, challenges≤6, 160000 states), then a restricted-horizon tree at depth 3 and then depth 2 (a hand-picked set of about 25 actions, 24000 states, continuation min(r,c,(c+0.61h)/3.9)), then a branch-and-bound certificate. Measurements show the exact tier hits its state limit on 18 of 34 eligible states and freezes the main thread for 7–57 s each time; the cause is the guessed-card set in the memo key. Dropping that set from the key (safe, because re-guessing a card is always strictly worse) plus merging equivalent actions in every state gave bit-identical values and solved every case in 0.3 ms–3.6 s of JS. Depth-3 always fails near the opening (1–2 s thrown away), depth-2 never uses more than about 1200 of its 24000 states, so the branch-and-bound path is dead in practice. Other findings: several dead functions and unused parameters, a 'lower bound' label on a heuristic value, a myopic zero value at the search cutoff, and exact-parity traps for the Rust port (Math.log2 vs Rust's log2 last-bit differences, summation order, the epsilon comparator panicking in Rust sort_by, and budget counts that depend on JS memo identity).

## Findings
- **Exact Bellman tier often hits its state limit and freezes the UI for 7–57 s**: I tested 34 random final-puzzle states that pass the gate (N≤7, h≤4, c≤6). solveExact threw EXACT_STATE_LIMIT:160000 on 18 of them, after 6.9–57 s of synchronous main-thread work, and the app then fell back to the restricted tree. Even N=2, h=0, c=6 fails. The cause is that the memo key contains the sorted guessed-card set, so every guess order becomes its own state. [dist/solver-core.js:69 (stateKey includes guessed), :102 (throw), dist/app.js:1042 (gate), :1059 (maxStates 160000), :917 (synchronous call); scratchpad exactprobe.js output: 'orig success 16 / 34']
- **Removing guessed from the memo key and merging equivalent actions per state keeps values bit-identical**: A variant with a packed key (candidate bitmask, known, matched, h, c; no guessed set) that also skips actions whose feedback over the current subset repeats an earlier one matched the original value exactly (maxdiff 0) on all 16 states the original solved. It also solved all 18 failing states, using 741–21261 states and 32 ms–3.6 s of JS. Proof sketch: a guessed card g leaves every remaining candidate with the same non-63 mask, already inside matchedMask. Re-guessing it therefore yields [0,0,-1]+V(s,c-1), which is strictly worse than V(s,c) (exact tier: some remaining candidate is always a guess with positive solve chance; restricted tier: V is monotone in depth and c, and stop exists). So guessed actions never reach optimalActions. [scratchpad /tmp/claude-1000/-home-parry-gitRepos-md-card-decoder/cb7e6cca-357d-4211-9aca-7cbb5c5749b0/scratchpad/exactprobe.js (solveFast)]
- **Depth-3 restricted search always fails near the opening, and depth-2 never fails**: With one reveal (N≈440–5000), depth 3 goes past 24000 states and throws after 1.0–2.0 s. All that work is thrown away and the search restarts at depth 2, which uses 654–1188 states in 60–490 ms. Mid-game states (N≈11–233) finish depth 3 with 1.4k–19k states in 18–660 ms. Depth-2 needs at most 1 + Σ_fields|values| + Σ_actions outcomes (≤57 per action), so the 24000 budget is never reached. [dist/solver-core.js:208; dist/app.js:1077-1086; scratchpad harness.js and d2probe.js ('worst d2 expansions 1188')]
- **Branch-and-bound certificate path is unreachable in practice**: boundedCertificate, the balanced branch of hintAdvice and the B&B reason text only run when both tiers return null. The restricted tier returns null only if DecoderSolver is missing or quick is empty, since depth 2 never exceeds its budget. The DecoderSolver-missing fallback vector [1, remaining, Infinity] is also malformed. [dist/app.js:920, :943-948, :979-1004 (:981 fallback), :1122-1132]
- **The 'feasible lower bound' label is wrong for the restricted value**: tryRestrictedPolicy sets lower = result.value, and the UI says '当前可行下界'. That value includes the heuristic continuation min(r,c,(c+0.61h)/3.9), which can overestimate: at the opening it assumes all 8 future puzzles get solved. So it is not a valid lower bound. The upper and value fields are never rendered at all. [dist/app.js:1082, :1180-1182 (stats label); dist/solver-core.js:197-203]
- **Myopic leaf value and saturated continuation in the restricted tree**: When depth reaches 0, an unsolved state gets ZERO, which drops the value of the rest of the current puzzle and all future puzzles. The first component therefore becomes P(solve within d actions)·(1+cont). The continuation is capped by `future`: at the opening (35+0.61·11)/3.9 = 10.7 > 8, so spending hints or challenges costs nothing in solved branches until resources get tight. [dist/solver-core.js:205 (ZERO leaf), :199-202 (cap); harness output value [9,54,-2.7] at puzzle 1 with N=17]
- **Dead code and unused parameters**: solver-core: thresholdGain is unused; rewardVector ignores its config and puzzle arguments; terminalPremium always returns 0; maxVector is unused; stateUpperBound computes `regular` and discards it; the defaults maxStates 250000 and 40000 are always overridden (160000 and 24000). initialValue is dead in production because config.puzzles is set to the current puzzle, and if it did run it would use the current candidates as the next puzzle's prior. visit() returns [1,0,0] for puzzle>puzzles, which is unreachable and would double-count a solve. app.js: simulateOneActivity, simulationPolicyDecision, pickSimulationChallenge, filterSimCandidates, simulationHintEntropy, percentile and maxActivityScore are never called (the Worker does the simulation). item.remaining is never read. item.score=points in balanced mode is unused. The final `else` in decideHintUsage and its balanced early return are unreachable. The balanced text in recommendationReason is unreachable. resetPuzzle has identical if/else branches. [dist/solver-core.js:19,42-47,49-54,65,71-94,97,184-186,192; dist/app.js:169,908-909,1101,1118,1203,1438-1653,1836-1837]
- **Analysis modes use heuristic hint thresholds; strict mode ignores reward points**: points/solve/info modes skip the solvers and use decideHintUsage thresholds (3 groups, 3.1 effective candidates, 0.05/0.6/1.5/0.28 bits, 0.92/0.48 solve, 3.1 challenges per puzzle, 0.72). The 'points' mode never recommends a hint. The balanced mode compares only (E[solved], E[first-time matched fields], -E[actions]), so the configured solvePoints and milestones have no effect on the default recommendation. [dist/app.js:1090-1120, :860-865, :967-969]
- **The policy's chosen card can be shown with an approximate info value**: If the policy picks a card that is not a finalist (possible for an exact-tier representative or a card from the top-8 infoApprox list), app.js copies infoApprox (a sum of independent binary entropies, an upper bound) into info and shows it as '信息增益 x bit'. [dist/app.js:922-929]
- **'all' pool can recommend cards that cannot be picked in game**: The action list is every card with weight>0 in the current pool. With pool 'all' (wa>0 for all 8210 groups) that includes the 187 non-Master-Duel groups. [dist/app.js:829, :1026, :265-267]
- **No time budgets, only state-count budgets; solvers are synchronous**: Only the quick and refine loops yield (setTimeout 0 every 80 actions or 30 finalists; progress shown as 0–70% and 70–100%). tryExactBellman and tryRestrictedPolicy run synchronously, so the calculation token cannot cancel them and the UI freezes. A single opening click takes about 1.5–3.3 s (quick 0.3–0.8 s + failed depth-3 1–2 s + depth-2 0.1–0.5 s). [dist/app.js:835,867-870,911-914,917-918,933; harness.js timings]
- **Measured time breakdown and native Rust baseline**: CPU profile: restricted visit takes 51% of time, then the quick loop, matchMask and vector add/scale, and 4.4% GC. Quick stage at N=5000, A=8023: 808 ms in JS. A native Rust naive AoS loop takes 220 ms; SoA plus a 64-bin mask histogram per action takes 110 ms. Every quick statistic (points, solve, newMatches, bitMass) comes from that 64-bin histogram. For balanced mode, solve, newMatches and infoApprox need only per-field marginals plus an exact-match lookup, which is O(N+A). [scratchpad prof.js cpuprofile; scratchpad qbench/src/main.rs]
- **The generated worker source is in sync**: simulation-worker-source.js exactly equals solver-core.js + '\n' + the worker with its importScripts line removed. The worker's decision() duplicates the balanced policy pipeline: quick without points, the same exact gate, the same 18/8/8 action set, depths [3,2], then fallback to quick[0]. It adds a FIFO cache of 4000 entries. [tools/build-simulation-worker.mjs; node check 'in sync: true'; dist/simulation-worker.js:20-28]

## Spec
# Card Decoder recommendation engine: reverse-engineered spec for a Rust/WASM port

Files: `/home/parry/gitRepos/md-card-decoder/dist/solver-core.js` (247 lines, read in full) and `/home/parry/gitRepos/md-card-decoder/dist/app.js` (1924 lines; engine section is lines 265–345 and 816–1204, simulation 1330–1700, summaries 1846–1920). `dist/simulation-worker.js` has a copy of the policy pipeline. `dist/simulation-worker-source.js` is generated and was verified to match the sources exactly.

Throwaway measurement tools (outside the repo): `/tmp/claude-1000/-home-parry-gitRepos-md-card-decoder/cb7e6cca-357d-4211-9aca-7cbb5c5749b0/scratchpad/`:
- `harness.js`: a headless copy of `calculateRecommendations`, usable as a starting point for a golden-fixture generator.
- `exactprobe.js`: the exact tier compared with an optimised variant.
- `d2probe.js`: depth-2 state counts.
- `prof.js`: CPU profile.
- `qbench/`: Rust micro-benchmark.

---

## 1. Data and state model

### 1.1 Card records
Each element of `CARD_DATA.cards` is one behaviour group. The engine uses these fields:

| field | meaning | domain |
|---|---|---|
| `b` | frame bitmask | 1 normal, 2 effect, 4 fusion, 8 synchro, 16 xyz, 32 link, 64 ritual, 128 pendulum; 13 distinct values incl. 129,130,132,136,144,192 |
| `a` | attribute bit | 7 values (1..64) |
| `r` | race bit | 25 values |
| `n` | level/rank/link value that gets revealed | 1..13 |
| `nm` | level match mask | `1<<n`; 4 cards have 2 bits (3 = {0,1}, 4097 = {0,12}) |
| `atk`, `def` | stats | -2 = '?', `def` -3 = link (no DEF) |
| `wm`, `wa` | weight (number of real cards in the group) in the MD pool / all-cards pool | wm>0 for 8023 groups (Σ9059, max 21); wa>0 for all 8210 |

`w(i) = pool==='md' ? wm : wa` (app.js:265). The prior over targets is proportional to w.

### 1.2 matchMask (solver-core.js:31, duplicated at app.js:305)
```
bit0 (1)  frame   : (t.b & g.b) != 0
bit1 (2)  attr    : t.a == g.a
bit2 (4)  race    : t.r == g.r
bit3 (8)  level   : (t.nm & g.nm) != 0
bit4 (16) atk     : t.atk == g.atk
bit5 (32) def     : t.def == g.def
```
The relation is symmetric: a pendulum guess also matches a non-pendulum target that shares a base-frame bit. Solved means mask == 63.

### 1.3 Puzzle state (app.js:53-73)
`{config, puzzle, hints, challenges, pool, known:{fieldKey:{value, source∈initial|hint|challenge}}, matchedMask, logs[], initialUsed, solved, scores…}`
- `known` holds every value displayed in the game. For a challenge match, the stored value is `borderReveal` (target.b) for frame, `numberReveal` (target.n) for level, and the guess's own value for the other fields (app.js:797-802).
- `matchedMask` is the OR of the challenge masks only. Reveals and hints never set it.
- `logs`: `{type:'reveal', field, value, source}` or `{type:'challenge', guess, mask, borderReveal|null, numberReveal|null, delta}`.
- The engine derives `knownMask = OR of bit(f) for f in known` (app.js:1043, 1074) and the guessed set `G = {log.guess | challenge logs}` (app.js:827).

### 1.4 candidateIndices (app.js:316-334)
```
for i in 0..|CARDS| ascending:
  if w(i) <= 0: reject
  for (key,{value}) in known: if field(i,key) !== value: reject      // exact equality, also on b and n
  for log in logs where type=='challenge':
    if matchMask(CARDS[i], CARDS[log.guess]) !== log.mask: reject
    if (log.mask&1) && borderReveal!=null && CARDS[i].b !== borderReveal: reject
    if (log.mask&8) && numberReveal!=null && CARDS[i].n !== numberReveal: reject
  emit i
```
The output is in ascending index order. That order carries through the whole engine and fixes the floating-point summation order. The `known` checks from challenges repeat the log checks. Cost is O(|CARDS|·(|known|+|logs|)), about 1–4 ms.

## 2. Observation and transition model

### 2.1 Hint
- Allowed when `hints>0 && knownMask!==63`.
- The field is drawn uniformly from the unknown fields, iterated in FIELDS order `[border, attribute, race, number, attack, defense]`.
- Candidates are partitioned by that field's value (`b, a, r, n, atk, def`), in first-occurrence order over the candidate list.
- Branch probability is `mass(S)/total/|U|`, evaluated left to right.
- Child state: `hints-1`, `C=S`, `knownMask|bit`. matchedMask is unchanged.
- Reward: none per branch; the summed hint value gets `value[2] -= 1` once at the end.

### 2.2 Challenge with guess g
- Feedback key per target t: `(m, m&1 ? t.b : ∅, m&8 ? t.n : ∅)`. The solvers build it as a string: `${m}|${b or ''}|${n or ''}` (solver-core:132, 229), or `${m}:${b}:${n}` for representative signatures (app.js:1032). The refine stage packs it as a number: `m | (m&1?t.b:0)<<6 | (m&8?t.n+1:0)<<14` (app.js:894-897). All three give the same partition.
- Outcomes are grouped in first-occurrence order over candidates. `p = mass(o)/total`.
- Reward vector `rewardVector = [solved?1:0, popcount(matched|m) − popcount(matched), −1]` (solver-core:49-52).
- If solved: exact tier adds `initialValue(puzzle+1, h, c−1)`, which is always [0,0,0] in production. Restricted tier adds `continuationValue(h, c−1)`.
- Otherwise the child state is `c−1`, `C=o.targets`, `knownMask|m`, `matchedMask|m`, `G∪{g}`.
- Actions in G are skipped.

### 2.3 Objective and comparison
- Value vector: `[E solved, E first-time matched fields (challenges only), −E actions]`.
- `add` is element-wise +. `scale` is element-wise ×p. The accumulation is `acc = add(acc, scale(child, p))`.
- `compare(l,r)` (solver-core:13): at the first index where `|l−r| > 1e-11`, return sign. Otherwise 0. This is an absolute epsilon, not transitive.
- Tie handling in the solvers: replace best only if `compare>0`; on `==0`, append to `optimalActions`. The first maximum in action order wins.
- In both tiers the hint is evaluated before any challenge, so it wins exact ties.
- The restricted tier starts with `best={value:ZERO, action:stop}`. The exact tier starts with `best=null` and has no stop, so it is forced to keep acting until solved or out of challenges.

### 2.4 Continuation (restricted only, solver-core:197-203)
```
future = R − 1                      // R = remainingPuzzles = puzzles − puzzle + 1, min 1
if future<=0 or c<=0: [0,0,0]
eq = c + 0.61·max(0,h)
s  = min(future, c, eq/3.9)
return [s, 6s, 0]
```
### 2.5 stateUpperBound (solver-core:180-188)
`solvable = min(max(0, P−puzzle+1), max(0, c))`, returns `[solvable, 6·solvable, 0]`. The `regular` loop is dead.

## 3. calculateRecommendations pipeline (app.js:816-954)

Preconditions: `N>0`, `challenges>0`, `!solved`. `mode ∈ {balanced (★ strict, default), points, solve, info}`.

1. **Candidate and action sets.** `C = candidateIndices()` (ascending), `T = Σw(C)`. `A = [i ascending | w(i)>0 ∧ i∉G]`, about 8023 cards in MD pool minus guesses. This is every legal guess, not only candidates.
2. **Quick stage**, over all of A × C (app.js:834-871). Per g:
   - `gain = thresholdGain(old, old|m)`:
     - premium puzzle (`puzzle ≤ premiumPuzzles`, default 3): Σ milestone.points for each milestone k with `popcount(old)<k≤popcount(old|m)` (default k=1,3,5 at 10 points each);
     - regular puzzle: `(popcount(old|m)−popcount(old))·regularMatchPoints`.
   - If m==63, also `gain += solveReward` (premium: solvePoints = 70; regular: regularSolvePoints = 0) and `solveMass += w`.
   - `points = Σgain·w/T`, `solve = solveMass/T`, `newMatches = Σ(popcount(old|m)−popcount(old))·w/T`.
   - `infoApprox = Σ_f Hb(bitMass_f/T)`, where `Hb` is binary entropy with 0 at p≤0 or p≥1.
   - `score`: balanced = points (unused); points = `points + 0.001·solve`; solve = `1000·solve + points`; info = `infoApprox + 0.001·solve`.
   - Yields every 80 actions; progress runs 0–70%.
3. **Sort quick**, stable. Balanced: descending by `compare([solve,newMatches,−1])`. Other modes: `b.score − a.score`. Ties keep ascending card index.
4. **Finalists**: `quick[0:300]`, then the entries for `stableSort(C, w desc)[0:40]` that are not already included, appended in that order. At most 340.
5. **Refine** each finalist in O(N): `info = −Σp·log2 p` over the full feedback keys, and `remaining = Σp·mass` (never used). Balanced sets `score = points` (unused); info mode sets `score = info + 0.001·solve`. Yields every 30; progress runs 70–100%. Then stable re-sort of the finalists with the same comparator as step 3.
6. **Policy (balanced only)**:
   - `exact = tryExactBellman(C, G)`.
   - If null: `restricted = tryRestrictedPolicy(C, G, quick)`.
   - `policy = exact || restricted`.
   - `lastProof = policy || boundedCertificate(quick)` (the latter is unreachable in practice).
7. **Promote the policy's challenge** (app.js:921-930). Find its item in the finalists. Otherwise take the quick item, set `info := infoApprox`, and append it. Move it to finalists[0]. `lastRecommendations = finalists[0:6]`.
8. **Advice object**:
   - With a policy: `{use: action.type==='hint', entropy: expectedHintEntropy(C,T), strict:true, proven, method, value, expandedStates, depth}`.
   - Without a policy: `hintAdvice(C, T, recs[0], mode)`.
   - If the policy chose a challenge: `equivalentChoices = unique(optimalActions where type=='challenge' and index∈C)`, kept only if there are at least 2.
   - Restricted tier: `reason` = the depth/expanded-states text.
   - B&B path (dead): merge `lastProof` and use the B&B text.
9. `renderRecommendation(recs, advice)`. The WebMCP tool `calculate_best_challenge` returns `recommendationSummary()`.

### 3.1 tryExactBellman (app.js:1039-1066)
- Gate: `puzzle == config.puzzles && N ≤ 7 && hints ≤ 4 && challenges ≤ 6`.
- Actions come from `exactActionRepresentatives` (app.js:1024-1037). For every card with w>0 and not in G, ascending, build the signature string over C, e.g. `m:b:n|m:b:n|…`. Keep the first card per signature.
- Calls `solveExact` with `config.puzzles := puzzle` (so the next-puzzle value is 0), `universe: C` (only initialValue uses it, and that path is dead), and `maxStates: 160000`.
- `EXACT_STATE_LIMIT` returns null; any other error is logged and null returned.
- Result: `{…, method:'exact-bellman', lower=upper=value, proven:true}`.

### 3.2 tryRestrictedPolicy (app.js:1068-1088)
- Actions are an insertion-ordered set: `quick[0:18]`, then `stableSort(quick, infoApprox desc)[0:8]`, then `stableSort(C, w desc)[0:8]`. Measured size 18–32, usually about 25.
- Parameters: `maxStates 24000`, `remainingPuzzles = P − puzzle + 1`, `resourceModel {0.61, 3.9}`.
- Tries depth 3, then depth 2. `HORIZON_STATE_LIMIT` moves to the next depth.
- Result: `{…, proven:false, lower:value, upper:stateUpperBound}`.

### 3.3 boundedCertificate (app.js:979-1004), dead in practice
- Each quick action gets `lower = [solve, newMatches, −1]`, sorted descending.
- `upper_i = (challenges ≤ 1) ? lower_i : globalUpper`.
- `pruned` counts `best.lower ≥ upper_i`. `proven = (every other action pruned) ∧ ¬hintPossible`.
- Returns `{method:'branch-and-bound', proven, lower, upper, pruned, totalBranches = |quick| + hintPossible, depth:1}`.

### 3.4 Hint helpers
- **expectedHintEntropy** (app.js:1006-1022): the average, over unknown fields, of the weighted entropy of the field's value distribution among C.
- **weightedCandidateEntropy** (app.js:956-965): H over groups with p = w/T.
- **hintAdvice** (app.js:1122-1151): balanced mode returns `{use:false, strict, proven:false, lower: aov(best), upper: lastProof.upper, reason}`. With no unknown field or no hints: `{use:false, entropy:0}`. Otherwise it calls decideHintUsage.
- **decideHintUsage** (app.js:1090-1120):
```
effC = 2^targetEntropy; R = max(1, P+1−puzzle); future = max(0, R−1)
futureReserve = min(h, future); spendable = max(0, h − futureReserve); last = future==0
challengeBudget = c/R ; hintBudget = h/R (reported only)
if mode=='balanced' → {use:false,…}                                // unreachable
if h<=0 || targetEntropy<=0 || mode=='points' → use false
if !last && (N<=3 || effC<=3.1) → false
rel = bestInfo>0 ? hE/bestInfo : hE
informative = last ? hE>=0.05 : hE>=0.6 && (hE>=1.5 || rel>=0.28)
if !informative → false
lowDirect = bestSolve < (last ? 0.92 : 0.48); pressure = challengeBudget < 3.1
canSpend = spendable>0 || (pressure && h>0)
info : use = canSpend && (last || hE >= 0.72·bestInfo)
solve: use = canSpend && lowDirect && (last || pressure)
else : use = canSpend && lowDirect && (last||pressure||spendable>0||premium)   // unreachable
```

### 3.5 Magic constants

| constant | where | role |
|---|---|---|
| 1e-11 | solver-core:9, app.js:974 | lexicographic epsilon |
| 300 / 40 | app.js:877/881 | finalists from quick / heaviest direct candidates |
| 80 / 30 | app.js:867/911 | yield intervals; progress split 70/30 |
| 0.001, 1000 | app.js:863-865, 910 | analysis-mode score blending |
| 7, 4, 6, final puzzle | app.js:1042 | exact gate |
| 160000 (default 250000 unused) | app.js:1059, solver-core:65 | exact state budget |
| 18 / 8 / 8 | app.js:1071-1073 | restricted action set |
| [3,2] (default 3) | app.js:1079, solver-core:192 | depths |
| 24000 (default 40000 unused) | app.js:1077, solver-core:192 | restricted state budget |
| 0.61 / 3.9 | app.js:1078, solver-core:194 | continuation hint/challenge ratio and resources per solve |
| 3, 3.1, 0.05, 0.6, 1.5, 0.28, 0.92, 0.48, 3.1, 0.72 | app.js:1103-1117 | heuristic hint rules (analysis modes) |
| 6 / 3 / 8 / 6 | app.js:931, 1190-1191, 1170 | recs kept / alternatives shown / tie buttons / tie names |
| 4000 FIFO, 250 ms | simulation-worker.js:27,29 | simulation decision cache, progress pulse |
| preset 9 puzzles, 11 hints, 36 challenges, 3 premium, milestones 1/3/5 at 10, solve 70, regular 1/0 | presets.js, app.js:11-18 | config |

No time budgets exist anywhere, only state counts.

## 4. Solver pseudocode (for exact reimplementation)
```
solveExact(o):                                   // solver-core:56-178
  memo = {}; expanded = 0; max = o.maxStates||250000
  visit(s):
    if s.puzzle > P: return [1,0,0]              // unreachable
    if s.c<=0 or s.C empty: return [0,0,0]
    key = (puzzle,h,c,known,matched,C list,sorted G)
    if key in memo: return memo[key]
    if ++expanded > max: throw EXACT_STATE_LIMIT
    T = Σw(C); best = null
    if h>0 and known!=63:
      U = [f in FIELDS order | !(known&bit f)]; v=[0,0,0]
      for f in U: for S in partitionByValue(C,f) (first-occurrence order):
          v = v + visit(s{h−1, C=S, known|bit f}) · (mass(S)/T/|U|)
      v[2] −= 1; best = (v, hint, [hint])
    for g in o.actions (given order), g ∉ s.G:
      v=[0,0,0]
      for o in partitionByFeedback(C,g):
        br = [m==63, pop(matched|m)−pop(matched), −1]
        br += (m==63) ? initialValue(puzzle+1,h,c−1) : visit(s{c−1,C=o,known|m,matched|m,G∪{g}})
        v = v + br·(mass(o)/T)
      if best==null or cmp(v,best)>0: best=(v,g,[g]) elif cmp==0: best.optimal.push(g)
    memo[key]=best; return best
  initialValue(p,h,c): p>P→[0,0,0]; c<=0→[0,0,0]; else Σ_f Σ_S (mass/T/6)·visit(p,h,c,C=S,known=bit f,matched=0,G=∅) over `universe`, memoised by (p,h,c)
  return visit(root) + {expandedStates, memoStates}

solveRestrictedHorizon(o):                       // solver-core:190-244
  visit(s,d):
    if d<=0 or C empty or (h==0 and c==0): return ([0,0,0], stop)
    key=(d,h,c,known,matched,C,sorted G); memo; if ++expanded>max throw HORIZON_STATE_LIMIT
    best=([0,0,0], stop, [stop])
    hint as above with visit(child, d−1); v[2]−=1; consider
    if c>0: for g in actions ∉G: br = reward + (solved ? cont(h,c−1) : visit(child,d−1)); consider
```
Parity details:
- Candidate lists stay ascending, because partitions preserve parent order.
- Summation order: fields in FIELDS order, groups in first-occurrence order, then `acc + child*p` with p computed as `(mass/T)/|U|`.
- Rust must use `[f64;3]` without FMA. rustc does not contract to FMA by default, and neither does wasm32 without relaxed-simd.

## 5. Complexity and measured cost (JS, this machine)

| step | complexity | measured |
|---|---|---|
| candidateIndices | O(8210·(K+L)) | 1–4 ms |
| quick | O(A·N·(1+|milestones|)), A≈8023 | N=105: 33 ms; N=1239: 217 ms; N=2179: 400 ms; N=5000: 808 ms |
| sorts | O(A log A) | few ms |
| refine | O(F·N), F≤340 | 1–31 ms |
| representatives | O(A·N) string building, N≤7 | about 5 ms |
| solveExact | O(S·(R·N + 6N)) plus string keys; R = representatives (62–588); S ≤ 160000 | success: 0–3.4 s (e.g. N4 h4 c3: 47.5k states, 3.4 s). Failure: 6.9–57 s, 18 of 34 eligible states |
| restricted d=2 | S ≈ 1 + Σ_f\|values\| + Σ_a outcomes(a) ≤ about 1200 | 60–490 ms |
| restricted d=3 | S ≈ 1 + b + b·b′, b ≈ 25·(outcomes ≤57) | N ≤ about 250: 1.4k–19k states, 18–660 ms. N ≳ 440: throws after 1.0–2.0 s |

- **Per click.** The opening costs 1.5–3.3 s, mid-game 0.1–0.7 s, and the final puzzle anywhere from ms to about 60 s.
- **Profile.** Restricted visit 51%, the quick loop about 8%, anonymous callbacks 7%, matchMask 6%, add/scale/compare about 10%, GC 4%.
- **Why the exact tier blows up.** With guessed in the key, the state count is exponential in c: about (R·k̄)^c paths with almost no transpositions. Without guessed, reachable states are bounded by 2^N·64·64·(h+1)·(c+1). The measured maximum was 21k.
- **Native Rust quick stage** at N=5000: naive AoS 220 ms, SoA + 64-bin mask histogram 110 ms. All of points, solve, newMatches and bitMass come from `hist_g[m]` with 64-entry gain and new-match lookup tables. For balanced mode, solve, newMatches and infoApprox need only per-field marginal masses (lookup per guess value, frame and nm via small compat tables) plus an exact-match hash for the solve mass. That is O(N+A) and makes the quick stage about 1 ms. Full points are then needed only for the displayed top 6.

## 6. Bugs, dead code, suspicious logic

| # | location | issue |
|---|---|---|
| 1 | solver-core:69, :206 | String memo keys include the sorted guessed set and the full candidate list: O(N) allocation per visit, plus a state explosion (see §5). Guessed actions are strictly dominated, so removing G from the key leaves values and root optimal-action sets unchanged (verified bit-identical on 16 of 16 cases). |
| 2 | solver-core:102, :208 | Running out of budget throws and discards all work. Depth 3 fails, then depth 2 restarts from scratch. |
| 3 | app.js:1042 | Exact gate admits states that take up to 57 s before failing. Runs on the main thread (app.js:917), and the calculation token cannot interrupt it (checked only at :835/:933). |
| 4 | solver-core:97 | `visit` returns [1,0,0] for puzzle>puzzles, unlike initialValue's [0,0,0]. Unreachable. |
| 5 | solver-core:71-94, app.js:1049 | initialValue is dead (config.puzzles := puzzle). If enabled, it would use the current candidates as the next puzzle's prior. |
| 6 | solver-core:42-47, 54, 19, 184-186 | thresholdGain unused; terminalPremium ≡ 0; maxVector unused; `regular` discarded. |
| 7 | solver-core:49-52, 233 | rewardVector ignores its config and puzzle arguments; called with `{}` and 0. |
| 8 | solver-core:65, 192 | Default budgets 250000 and 40000 are never used. |
| 9 | solver-core:205 | Depth-0 leaf is ZERO for unsolved states, so the search maximises P(solve within d)·(1+cont). This is a myopic proxy. |
| 10 | solver-core:199-202 | Continuation saturates at `future`, so resources are free early (10.7 > 8 at the opening). It counts 6 matches per future solve and nothing for unsolved leaves. |
| 11 | solver-core:209 vs 104 | Restricted tier has a stop action; exact tier does not. The meaning of the −E[actions] component differs between tiers. |
| 12 | solver-core:118, 221 | `new Set(guessed)` is copied for hint children needlessly. |
| 13 | app.js:1082, 1180 | A value containing the heuristic continuation is labelled a 'feasible lower bound'. `value`, `lower` and `upper` are never rendered. |
| 14 | app.js:920, 943-948, 979-1004, 1122-1132 | B&B path and hintAdvice's balanced branch are unreachable, since depth 2 uses at most 1188 of 24000 states. The fallback upper vector `[1, remaining, Infinity]` is malformed. |
| 15 | app.js:925 | A non-finalist policy card is shown with infoApprox as its info bits. |
| 16 | app.js:829, 1026 | 'all' pool actions include non-MD cards that cannot be picked in game. |
| 17 | app.js:908-909 | `remaining` computed and never used; balanced `score` assignment unused. |
| 18 | app.js:1101, 1118, 1203 | Unreachable branches (balanced early return, final else, balanced reason text). |
| 19 | app.js:935 | `entropy` is computed and shown even when hints = 0. |
| 20 | app.js:1438-1653, 1655, 169 | In-page simulation code (simulateOneActivity, simulationPolicyDecision, pickSimulationChallenge with its own constants 220/16/24/32/48/.75/3.5/.35/5.5/1.8/.2, filterSimCandidates, simulationHintEntropy, percentile) and maxActivityScore are dead. The Worker is used instead. |
| 21 | app.js:1836-1837 | resetPuzzle: both if/else branches are identical. |
| 22 | app.js:305/340 vs solver-core | matchMask, popcount, thresholdGain and FIELDS are duplicated in three places (app, core, worker). |
| 23 | design | The strict decision ignores the configured reward points (premium vs regular puzzles are identical). Only the analysis modes use points. |

## 7. UI and engine contract

**Inputs**
- Cards (b, a, r, n, nm, atk, def, wm, wa) and pool.
- config `{puzzles, totalHints, totalChallenges, premiumPuzzles, milestones[{matches,points}], solvePoints, regularMatchPoints, regularSolvePoints}`.
- state `{puzzle, hints, challenges, known, matchedMask, logs, solved}` and `mode`.

**Outputs rendered** (renderRecommendation app.js:1153-1196, clearRecommendation :619-633):
- `recs[0]`: `.index` for the name, image, frame/attr/race/n/atk/def chips and the click-to-select card; `.points` (2 dp, metricPoints); `.solve` (percent via formatPercent: '<0.05%', 2 dp below 10%, else 1 dp; metricSolve); `.info` (2 dp bits).
- `recs[1..3]`, or `recs[0..2]` when a hint is recommended: index, points, solve, info.
- `advice.use`, which shows '先使用1次提示'.
- `advice.equivalentChoices` (at least 2): up to 8 buttons and 6 names.
- `advice.strict / proven / method ('exact-bellman' | 'restricted-horizon' | 'branch-and-bound') / depth / expandedStates / entropy / reason`.
- Heuristic-mode advice: `lastPuzzle, futureReserve, spendableHints, effectiveCandidates, entropy`.
- Candidate group count; methodNote chosen by method.

**Other**
- Progress: percentage strings.
- `recommendationSummary` (WebMCP): `{action: 'use_hint'|'challenge', card, expectedImmediatePoints (3 dp), solveProbability (6 dp), informationBits (3 dp), hintInformationBits}`.
- Other engine-adjacent helpers the UI uses:
  - candidate mass and probability (w/T);
  - populateRevealValues (mass per field value);
  - populateSpecialReveal (possible target.b / n given the guess);
  - searchCards (substring over normalised names[], top 24);
  - thresholdGain / solveReward for score bookkeeping in recordChallenge.
- Simulation worker: input `{cards compact, config, pool, rounds}`. Output progress/complete snapshots with `summary {count, meanSolved, CI, p10/p50/p90, completion + Wilson CI, matchedItems, hintsUsed, challengesUsed, distribution, diagnostics{solverCalls, cacheHits, exactCalls, depth3Calls, depth2Calls, fallbackCalls, hintDecisions, challengeDecisions}, elapsed}`.

## 8. Port notes

**Keeping results identical to the JS**
1. Keep ascending candidate order and first-occurrence grouping; JS Map insertion order must be reproduced with an index-preserving map or a small Vec.
2. Sorts must be stable, like JS. Rust `sort_by` is stable.
3. Do not put the epsilon comparator into `sort_by`: Rust ≥1.81 may panic on a non-total order. Quick-stage vectors are integer/T, so compare integer numerators (solveMass, newMatchMass) exactly. This is identical to the epsilon compare, because distinct values differ by at least 1/T ≫ 1e-11.
4. Linear-scan comparisons inside the solvers must keep the 1e-11 epsilon and first-max-wins, with the hint evaluated first.
5. `Math.log2` may differ from Rust `f64::log2` in the last bit. This affects info, infoApprox, the infoApprox top-8 ordering and info-mode sorting only in near-ties. Golden tests should use a tolerance.
6. Tier selection depends on expansion counts, which depend on JS memo identity (G in the key, duplicate actions expanded under distinct G). A `compat` mode must reproduce keys (d, h, c, known, matched, C, sorted G) and the exact visit order to get the same exact → depth3 → depth2 choice.

**Changes that keep every value the same (tier selection may still change)**
- Remove G from the memo key.
- Merge actions per state by feedback signature over the current subset.
- Pack keys into u64 (candidate subset as a bitmask over root candidates when N ≤ 64, known 6 bits, matched 6 bits, h, c, d), or use a 128-bit Zobrist hash of the subset built during partitioning.
- Partition with a counting sort into reusable arena buffers.
- Pack feedback into u16/u32 (`m | bcode<<6 | n<<10`).
- Prune actions whose cheap lexicographic upper bound is below the current best, e.g. first component ≤ P(solve now) + P(not)·[c−1>0 and d−1>0]·(1+cont_max). Order actions by solve probability and info to make the pruning bite.
- Iterative deepening that keeps the depth-2 result and reuses the transposition table.
- Quick stage via mask histograms or O(N+A) marginals; refine with a dense histogram of about 16k bins and a touched list.
- Candidate filtering via per-(field, value) bitsets.
- Run the engine in a Web Worker with a cancel message.

**Changes that alter behaviour (put behind a flag / v2)**
- Wider exact gate (N up to 12–16, h ≤ 11, c ≤ 8) with deterministic work budgets.
- Depth 3–4 search at mid-game.
- A leaf value better than ZERO.
- A continuation table W_r(h,c) learned by self-play instead of min(r,c,(c+0.61h)/3.9).
- Restricting 'all' pool actions to MD cards.
- Surfacing the value vector in the UI.

## Risks
- Exact parity with JS depends on floating-point details: the order of additions, no fused multiply-add, and Math.log2 versus Rust's log2 differing in the last bit. Entropy values and near-tie orderings (infoApprox top-8, info-mode sort) could differ occasionally, so golden tests need tolerances.
- Porting the epsilon comparator into Rust sort_by can panic in Rust ≥1.81, which detects comparators that are not a total order. Quick-stage sorts must compare integer numerators instead.
- Which tier runs (exact, depth 3, depth 2) depends on state counts, and those depend on the JS memo keys (guessed set in the key, duplicate actions expanded). A faster Rust memo will choose different tiers than JS unless a compat mode copies the JS key identity and visit order.
- WASM is typically 1.5–3x slower than native, and the engine currently runs on the main thread. Without a Worker and cancellation, large exact or depth-3 searches will still freeze the UI.
- Improving the model (leaf value, continuation table, wider exact gate) will intentionally change recommendations, so parity tests and improvement tests must be kept separate.
- I built the harness and probes from my reading of app.js, not from the UI itself. The DOM-driven paths (renderRecommendation text, WebMCP) were checked by reading the code only, never executed in a browser.

## Open questions
- Should the Rust port offer a strict compat mode (same JS memo keys, budgets and tier choice) for golden tests, and default to the faster memo that keeps values identical but lets the exact and depth-3 tiers succeed more often?
- Frame symmetry: in the game, does a Pendulum guess match a non-Pendulum target that shares a base frame (e.g. an effect-pendulum guess against a plain effect target)? The model says yes (b&b); the screenshots only confirm the case where the target is the Pendulum card.
- For the 4 cards whose level mask has two bits (nm = 3 or 4097), which value does the game reveal on a hint or level match? The model always uses n.
- Should 'all' pool mode limit guesses to Master Duel cards (wm>0), since other cards cannot be selected in game?
- Should v2 change the model (non-zero leaf value, a continuation table learned by simulation instead of min(r,c,(c+0.61h)/3.9), wider exact gate), or should the port keep the current model and only get faster?
- Should the three analysis views (points/solve/info) and their heuristic hint thresholds be kept, and should the configured reward points ever affect the strict recommendation?
- Should budgets be deterministic work units (reproducible across machines and good for tests) or wall-clock limits in the browser?
- Can the dead in-page simulation code in app.js be dropped, and should the simulation be rewritten on the Rust engine with a seeded RNG for reproducible results?
