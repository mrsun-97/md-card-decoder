# Plan draft: Rust/WASM rewrite of md-card-decoder, algorithm-first

> Scope: planning only. No repo file changes. All numbers below come from the reader reports: JS figures are from Node 26 and native Rust figures from the scratchpad prototypes. The WASM figures are estimates at 1.3–2x native.

## 0. TL;DR
- The engine is slow because of four things: `guessed` is in the memo keys, so exact search blows up; there is an O(8023·|C|) quick pass; depth 3 gets thrown away every time it hits its budget; and everything runs on the main thread. The fix is not "Rust is faster" on its own.
- The Rust prototype already reproduces all 249 JS value vectors. It runs the worst opening in about 75 ms instead of about 3.2 s, final-puzzle exact solves in 5 ms or less instead of failing after 7–57 s, and a whole activity in 0.33 s instead of about 21 s.
- Proposed layout: one pure `decoder-core` crate with no wasm dependencies, a thin `decoder-wasm` facade, a native `decoder-cli` for benchmarks, golden tests, simulation and table fitting, and a static-assets Cloudflare Worker copied from `ygo-small-world`.

## 1. Goals and non-goals
**Goals**
1. A Rust engine that gives the same values as the JS solvers within 1e-9, with every speed-up proven to preserve values and checked against a naive reference.
2. Browser latency: recommendation p50 ≤ 50 ms and p99 ≤ 250 ms in a Worker, with a hard 300 ms anytime cutoff and never blocking the main thread. Final-puzzle exact solve < 200 ms.
3. Simulation: 500 browser activities at depth 2 in under 10 s with 4 Workers (about 3 h in JS today). Natively, 10k activities in under 1 min, which is what telling policies apart needs.
4. Honest labels for results: exact / optimal within a restricted model / heuristic.
5. Online deployment plus the file:// double-click path. Chinese README.

**Non-goals for v1**: a Rust UI framework (Leptos/Yew); WASM threads (they need nightly Rust plus COOP/COEP); reproducing the JS tier choice (exact → d3 → d2), which depends on JS memo identity; changing the model's objective.

## 2. Architecture
```text
md-card-decoder/
├─ Cargo.toml              # workspace, resolver="3"; [profile.release] opt-level=3, lto="fat", codegen-units=1, panic="abort"
├─ rust-toolchain.toml     # stable + wasm32-unknown-unknown, rustfmt, clippy
├─ crates/
│  ├─ decoder-core/        # pure std, deps: rustc-hash, libm, smallvec, serde (feature), proptest/criterion (dev)
│  │  ├─ src/data.rs       # CardTable SoA, dictionaries, bin/legacy loaders, passcode map
│  │  ├─ src/rules.rs      # RulePolicy, compat tables, feedback code, match_mask
│  │  ├─ src/bits.rs       # BitSet129 (8210 groups), per-(field,value) bitsets
│  │  ├─ src/state.rs      # PuzzleState, Log, filter/apply_reveal/apply_challenge
│  │  ├─ src/value.rs      # Val=[f64;3], cmp_lex (EPS 1e-11), exact integer sort keys
│  │  ├─ src/kernel.rs     # partition (dense histogram + touched list + counting sort), field histograms
│  │  ├─ src/metrics.rs    # O(|C|+U) quick metrics, points kernel, refine entropy, entropy UB
│  │  ├─ src/actions.rs    # effective-class dedup, no-op filter, root action-set builders
│  │  ├─ src/tt.rs         # keys (u64/u128/Zobrist-128), transposition table
│  │  ├─ src/solver/{horizon.rs, exact.rs, reference.rs, bounds.rs}
│  │  ├─ src/cont.rs       # ContinuationModel::{Legacy{0.61,3.9}, Table(W)}
│  │  ├─ src/recommend.rs  # pipeline, modes, budgets, proof labels, tie sets
│  │  ├─ src/sim.rs, src/rng.rs  # activity sim, SplitMix64/xoshiro256++, mergeable SimStats
│  │  └─ benches/ tests/
│  ├─ decoder-wasm/        # cdylib; wasm-bindgen "=0.2.x" (pinned), js-sys, serde-wasm-bindgen
│  ├─ decoder-cli/         # `mdd bench|golden|sim|fit-cont|compare`, rayon
│  └─ xtask/               # `cargo xtask data fetch|build|verify` (cards.bin from ygocdb+YgoMaster)
├─ tests/golden/           # JSONL fixtures exported from the legacy JS (committed)
├─ tools/                  # export-golden.mjs, build-wasm.mjs, build-site.mjs, images.mjs, existing pool tools
├─ web/                    # UI source (today's dist/ refactored; engine-client.js, worker-entry.js)
├─ legacy/                 # frozen copy of old dist/*.js for golden export (deleted after M5)
├─ public/                 # GENERATED, gitignored: wrangler assets dir + offline zip content
└─ package.json, wrangler.jsonc, .github/workflows/ci.yml
```

## 3. Core data structures
```rust
pub type Gid = u16;                           // 8210 groups
pub struct CardTable {                        // SoA, u8 value ids, ~74 KB
    frame: Box<[u8]>, attr: Box<[u8]>, race: Box<[u8]>,
    lvl: Box<[u8]>,    // nm-id (match semantics; 15 ids, 33 if kind-aware)
    hint_n: Box<[u8]>, // printed n (hint/reveal semantics) — NEVER merge with lvl
    atk: Box<[u8]>, def: Box<[u8]>, wm: Box<[u8]>, wa: Box<[u8]>, dict: Dicts,
}
pub struct Rules {                            // built from RulePolicy at load; zero hot-loop cost
    ft: [[u8; 16]; 16],   // [g_frame][t_frame] -> 0 | 1+t_frame  (reveal included)
    lt: [[u8; 64]; 64],   // [g_lvl][t_lvl]   -> 0 | 1+n
}
#[derive(Clone, Copy, PartialEq, Eq, Hash)] pub struct Fb(u16);   // factorized code < 3360
#[inline] fn fb(&self, g: Gid, t: Gid) -> Fb  // ft*240 + (a==)*120 + (r==)*60 + lt*4 + (atk==)*2 + (def==)
pub struct FullMatch { off: Box<[u32]>, ids: Box<[Gid]> } // ≤3 per guess, 8149 pairs total
pub type Val = [f64; 3];                       // (E solved, E first-time matches, −E actions)
pub enum Proof { ExactFinal, HorizonOptimal { depth: u8, n_actions: u16, cont: ContKind }, Heuristic }
pub trait Budget { fn tick(&mut self, work: u32) -> core::ops::ControlFlow<()>; }
```
- `RulePolicy { frame: Intersect|PendRequiresPend, level: Numeric|SameKind, alt_level: BothSides|TargetOnly, level_reveal: Rule|Printed, unknown_stat, actions: Any|ConsistentWithKnown }` defaults to today's behaviour. Its only effect is to rebuild `ft`/`lt` and the equivalence classes.
- Group order in `cards.bin` v1 stays the same as legacy `cards-data.js`, because saved logs store indices. State v5 stores passcodes plus a data hash, and a v4 importer migrates by index.

## 4. Algorithms and complexity
### 4.1 Partition kernel
For a guess g and a candidate slice, compute `fb` per target, add the weight into `hist[3360]: u32`, and push new codes onto a `touched` list. Touched order equals the JS first-occurrence order, which keeps summation close to JS. A counting sort then writes child slices into an arena `Vec<u16>` of local indices, ascending and stable.
- Cost: O(|S|), measured at 2.2 ns per pair native. That is 2.8x faster than JS typed arrays and 35x faster than the JS string-key Map.
- We factor the table instead of materialising the full 8023² matrix: about 56 KB of tables versus 128 MB.
- SIMD (`+simd128`) could give at most about 2x. Deferred until profiling says otherwise.

### 4.2 Quick stage: O(|C|·U) → O(6|C| + U·~30)
Build per-field weighted histograms over C once. For each guess g:
- P(frame match) = Σ mass_frame[tid] over the ≤13 compatible tids. Level works the same way (≤15). The other fields read `mass_f[g.v]` directly.
- solveMass = Σ w over FullMatch(g) ∩ C, using BitSet129 membership.
- newMatchMass = Σ_{f∉matched} matchMass_f. infoApprox = Σ_f Hb(matchMass_f/T), over all 6 bits as in JS.
- Result: 0.13–0.50 ms native against 7–586 ms in JS, with identical numbers.
- Sorting uses integer keys `(solveMass desc, newMatchMass desc, gid asc)` with `sort_unstable_by_key` on a total order. This is identical to the epsilon compare because values differ by at least 1/T ≫ 1e-11, and it avoids the Rust ≥1.81 panic on non-total comparators.
- The analysis modes (points/solve) need the exact 64-bin mask histogram per guess, because milestones are nonlinear in popcount. That keeps the SoA O(U·|C|) kernel: 110 ms native at N=5000, run in the Worker.
- Refine (≤340 finalists) computes exact entropy from the dense histogram. `libm::log2` is used everywhere so native and wasm agree bit for bit.

### 4.3 Keys and transposition table
- Drop `guessed` from every key. Proof sketch: a previously guessed card is single-cell on every descendant, and its mask is ⊆ matched. Its value is therefore [0,0,−1] + V(s, c−1, d−1), which is strictly below V(s, c, d) because V is monotone in c and d and `stop` exists (in the exact tier some candidate always has positive solve chance). Verified: 93/93 exact values and 156+42 horizon values unchanged.
- `known` stays in the key because it drives the hint distribution. `matched` stays because it drives rewards.
- Key choice by search size:
  - Exact search (n ≤ 64): `u128 = set:u64 | known:6 | matched:6 | h:5 | c:6`, collision-free.
  - Horizon search: 128-bit Zobrist of the local indices, XOR-accumulated during partition at no extra cost, packed with `(d, h, c, known, matched)`.
- Table: `FxHashMap<Key, Entry>` in v1. Later, a fixed-size open-addressing table (2^18 entries in the browser, 2^20 native) with depth-preferred replacement. Eviction only costs recomputation. The chance of a 128-bit collision is about 1e-27 at 1e6 states.

### 4.4 Action reduction (all value-preserving)
1. **Effective equivalence classes.** The signature is computed in O(1) per guess from C's presence masks: the frame-compat set ∩ present tids, the level-compat set ∩ present ids, the value id or MISS for each equality field, with fields already in `matched` removed, plus FullMatch(g) ∩ C. Hash it to u128, then sort and dedup (0.12–0.20 ms for 8023 guesses). Children inherit the parent's representative list, because equivalence only coarsens on subsets. Effect: endgames drop about 2.5x at the median and up to 15x (n=2: 75 → 5); openings drop 1.7x at most.
2. **No-op pruning.** Remove any single-cell outcome with an empty effective label and no solve. This covers all guessed cards.
3. **Behind a flag (medium–high proof risk):** same-partition Pareto and refinement dominance, 1.2–2x more.

### 4.5 Restricted horizon
- `fn visit(&mut self, s: NodeRef, d: u8) -> Val`. The hint is evaluated first and the first maximum wins, as in JS. A linear-scan `cmp_lex` with EPS=1e-11 is used only inside solvers.
- **Closed-form leaf at d=1.** Every child is worth ZERO, so a challenge is worth `[Ps·(1+contS(h,c−1)), E[gain] + Ps·contM, −1]`, computed from the node's field histograms. A hint is worth [0,0,−1] < stop and is skipped. This removes about 95% of partition work (5x faster than the faithful port).
- Combined measurements: worst opening (n=5000) depth 3 in 72 ms versus 7.2 s JS uncapped; typical openings 10–40 ms; depth 2 in 0.4–2.5 ms.
- **Iterative deepening** (d = 1, 2, 3, then 4 if budget allows). The TT is kept across iterations, root move ordering comes from the previous iteration, and on budget exhaustion the last *completed* depth is returned. Nothing is thrown away.
- **Root action sets:**
  - `Legacy` is the JS rule: quick top 18 ∪ infoApprox top 8 ∪ weight top 8.
  - `Informative(K)` is v2: legacy ∪ effective reps ∪ exact top-K entropy. The top-K comes from lazy selection with the upper bound H ≤ Σ_f H(response_f): stop once the K-th exact value ≥ the next bound. That takes 84–206 partitions for K=64 instead of 8023, about 1.3 ms at n=5000.
  - Measured effect at n=2258: depth-3 E[S] goes 0.296 → 0.611 as K grows (26 → 140 actions, 39 → 724 ms native). A better action set is worth more than extra depth.

### 4.6 Exact tier (final puzzle, zero continuation)
- The set is a u64 bitmask over root-local indices. A per-root table `fbl[a][i]: Fb` (effective actions × n) is built once, and children are built by iterating set bits.
- Resource caps: `c_eff = min(c, |S| + 6 − |matched|)` and `h_eff = min(h, #unknown)`. Valid **only** with zero continuation, so the type system enforces it: caps live in `ExactFinal` and never in horizon code. Measured 6–30x faster, with identical values.
- Gate: replace the legacy gate (n≤7, h≤4, c≤6) with a cost estimate from (n, effective actions, h_eff, c_eff) and a work budget. Always run the horizon first; exact overrides it only when it completes. Measured:
  - n ≤ 7: ≤ 5 ms at any resources.
  - n ≤ 13: ≤ 350 ms.
  - n ≈ 15 with h4 c6: 2.4–4 s, which is why item 4.7 exists.

### 4.7 Bounds (M8, behind a flag)
Lexicographic branch-and-bound with Wordle-style lower bounds on E[actions]: with weights sorted, at most kmax targets are solvable at depth 1, K·kmax at depth 2, and so on. This matters only when the first two components are saturated, which is exactly the slow n≈15 band. The suggested top-(c·kmax) E[S] bound is **not admissible**, so it is banned. Every bound gets a `debug_assert!` against the reference solver.

### 4.8 Continuation model
- `Legacy` = min(r, c, (c+0.61h)/3.9)·[1, 6, 0], kept for parity.
- `Table(W[r][h][c] → (W_S, W_M))` is v2: 9×12×37 f32 pairs (32 KB, `include_bytes!`). It is fitted offline by `mdd fit-cont` with approximate policy iteration: common random numbers, backward over r, isotonic in h/c/r, 2–4 rounds of about 3 min each on 20 threads at depth 2. The UI must label it 近似 (approximate).
- Optional later: a learned leaf value to replace ZERO at the cutoff.

### 4.9 Simulation and parallelism
- `simulate(cfg, pool, EpisodeRange{first,count}, seed, policy, budget) -> SimStats`, with per-episode seed `splitmix64(seed ^ idx)`.
- SimStats uses integer accumulators (sum solved, sum², histogram, completion count), so results are identical at 1, 4 or 20 threads and between native and wasm (libm).
- Each worker has a decision cache keyed by the 128-bit state hash, bounded.
- Native: rayon (measured 1000 activities at depth 2 in 3.7 s wall; 400 at depth 3 in 12.8 s).
- Browser: a pool of N = clamp(hardwareConcurrency−1, 1, 8) classic Workers sharing one compiled `WebAssembly.Module`. Work goes out in chunks of 50–200 ms so cancel messages are seen; hard cancel is `terminate()` plus respawn. Fix the JS bug where a cancelled partial activity was counted.

### 4.10 Expected performance (native measured / WASM estimated)
| workload | JS today | native | WASM |
|---|---|---|---|
| quick metrics (all guesses) | 7–586 ms | 0.13–0.5 ms | 0.3–1 ms |
| full recommendation, worst opening | ~3.2 s (main thread) | ~75 ms | 110–150 ms |
| depth 3, 76–80 actions, n≈2200 | n/a | 230 ms | 350–460 ms |
| exact, n≤7, any resources | 0–57 s / fails 18 of 34 | ≤5 ms | ≤10 ms |
| one activity, depth 2 / 3 | 20.7–26 s | 0.036 / 0.33 s | 0.06 / 0.5–0.66 s |

## 5. Keeping exactness and proof claims
- `SolverOpts { dedup, noop_prune, leaf_closed_form, caps, bnb, dominance }`. Every flag defaults to value-preserving except `bnb` and `dominance`, which default off until proven.
- `solver::reference` is the naive JS-faithful solver (guessed in key, string-equivalent semantics). CI runs property tests showing that every flag combination matches it on random small pools.
- Result labels:
  - `ExactFinal` → "已证明最优" (proven optimal).
  - `HorizonOptimal` → "受限模型最优（深度 d、N 个候选动作、启发式续值）" (optimal within the restricted model: depth d, N candidate actions, heuristic continuation).
  - Retire the incorrect "当前可行下界" (feasible lower bound) label.
- Tie sets are computed with an explicit tolerance and a deterministic tie-break (lowest gid), and shown as 任选其一 (pick any one).

## 6. Milestones
Effort is in engineer-days.

| # | Deliverables | Acceptance | Effort |
|---|---|---|---|
| M0 Spike | workspace, toolchain, pinned wasm-bindgen, hello wasm in a Blob worker (file://) and in `wrangler dev` | loads in Chrome and Firefox over file:// and http; build script rejects a CLI/crate version mismatch | 0.5–1 |
| M1 Golden export | `tools/export-golden.mjs` built from scratchpad `harness.js`/`hcases`: match masks, quick, refine, horizon, exact, recommend fixtures | fixtures committed; covers the 34+56 exact states, 156+42 horizon states, the nm/n trap and Zarc examples | 1 |
| M2 Data + rules | `data.rs`, `rules.rs`, `xtask data build` (legacy order) | match_mask digest over all 8023×8210 pairs equals JS; self-match = 63 under every policy | 1.5–2 |
| M3 Kernels + metrics | kernel, quick O(\|C\|+U), points kernel, entropy, bitset filter | integer numerators exact, entropy within 1e-12; criterion baselines recorded | 1.5–2 |
| M4 Solvers | horizon + exact + reference + TT + opts | 249/249 values within 1e-9; 56 former JS-limit cases < 10 ms; proptest passes | 3–4 |
| M5 Pipeline | `recommend()` with modes, iterative deepening, Budget, Proof, ties; `Legacy` action set by default | same recommended action as JS on fixtures, up to tolerance-ties; p99 < 250 ms native-equivalent | 2 |
| M6 WASM + UI | `decoder-wasm` Engine, WorkerPool, engine-client; remove solver-core.js and the in-page solvers; bump token on render; fix onerror leak | all UI features still work; no main-thread task > 50 ms during recommend; file:// and https both work | 3–4 |
| M7 Simulation | sim.rs, `mdd sim`, browser pool, seeded runs | same stats across thread counts and native/wasm; 500 × depth 2 < 10 s in the browser | 1.5–2 |
| M8 Model v2 (optional) | Informative(K) action sets, cost-based exact gate, W table, B&B | ≥ +X in E[solved] or P(9/9) over 10k CRN activities versus Legacy; flags in the UI | 4–6 |
| M9 Deploy + docs | build-site, wrangler, `_headers`, CI, Chinese README, 使用说明 update | `npm run deploy` works; CI green | 1.5 |

Total: about 20–27 days. M8 can come after launch.

## 7. Testing strategy
1. **Golden parity** (tests/golden, JSONL): values within 1e-9, integer masses exact, entropy within 1e-12 absolute. We check solver-level parity (same inputs, same values), not tier-selection parity.
2. **Reference equivalence**: proptest pools of 8–40 random groups with random reveals, h ≤ 4, c ≤ 6; all `SolverOpts` combinations against `reference`.
3. **Rule invariants**: frame/level compat tables versus brute force; partitions agree with the known-field filter; Tzolkin/赤龙 collision; the 7 rule-level cards.
4. **Bound admissibility**: `debug_assert!` that the entropy upper bound is ≥ exact entropy and the B&B bound is ≥ the true value, in debug and CI builds.
5. **WASM smoke**: `node --test` loads the wasm and compares against native output on fixtures, which must be bit-identical.
6. **Performance**: criterion benches on `states.json` (from the scratchpad) and `mdd bench` reports. These are not a CI gate because timings are noisy, but PRs record them.
7. **Simulation**: determinism tests, plus A/B at 10k activities with common random numbers (differences are small: 8.98 of 9 solved).

## 8. Deployment (brief)
- Same setup as ygo-small-world: `wrangler.jsonc` with no `main`, `assets.directory: ./public`, `404-page`, `auto-trailing-slash`, and Chinese comments.
- `public/` is built from a **whitelist**, because wrangler does not read `.gitignore` and must never upload `dist/card-images*`.
- Hashed `engine/*` files get `Cache-Control: immutable`. Wrangler already serves `.wasm` as `application/wasm`. No COOP/COEP.
- The file:// build loads a base64 wasm script and starts a Blob worker from embedded source, then posts the compiled Module.
- CI: GitHub Actions (fmt, clippy, test, wasm build, node tests), then `cloudflare/wrangler-action@v4`. Workers Builds has no Rust, so it is not used.
- Build commands: `rustup target add wasm32-unknown-unknown`, `cargo install wasm-bindgen-cli --version <pinned> --locked --root .tools`, `npm i -D wrangler esbuild`, `npm run build && npx wrangler dev`.

## 9. Risks
- **Semantic drift**: nm versus n (the prototype hit this bug once), frame reveal bits, hints over unknown fields only. Golden tests are required before any optimisation lands.
- **Unconfirmed game rules** (pendulum guess against a non-pendulum target: 2.2% of pairs; level/rank/link cross-match: 19.7% of level matches; auto-filtered action set) could bias every recommendation. `RulePolicy` plus a log-replay consistency check reduces this.
- **Float differences**: JS `Math.log2` versus libm, and summation order, can resolve near-ties differently. Handled with tolerance-based ties.
- **Continuation-dependent shortcuts**: caps or bounds applied where the continuation depends on (h, c) would silently change values. Kept separate by type.
- **WASM speed** is still an estimate (the wasm32 target is not installed). Measure in M0/M4.
- **Browser memory**: the exact search at n=20 reached about 1M states, so the TT must be bounded.
- **Changed advice**: the v2 model changes recommendations, and users will notice. It needs flags, release notes and simulation evidence.
- Hosting card art or fan-art wallpapers publicly raises copyright exposure.

## 10. Decisions for the user
1. **Parity vs improvement**: ship v1 with `Legacy` action sets and continuation (recommended), then v2 behind a toggle? Or adopt Informative(K) and uncapped depth 3 immediately?
2. **Compat tier mode**: skip reproducing JS memo identity and tier choice, and test values only (recommended)?
3. **Budgets**: deterministic work units by default with a wall-clock safety cap (recommended), or wall-clock only?
4. **Exact gate**: cost-based (recommended) or the legacy n≤7/h≤4/c≤6 rule?
5. **Objective**: keep (E solved, E first-time matches, −E actions)? Or should P(9/9) or the configured points ever affect strict mode?
6. **Rule policy**: which defaults, and should players be able to switch R3, R5, R7 and R15 in the UI?
7. **'all' pool**: restrict guesses to MD cards (wm>0)?
8. **Keep file:// support** (doubles the loader work), or hosted plus a PWA offline cache?
9. **Card images and wallpapers online**: none, a WebP thumbnail set (~9k files), or R2?
10. **Data pipeline**: Rust xtask rebuild from ygocdb (recommended), or keep transforming the frozen cards-data.js? Keep the legacy group order for v1?
11. **Analysis modes** (points/solve/info with heuristic hint thresholds): keep, simplify or drop?
12. **Build artifacts**: commit them, or publish an offline zip through GitHub Releases? Worker name/URL, and whether CI deploys automatically.