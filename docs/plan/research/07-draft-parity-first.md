# Plan draft: Card Decoder, Rust/WASM rewrite (parity- and risk-first)

> Angle: the new tool must never give a worse or silently different recommendation than today's JS tool unless we choose that change on purpose. Every milestone ends with a tool that works and a rollback point you can name. All measurements below come from the reader reports. Anything not measured is marked as an open question.

## 1. Goals and non-goals

**Goals**
1. One Rust engine (`decoder-core`) that owns the game rules, candidate filtering, recommendation (quick, refine, exact, horizon), hint helpers and simulation. It compiles natively for the CLI and tests, and to `wasm32-unknown-unknown` for the browser.
2. **Parity first.** A faithful port reproduces the legacy JS outputs: values, chosen tier, `expandedStates`, chosen action and tie set. Faster variants must then match that faithful port's values, checked by differential tests.
3. Speed that does not change results. The report's targets are 3.2 s → about 75 ms native at the worst opening, and 11 s exact-limit failures → 5 ms or less. The engine never runs on the main thread.
4. Game-rule ambiguities become a `RulePolicy`. The legacy behaviour stays the default.
5. Online hosting as Cloudflare Workers static assets, the same pattern as ygo-small-world. The legacy tool stays reachable at `/legacy/` during the transition.
6. A Chinese README and user guide. Code comments in English.

**Non-goals (this rewrite)**
- No full-Rust UI (Leptos/Yew). The vanilla JS UI stays and calls a coarse WASM API.
- No WASM threads or COOP/COEP. We use a pool of Web Workers instead.
- No model changes by default: no new leaf value, no W-table, no bigger action sets, no change to the objective. These can only arrive later behind flags in `v2`, each validated by simulation.
- No rebuild of the card dataset in the critical path. The existing `cards-data.js` is the input until M7.

## 2. Principles that drive every milestone

| Principle | Mechanism |
|---|---|
| The oracle is the legacy code, not our reading of it | Golden fixtures come from the tagged legacy commit, and the harness runs the real `solver-core.js` plus app.js functions pulled out by AST (see §5.1). Nobody hand-copies code. |
| Three engine profiles | `Compat`: faithful port with the JS memo keys (guessed set G included), budgets and visit order, so tier selection matches the JS. `Fast`: values are proven identical, but tier selection can differ (exact and depth-3 succeed more often). `V2(flags)`: model changes. |
| Separate kinds of bugs | UI and reporting bugs are fixed directly, each with a regression test. Examples: the stale recommendation after a state change, a partial activity counted on cancel, `undefined` in the progress pulse, the worker leaked in `onerror`. Engine-semantics changes (for example the "lower bound" label, the myopic ZERO leaf, `all`-pool actions) go only behind a profile or flag. |
| Deterministic budgets | Budgets are counted in states and work units, never wall-clock time inside Rust, because `Instant` panics on wasm32. The hard stop is JS `worker.terminate()`. |
| Rollback at every layer | Git tags, the runtime `?engine=js\|wasm` and `?profile=compat\|fast` switches, `/legacy/` online, and `wrangler rollback`. |

## 3. Architecture

```
md-card-decoder/
├─ Cargo.toml  Cargo.lock  rust-toolchain.toml   # workspace, resolver 3; stable + wasm32 target
├─ crates/
│  ├─ decoder-core/src/
│  │  ├─ data.rs          # CardTable: SoA u8 value-id columns, nm u16, n u8, wm/wa u8, dicts, legacy index↔passcode
│  │  ├─ rules.rs         # RulePolicy → precomputed frame_compat[13], level_compat[15], reveal tables
│  │  ├─ feedback.rs      # Feedback(u16) = mask | frame_tid+1<<6 | n<<10 ; matches JS partition
│  │  ├─ state.rs         # PuzzleState, Log, Config (normalizeConfig port), v4→v5 migration
│  │  ├─ candidates.rs    # candidate filter (ascending order preserved)
│  │  ├─ metrics.rs       # quick (naive A·N AND O(N+A) histogram), refine entropy, infoApprox
│  │  ├─ value.rs         # Value([f64;3]), add/scale in JS order, cmp_eps (1e-11, scans only)
│  │  ├─ solver/{reference.rs, exact.rs, horizon.rs, actions.rs, tt.rs, bounds.rs}
│  │  ├─ recommend.rs     # pipeline + tier selection per Profile
│  │  ├─ hint_heuristic.rs# decideHintUsage/hintAdvice (analysis modes)
│  │  ├─ sim.rs  rng.rs   # mulberry32 (compat) + SplitMix/xoshiro (fast), per-episode seeds
│  │  └─ oracle.rs        # feature "oracle": brute-force expectimax for tests
│  ├─ decoder-wasm/       # cdylib; #[wasm_bindgen] Engine facade; serde-wasm-bindgen
│  └─ decoder-cli/        # recommend | sim (rayon) | golden-check | digest | replay-policies | bench
├─ harness/               # Node: extract-legacy.mjs, gen-golden.mjs, gen-pools.mjs, live-diff.mjs
├─ fixtures/golden/v1/    # committed; MANIFEST.json {legacy_commit:7dbc65b, data_sha256, generator_sha}
├─ legacy/                # frozen copy of today's dist/ (from M6), served at /legacy/
├─ web/                   # UI source: index.html, 404.html, _headers, app/*.js, engine-client.js, worker-entry.js
├─ tools/                 # build-wasm.mjs, build-site.mjs, existing pool/image tools
├─ public/                # GENERATED deploy dir (gitignored, whitelisted copy; no card-images*)
├─ docs/plan/             # PLAN.md, decisions.md, parity-report-<milestone>.md
└─ package.json  wrangler.jsonc  .github/workflows/ci.yml
```

Core Rust API (sketch):
```rust
pub struct Engine { table: CardTable, rules: Rules }
impl Engine {
    pub fn new(table: CardTable, policy: RulePolicy) -> Self;
    pub fn candidates(&self, s: &PuzzleState, pool: Pool) -> Vec<CardId>;
    pub fn feedback(&self, target: CardId, guess: CardId) -> Feedback;
    pub fn recommend(&self, s: &PuzzleState, cfg: &Config, mode: Mode,
                     opts: &RecommendOptions /* profile, budgets */) -> Recommendation;
    pub fn simulate(&self, cfg: &Config, pool: Pool, seed: u64,
                    episodes: Range<u32>, cancel: &dyn Fn() -> bool) -> SimStats;
}
pub struct Recommendation { recs: Vec<Rec>, advice: Advice, method: Method, depth: u8,
                            expanded_states: u32, value: Value, optimal: Vec<Action> }
```
WASM boundary: typed arrays go in (the columns are transferred), and small state objects move through serde-wasm-bindgen. Each call is coarse (one per click or batch). JS keeps DOM rendering, storage, images and WebMCP.

## 4. Milestones

Effort is in focused dev-days. Each milestone ends with a tag `mN-…`.

**M0: Freeze the baseline and build the golden harness (1.5 d).** No Rust yet.
- Tag `legacy-js-v1` on 7dbc65b.
- `harness/extract-legacy.mjs` parses `dist/app.js` with acorn. It pulls out the engine functions by name (candidateIndices … recommendationReason) and records a sha256 of each function's source text. It runs them with stubs for `state`, `CARDS`, `nextFrame` and the real `dist/solver-core.js`. This replaces the hand-copied `harness.js`.
- `gen-golden.mjs` writes these fixtures:
  1. **Rule digest.** For each guess, a 64-bit FNV hash of its feedback codes over all 8210 targets (one full 67M-pair pass).
  2. Candidate lists for about 200 fixed-seed states (the mulberry32 openings, mid-game and endgame generators from algo-bench).
  3. `calculateRecommendations` output for all four modes: quick top-40 with integer numerators, finalists, refine info, policy {method, depth, expandedStates, value, action, optimalActions}, and advice.
  4. `solveExact` and `solveRestrictedHorizon` cases in the `hcases` format (including the JS `LIMIT` outcomes).
  5. **Seeded simulations.** The harness sets `Math.random = mulberry32(seed)` and runs the worker's `activity()`: 20 seeds × 1 activity, about 9 min, run once.
  6. JS `solveExact` on 2k random small pools (§5.3).
- Acceptance: running the generator twice gives byte-identical fixtures, and a MANIFEST records the source hashes.
- Working tool: unchanged `dist/`. Rollback: n/a.

**M0.5: Put the legacy tool online (0.5 d).** Risk-first: the tool is online on day 1, and there is a baseline to fall back to.
- Copy `dist/` into `public/` through a whitelist (without `card-images*`, and with or without wallpapers per decision D6). Add `wrangler.jsonc` (no `main`, `404-page`), `_headers`, `404.html`, and a Chinese `package.json` like the sibling's. Run `npx wrangler deploy`.
- Acceptance: the site works at `*.workers.dev`, and `public/` holds fewer than 100 files.
- Rollback: this deploy is the rollback target for everything after it.

**M1: Workspace, data and rules (1.5 d).**
- Load `cards-data.js` by stripping the `window.CARD_DATA =` prefix and parsing with serde_json. Build `CardTable`, the `RulePolicy` defaults, the precomputed compatibility tables, `feedback()` and `candidates()`.
- Acceptance:
  - The rule digest equals the JS digest.
  - All candidate fixtures match exactly.
  - Property tests pass (§5.4 P1–P3).
  - `cargo clippy -D warnings` is clean.
- Working tool: JS UI unchanged, plus `decoder-cli digest`. Rollback: tag `m1`.

**M2: Faithful reference port, native (3 d).**
- `solver::reference` ports the JS exactly:
  - The memo key is (puzzle|d, h, c, known, matched, C, sorted G), with visit order, first-occurrence grouping and summation order preserved.
  - Expansions are counted the same way, and the state limit is thrown at 160000 (exact) or 24000 (horizon).
  - Tiers run in order: exact (gate), then depth 3, then depth 2, then B&B (kept, even though unreachable), then the quick[0] fallback.
- Port the quick, finalists and refine stages, the analysis-mode heuristics, and the simulation with mulberry32 in compat draw order.
- Acceptance:
  - method, depth and expandedStates are **equal** on every fixture.
  - Value vectors are within 1e-9.
  - The chosen action and equivalentChoices are equal.
  - Quick ordering is equal, compared on integer numerators. infoApprox/info near-ties within 1e-12 go on an allowlist, because `Math.log2` and `f64::log2` can differ in the last bit.
  - Seeded simulation trajectories are equal activity by activity.
- Working tool: `decoder-cli recommend --state exported.json` gives legacy-identical advice. Rollback: tag `m2`.

**M3: WASM drop-in inside the existing UI (3 d).**
- Toolchain:
  - Run `rustup target add wasm32-unknown-unknown`.
  - Install a pinned `wasm-bindgen` CLI that equals Cargo.lock (D10).
  - `tools/build-wasm.mjs` refuses to build on a version mismatch, and wasm-opt is optional with explicit feature flags.
- `engine-client.js` runs the engine in a Worker and loads it one of two ways:
  - https: `compileStreaming`;
  - file://: base64 plus a Blob worker, if D2 says file:// must keep working.
- The UI calls the engine through an adapter that has the same shape as the `calculateRecommendations` output. `?engine=js` keeps the old path; the default is `wasm` + `Compat`.
- Fix the UI bugs: bump the token in `render()`, add cancel, and never block the main thread.
- A `parity.html` page runs all golden fixtures inside the browser's WASM and shows a diff table.
- Acceptance:
  - parity.html is 100% green in Chrome and Firefox (plus Safari over https).
  - No main-thread task over 50 ms during a recommendation. Interim compat-tier cost in WASM is not measured; the estimate is 0.3–2.6 s for the exact-limit states, now inside the worker.
  - Deployed to `/` with `/legacy/` alongside.
- Rollback: `?engine=js`, or `wrangler rollback`.

**M4: Fast profile, same values (4 d).**
- In `solver::{exact,horizon}`:
  - Drop G from keys and prune no-op guesses.
  - Merge effective action classes by the O(1) projected signature, and let children inherit the parent's class list.
  - Evaluate depth-1 leaves in O(1) from histograms.
  - Compute quick metrics in O(N+A).
  - Use the factorized u16 codes with a counting-sort partition.
  - Keys: u128 when N ≤ 64; otherwise a 128-bit Zobrist hash plus a bounded open-addressing TT.
  - Iterative deepening that keeps the last completed depth instead of throwing work away.
  - Resource caps (c_eff, h_eff) **only** when the continuation is zero (final puzzle).
- Acceptance:
  1. At the same tier and depth, Fast values equal the reference values within 1e-9 on all fixtures, and Fast's optimal set equals the reference set mapped to classes.
  2. Fast, the reference and the oracle agree on 10k random pools.
  3. Perf gates:
     - native: worst opening, depth 3, 26 actions ≤ 100 ms (measured 72 ms);
     - exact gate states ≤ 5 ms;
     - WASM: p99 ≤ 250 ms (to be measured).
  4. `docs/plan/parity-report-m4.md` lists every fixture where Fast chose a different **tier** from Compat, with the value delta (expected: exact and depth 3 now complete).
- Default flips to Fast only after you review that report (D1). Rollback: `?profile=compat`.

**M5: Simulation on Rust (2 d).**
- Browser: a pool of `clamp(hardwareConcurrency−1, 1, 8)` workers sharing one compiled Module, with episodes sharded and per-episode seeds (results do not depend on the worker count). Chunked batches let cancel take effect; a cancelled partial activity is excluded. The seed is shown in the UI.
- Native: `decoder-cli sim --threads 20` with rayon.
- Acceptance:
  - Compat seeded runs equal the M0 fixtures.
  - Fast with 1000 activities at depth 2 runs natively in under 5 s (measured 3.7 s).
  - Browser: 500 activities at depth 2 in under 10 s with 4 workers.
- Rollback: the legacy worker behind `?engine=js`.

**M6: Web restructure, state v5, CI (3 d).**
- Code: `git mv dist legacy` (frozen); build `web/` from the legacy UI split into modules; `build-site.mjs` copies a whitelist into `public/` with hashed names; add `.assetsignore`.
- State v5 stores cards as `{passcode, data_sha}` instead of array indices. A shipped `legacy-index-map.json` (index → ids[0] for the current 8210 groups) lets v5 import v4 saves. Add JSON export and import, because moving from file:// to https empties storage.
- Wallpapers are gated by `matchMedia` and converted to WebP, or dropped (D6).
- CI: GitHub Actions runs fmt, clippy, tests, the WASM build, a Node parity smoke test and the artifact upload; deploy is manual or automatic (D9).
- Acceptance: a v4 save round-trips; `wrangler dev` smoke passes (wasm served as `application/wasm`, a missing image returns 404 and falls back to the card back); no `card-images` in `public/`.

**M7: Reproducible data pipeline (2 d, can run in parallel after M1).**
- `cargo xtask data fetch|build|verify` against pinned sources (the ygocdb md5 and the YgoMaster commit) recorded in `data/sources.lock.json`.
- Invariants are enforced by checks (§5.5). **Group order policy:** keep the legacy order and append new groups, so index-based fixtures and v4 import stay valid (D7).
- Emit a binary columnar core plus lazily loaded names and aliases.
- Acceptance: the rebuild gives 0 field and 0 wm mismatches against the legacy data (already shown possible in the audit); the golden fixtures still pass.

**M8: v2 model experiments (open-ended, each behind a flag).**
- Candidates:
  - root action sets built from the entropy upper bound;
  - a cost-based exact gate;
  - a fitted W[r][h][c] continuation table, labelled "近似";
  - a non-ZERO leaf;
  - `actions: ConsistentWithKnown`;
  - points in the objective.
- Each flag must win a native A/B of at least 10k activities with common random numbers, because mean solved sits near saturation at about 8.98/9. Each flag ships a Chinese release note.

**M9: Docs and cleanup (1 d).**
- Write the Chinese README and 使用说明 (hosted URL, offline zip, data update on Linux).
- Delete the dead JS code only in `web/`; `legacy/` stays frozen.
- Retire `/legacy/` after the event ends: D11.

## 5. Testing strategy

### 5.1 Golden tests (legacy is the oracle)
- `cargo test -p decoder-core --test golden` reads `fixtures/golden/v1`. Fixtures are regenerated **only** from the `legacy-js-v1` tag with `npm run golden:gen`, never from new code.
- Tolerances:
  - integer-derived quantities (masses, counts, expandedStates): exact;
  - value vectors: 1e-9;
  - entropy values: 1e-12 abs, plus an allowlist for order flips within 1e-12.
- Float discipline:
  - no FMA (rustc does not contract, and WASM has no relaxed-simd);
  - summation order `acc + child*p` with `p = (mass/T)/|U|`;
  - the epsilon comparator is used only in linear scans. Sorts use integer numerators or `total_cmp`, because Rust 1.81+ `sort_by` may panic on a non-total order.

### 5.2 Differential testing
- **JS vs Rust reference:** the fixtures, plus `harness/live-diff.mjs` (optional, needs Node), which runs random states through both engines side by side.
- **Reference vs Fast:** the same states, compared at a forced tier and depth.
- **Native vs WASM:** the same fixture suite through `wasm-bindgen-test` (Node) and `parity.html`.

### 5.3 Brute-force oracle on random small pools
`gen-pools` and a proptest strategy build pools of 4–10 synthetic cards:
- Field values come from tiny domains so collisions are forced.
- Pendulum combos, `nm` values with 2 bits, and '?' = −2 are included; the invariant `def = −3 ⇔ b = 32` is kept.
- Weights are 1–3; h ≤ 2, c ≤ 4.

`oracle::brute_value` is an expectimax with no memo and no pruning over **all** actions, re-guesses included, which tests the G-dominance proof directly. Three-way check: JS `solveExact` (the pools are pre-generated in M0) = Rust reference = Rust Fast = oracle.

### 5.4 Property tests (proptest)
- **P1** A card guessed against itself gives mask 63 for every group under every RulePolicy.
- **P2** The default policy is symmetric (b&b); the alternative policies are documented as asymmetric.
- **P3** The feedback partition equals the known-field and log filter; child masses sum to the parent's.
- **P4** Monotonicity (lexicographic ≥ within epsilon):
  - V(h+1, c) ≥ V(h, c);
  - V(h, c+1) ≥ V(h, c);
  - horizon V_{d+1} ≥ V_d.
- **P5** A single candidate (or one full-match class) with c ≥ 1 gives a direct challenge with value [1, 6−pop(matched), −1] (plus the continuation in the horizon), and no hint.
- **P6** Removing G, merging action classes and applying the resource caps each leave the value unchanged.
- **P7** Quick metrics: the O(N+A) version equals the naive A·N version exactly on the integer numerators.
- **P8** Entropy upper bound ≥ exact entropy for every guess.
- **P9** The level trap: matching uses `nm`, hints and reveals use `n`. A fixed fixture pins the case where the prototype went wrong (1/1032 E[M]).
- **P10** Seeded simulations are reproducible, and results are identical for 1, 4 and 8 workers.

### 5.5 Rule, data and perf gates
- Rule fixtures in TOML include the two official Zarc examples (13331639 against 83257450 and 90036274).
- `decoder-cli replay-policies` replays the user's real logs under each RulePolicy combination and reports which ones keep the candidate set non-empty. This is how we learn the true answers to R3, R5, R7 and R8 from play.
- Data invariants (counts, domains, `wm ≤ wa ≤ 255`, representative passcode is an MD card) fail CI.
- Criterion benches check native perf budgets with 2× headroom.

## 6. Algorithm and data-structure decisions

| Decision | Justification (measured) |
|---|---|
| Keep a faithful `reference` solver forever | It is the only way to get legacy tier parity (budgets depend on JS memo identity), and it serves as the test oracle for Fast. |
| Drop G from the memo key + prune no-ops (Fast) | Proof: a re-guess is strictly dominated. Values identical on 249/249 cases; 56 JS limit failures (about 11 s each) solve in ≤5 ms. |
| O(N+A) quick metrics from per-field histograms plus full-match lists (≤3 per guess) | 529 ms → 0.5 ms with identical numbers. |
| u8 SoA columns + factorized u16 feedback + counting sort | 2.2 ns per pair and 56 KB of tables; a full 8023² matrix (128 MB) is rejected. |
| Effective action classes via the O(1) projected signature | Endgames have 2.5× fewer actions (median) and up to 15×; the bijection was verified against brute force. |
| O(1) depth-1 leaf evaluation + iterative deepening that keeps completed depths | Worst opening at depth 3: 7.2 s JS → 72 ms native; no discarded work. |
| u128 keys (N ≤ 64) / 128-bit Zobrist + bounded TT | Removes O(N) string keys and bounds browser memory (the exact solve at n=20 reached about 1M states). |
| Resource caps only when the continuation is zero | Valid only then; 6–30× faster at large c. |
| Deferred to v2 / flags | Refinement-dominance pruning and lexicographic B&B (both need proofs), bigger action sets, W-table, SIMD. |
| Worker pool, no threads | Stable Rust, works on file://, cross-origin images unaffected. |
| RNG: mulberry32 (compat) / SplitMix-seeded xoshiro (fast) | Draw-order parity with the seeded JS simulation vs speed and worker-count independence. |

## 7. Deployment

- Workers static assets as in the sibling project: no `main`, `assets.directory: ./public`, `not_found_handling: 404-page`, `html_handling: auto-trailing-slash`.
- `_headers`: hashed `/engine/*` and `/assets/*` get one-year immutable caching; `/card-images/*` gets 30 days; no COOP/COEP.
- wrangler is a devDependency; npm scripts `build`, `dev`, `deploy`.
- The first deploy needs `npx wrangler login` or an API token.
- `public/` is a whitelisted build output, because wrangler ignores `.gitignore`.
- Downloads and installs use `/usr/bin/curl`; the anaconda curl on PATH has a broken CA bundle.
- Rollback: `wrangler rollback` to the previous version, and `/legacy/` until D11.

## 8. Risks

| Risk | Mitigation |
|---|---|
| The harness disagrees with the real UI | AST extraction plus source hashes. Also compare about 10 states by hand (or with Playwright, if you want it installed) between the real page and the harness. |
| Float or log2 last-bit drift flips near-ties | Integer numerators in sorts, tolerances, an allowlist, and a deterministic tie-break (JS action order, then card index). |
| Fast changes recommendations through tier selection | The parity report plus your sign-off before the default flips; Compat stays one click away. |
| Rule assumptions are wrong (R3: 2.2% of pairs; R5: 19.7% of level matches; R15) | RulePolicy switches plus the replay diagnostic; defaults stay legacy until evidence arrives. |
| Saves break (index-based, origin change) | v5 stores passcodes, the legacy index map imports v4, and JSON export/import exists. |
| The event runs during the migration (it ends 2026-10-02, plus 9 days from first login) | Legacy online at M0.5; `?engine=js`; engine-default flips only between play sessions. |
| WASM perf is unmeasured (estimated 1.3–2× native) | Measure in the M3 spike before committing to budgets. |
| wasm-bindgen CLI/crate mismatch (pacman 0.2.128 vs crates.io 0.2.129) | `=` pin plus a build-script check. |
| Posting a WebAssembly.Module to Blob workers under file:// is unverified per browser | Spike it in M3; fallback: post the bytes and compile in each worker. |
| Licensing of wallpapers and card images on a public URL | Off by default online (D6). |
| Missing data builder: new monsters cannot be added | M7. |

## 9. Decisions you need to make

| # | Decision | My default recommendation |
|---|---|---|
| D1 | After M4, make Fast the default (same values, deeper tiers)? Or keep Compat as the default for the whole event? | Fast after you review the parity report |
| D2 | Must opening the file directly (file://) keep working, or is hosted-only plus an optional PWA acceptable? | Keep file:// support |
| D3 | RulePolicy defaults for R3, R5, R7/R8, R9/R10, R15, and whether to run the replay diagnostic on your real logs | Keep the legacy defaults |
| D4 | Allow v2 model changes (action sets, W-table, exact gate, points in the objective)? | Only behind flags, after a ≥10k-activity A/B |
| D5 | Keep the analysis modes (points/solve/info) and their heuristic hint thresholds? | — |
| D6 | Images and wallpapers online: none, WebP thumbnails (w300 ≈ 183 MB), or R2? | No third-party wallpapers |
| D7 | Data pipeline in Rust xtask or Node, and the group-order policy? | Keep legacy order + append |
| D8 | Commit build artifacts, or publish an offline zip through GitHub Releases? | — |
| D9 | Worker name/URL (`md-card-decoder.<sub>.workers.dev`), manual vs CI deploy, and whether the repo is on GitHub with Actions | — |
| D10 | wasm-bindgen: pin 0.2.128 (pacman) or `cargo install --root .tools` 0.2.129? | — |
| D11 | When to retire `/legacy/` and the JS engine path | After the event ends and one clean week on Fast |
| D12 | Restore the simulation metrics that the docs describe but the worker dropped (高价值题全通率, 后段匹配收益), or fix the docs? | — |