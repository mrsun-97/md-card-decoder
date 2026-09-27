# Implementation Plan Draft: Card Decoder Rust/WASM Rewrite (product- and deploy-first)

Suggested path in the repo when merged: `docs/重构计划.md`, with a dated log section at the bottom so the workflow can be traced. The event ends 2026-10-02 22:59, seven days from today, and the plan is ordered with that in mind.

## 1. Goals and non-goals

**Goals**
1. One static bundle that works in three places:
   - online at `https://md-card-decoder.<sub>.workers.dev`;
   - offline, by double-clicking `index.html` in a zip (`file://`);
   - locally through `wrangler dev`.
2. The engine is Rust (`decoder-core`) compiled to WASM. It runs in Web Workers so the main thread never blocks. Measured targets: a recommendation takes p50 ≤ 50 ms and p99 ≤ 250 ms. 500 simulated activities at depth 2 finish in under 10 s (JS today takes about 3 h).
3. Recommendations keep the current model's values. Where the JS finishes, Rust values match it within 1e-9. Model changes such as a new continuation or larger action sets ship only behind a flag, as v2.
4. Development is Linux-first: npm scripts plus cargo, no `.cmd`. CI builds and deploys. The data pipeline can be rebuilt from upstream.
5. The README is regenerated in Chinese. Code comments stay in English, and config files use Chinese comments as in ygo-small-world.

**Non-goals (v1)**
- No full-Rust UI (Leptos, Yew or Dioxus).
- No WASM threads or COOP/COEP headers.
- No server-side compute. The Worker only serves static files and has no `main`.
- No hi-res card art online. No change to the objective vector.

## 2. Architecture

### 2.1 Repo tree (after M4)
```text
md-card-decoder/
├─ Cargo.toml  Cargo.lock  rust-toolchain.toml   # workspace, resolver 3; stable + wasm32 target
├─ crates/
│  ├─ decoder-core/   # pure Rust, no wasm deps
│  │  └─ src/ data.rs rules.rs state.rs candidates.rs quick.rs partition.rs
│  │          value.rs exact.rs horizon.rs recommend.rs sim.rs rng.rs search.rs error.rs
│  ├─ decoder-wasm/   # cdylib facade: wasm-bindgen "=0.2.x", serde-wasm-bindgen 0.6, js-sys
│  └─ decoder-cli/    # `mdcd`: sim (rayon), bench (criterion), golden, data {fetch,build,verify,diff}
├─ web/               # UI source (today's dist/, split)
│  ├─ index.html 404.html _headers .assetsignore styles.css presets.js card-back.jpg
│  ├─ js/ store.js render-*.js dialogs.js images.js storage.js webmcp.js engine-client.js
│  └─ worker/engine-worker.js
├─ legacy/js/         # frozen solver-core.js + app.js: the golden oracle, never deployed
├─ tests/ golden/*.jsonl  node/*.test.mjs  e2e/*.spec.mjs
├─ tools/ build-wasm.mjs build-site.mjs export-golden.mjs images.mjs doctor.mjs
│         update-master-duel-pool.mjs refine-master-duel-pool.mjs      # kept until M6
├─ data/ sources.lock.json summary.json raw/(gitignored)
├─ images/wrangler.jsonc   # optional second static Worker for thumbnails (see §6)
├─ public/            # GENERATED, gitignored = wrangler assets dir = offline zip content
├─ .tools/            # project-local wasm-bindgen / wasm-opt (gitignored)
├─ package.json wrangler.jsonc .github/workflows/{ci.yml,release.yml,data-refresh.yml}
└─ README.md (zh)  docs/{规则与模型.md,算法说明.md,重构计划.md,资源价值实验报告.md,壁纸来源与授权.md}
```
`dist/` goes away. `build-site.mjs` copies a whitelist from `web/` into `public/`, so local `card-images*` folders can never be uploaded. This matters because wrangler ignores `.gitignore`.

### 2.2 Frontend: keep vanilla JS, refactor
- **Why:** 104 KB of working Chinese UI already exists, and the engine boundary is only about 10 calls. Classic scripts keep `file://` working. `// @ts-check` with the wasm-bindgen `.d.ts` plus `tsc --noEmit` gives type checks without a TS build.
- **Bundling:** esbuild produces IIFE bundles, `app.<hash>.js` and `engine-worker.<hash>.js`. No ES modules at runtime, because they break `file://`.
- **Remove:** the whole JS solver, the dead in-page simulation (app.js:1423-1658) and duplicated rule code (matchMask exists in 5+ places).
- **UI bugs fixed during the port:**
  - A stale recommendation shows after a state change (fix: a generation token that `render()` increments).
  - The worker `onerror` path leaks the worker.
  - Cancelling a simulation counts the partial activity.
  - The progress pulse shows "undefined".
  - The candidate table silently truncates to 100 rows.
  - Wallpapers keep preloading on mobile.
  - Search runs on every keystroke with no debounce, no NFKC folding and no display of the matched alias.

### 2.3 WASM integration: two engine tiers
| Tier | Where | Calls | Why |
|---|---|---|---|
| Sync engine | main thread, one wasm instance | `candidates`, `reveal_histogram`, `special_reveal_options`, `apply_reveal`/`apply_challenge` validation, `search` | 1–5 ms each; one source for the rules instead of JS copies |
| Interactive worker | 1 Worker | `recommend` (iterative deepening, streams best-so-far) | never blocks the UI |
| Sim pool | N = clamp(hardwareConcurrency−1, 1, 8) Workers | `simulate_batch(seed, first, count)` | activities are independent, so there is no shared memory |

The main thread compiles the wasm once, `postMessage(module)`s it to every worker, and each worker calls `initSync({module})`. The card columns go to each worker as a transferred `ArrayBuffer`.

Cancelling calls `worker.terminate()` and respawns the worker from the cached Module in milliseconds. A synchronous wasm call cannot see a `cancel` message, so this is the only real way to stop one. The UI keeps the last streamed result, for example the finished depth-2 result while depth 3 was still running.

```rust
// decoder-core
pub fn candidates(t: &CardTable, s: &PuzzleState) -> Vec<u32>;
pub fn recommend(t: &CardTable, cfg: &Config, s: &PuzzleState, mode: Mode,
                 budget: WorkBudget, on_progress: &mut dyn FnMut(&Partial)) -> Result<Recommendation, EngineError>;
pub fn simulate_batch(t: &CardTable, cfg: &Config, pool: Pool, policy: &PolicyCfg,
                      base_seed: u64, first: u32, count: u32) -> SimStats; // mergeable: sums, sumsq, histograms, counters
// decoder-wasm
#[wasm_bindgen] impl Engine {
  #[wasm_bindgen(constructor)] pub fn new(cards_bin: &[u8]) -> Result<Engine, JsError>;
  pub fn recommend(&mut self, state: JsValue, mode: &str, work: u32, progress: &js_sys::Function) -> Result<JsValue, JsError>;
  pub fn simulate_batch(&mut self, cfg: JsValue, pool: u8, seed_hi: u32, seed_lo: u32, first: u32, count: u32) -> Result<JsValue, JsError>;
}
```
**State ownership:** the state stays in JS (it already has a schema and persistence), and Rust is stateless per call. Error codes map back to the existing Chinese toast strings.

### 2.4 Offline vs online loading (one build, chosen at runtime)
| | `https:` | `file:` |
|---|---|---|
| wasm | `WebAssembly.compileStreaming(fetch('engine/decoder_bg.<h>.wasm'))` | classic `engine/wasm-b64.<h>.js`, then `WebAssembly.compile(bytes)` |
| workers | `new Worker('engine/engine-worker.<h>.js')` (classic) | Blob URL made from embedded `ENGINE_WORKER_SOURCE` (generalises today's trick) |
| card data | classic `data/cards.<h>.js` (base64 bin + zh names), same in both modes | same |
| aliases | injected lazily on first search focus, via `data/aliases.<h>.js` | same |

Data always ships as classic `.js` for three reasons: it keeps one code path, Cloudflare edge-compresses JS, and it avoids the risk that `application/octet-stream` is sent uncompressed. The base64 wasm file is only downloaded in `file://` mode.

Storage origins differ between `file://` and https, so M3 adds **export/import JSON**. The v5 export stores passcodes plus the data hash, and import also accepts v4 files.

Optional in M7: a PWA service worker (https only) for offline use after the first visit.

## 3. Algorithm and data-structure decisions

All of these come from measurements in the reader reports.

| Decision | Justification |
|---|---|
| u8 column-per-field `CardTable` + value-id dictionaries; frame/level compat tables (`u16`/`u64` rows); feedback code `u16` | 2.2 ns per pair native (35x the JS string-key Map); 56 KB of tables instead of a 128 MB matrix |
| **Separate level columns**: match uses the `nm` id, hint/reveal uses the printed `n` | a prototype that mixed them diverged on 2/156 cases; becomes a golden test |
| Quick stage from per-field histograms + static full-match lists, O(\|C\|+U) | 529 ms becomes 0.5 ms with identical numbers |
| Quick sorts compare **integer numerators** (solveMass, newMatchMass), not an epsilon | Rust ≥1.81 `sort_by` can panic on a non-total order; the result is equivalent because 1/T ≫ 1e-11 |
| Solver scans keep the `1e-11` epsilon, first maximum wins, hint evaluated first | parity with JS |
| Memo key drops the guessed set; no-op guesses pruned; keys are `u128` (u64 bitset over root candidates + known + matched + h + c + d) for n ≤ 64, 128-bit Zobrist otherwise; `FxHashMap` / bounded TT | exact-tier failures (7–57 s) become ≤ 5 ms; proof plus 249/249 values identical |
| Per-node effective action classes (O(1) projected signature), children inherit parent lists | 2.5x (median) to 15x fewer actions in endgames |
| O(1) closed-form evaluation at depth 1 | 5x on top of the faithful port; depth 3 at n = 5000 in 72 ms native |
| Iterative deepening 1→3 with a **deterministic work budget** (expanded states), streaming each finished depth | removes the throw-and-restart waste; reproducible tests; JS adds a wall-clock watchdog |
| Exact tier: cost-based gate (final puzzle, n ≤ ~12–16), resource caps `c_eff`, `h_eff` | only valid with zero continuation (final puzzle); caps not used anywhere else |
| Seeded xoshiro256++, per-episode seed `splitmix(base ^ i)` | same results whatever the worker count |
| **Parity strategy**: golden tests compare *per-tier* values (same inputs, same action set, same depth) against the legacy JS; tier selection is not compared | copying the JS memo identity just to match tier choice would cost more than it is worth |
| v2 (flag, M8): top-K entropy upper bound for root action sets, fitted `W[r][h][c]` continuation, Wordle-style lower bounds on E[actions] | depth-3 E[S] rises from 0.296 to 0.611 at n = 2258; changes advice, so it must pass a 10k-activity simulation with common random numbers first |

## 4. Milestones

Effort is in focused dev-days.

**M0: ship the legacy tool online now (optional, 0.5 d).**
- Deliverables:
  - `wrangler.jsonc`, `_headers` and `404.html` in the sibling's style;
  - a whitelist copy of `dist/` into `public/`, without card images or wallpapers;
  - optionally, a 2-line JS hotfix that removes `guessed` from `stateKey` in solver-core.js, which ends the 7–57 s freezes.
- Accept: the URL works in Chrome and on a phone, and an exact-tier state that failed before now responds.

**M1: toolchain, skeleton, spikes (1 d).**
- Deliverables:
  - workspace, `rust-toolchain.toml`, `tools/doctor.mjs` (checks that the wasm-bindgen CLI version equals Cargo.lock, rustup target, wrangler login, and the anaconda curl warning);
  - `build-wasm.mjs`: cargo, then `wasm-bindgen --target web --omit-default-module-path`, then optional `wasm-opt` with explicit `--enable-*` flags;
  - `build-site.mjs`: hashing, rewriting `index.html`, the b64 wasm file and the worker string.
- Spikes:
  1. Post a Module into a Blob worker from `file://` in Chrome, Edge and Firefox. Fallback: post the bytes and compile in each worker.
  2. Check `content-type: application/wasm` and the `_headers` rules under `wrangler dev`.
  3. Measure wasm size raw and brotli, with and without wasm-opt and `+simd128`.
- Accept: a hello engine runs over https and `file://`, and `npm run doctor` passes.

**M2: decoder-core with parity (3 d).**
- Deliverables:
  - `data`, `rules` (`RulePolicy` defaulting to the current behaviour), `state`, `candidates`, `quick`, `partition`, `exact`, `horizon`, `recommend` in all four modes (balanced, points, solve, info), with the heuristic hint rules kept;
  - `tools/export-golden.mjs`, built from the scratchpad `harness.js`, which dumps about 300 states (openings, worst cases, mid-game, endgames) with legacy outputs.
- Accept:
  - candidates are identical;
  - quick numerators are exact;
  - entropies agree within 1e-12 relative;
  - per-tier value vectors agree within 1e-9;
  - native worst-opening recommendation takes ≤ 100 ms and exact n ≤ 7 takes ≤ 5 ms (`mdcd bench`).

**M3: WASM, workers and UI refactor (2.5 d).**
- Deliverables:
  - `engine-client.js` (promise API, request ids, generation token, progress);
  - the worker; the main-thread sync engine;
  - app.js split up, with dead code and JS rules removed and the bugs from §2.2 fixed;
  - state v5 with export/import; WebMCP detecting both `document.` and `navigator.modelContext`.
- Accept:
  - the feature checklist (reader groups A–J) passes a manual QA script;
  - no long task over 50 ms on the main thread during a recommendation (PerformanceObserver);
  - changing state mid-calculation never renders a stale result.

**M4: deploy and CI (1 d).**
- Deliverables: the final `wrangler.jsonc`, `_headers` and `.assetsignore`; `ci.yml`, `release.yml`; a draft Chinese README deploy section.
- Accept: a push to main deploys; a `v*` tag attaches `md-card-decoder-offline.zip`; CI fails when a size budget is exceeded.

**M5: simulation on Rust (1 d).**
- Deliverables: `simulate_batch` shards across the pool; merged statistics (mean, CI, p10/p50/p90, Wilson, histogram, diagnostics); the seed is shown in the UI; `mdcd sim --rounds 10000 --threads 20`.
- Accept:
  - 500 activities at depth 2 with 4 workers take under 10 s in Chrome;
  - the same seed gives the same summary with 1 or 8 workers;
  - cancelling reports only completed activities.

**M6: data pipeline (2 d).**
- Deliverables: `mdcd data fetch|build|verify|diff`, which rebuilds from ygocdb `cards.zip` plus YgoMaster at pinned commit `0e20782…`; `sources.lock.json` (sha256, md5, commit); deterministic output with no wall-clock timestamps; `data-refresh.yml` runs weekly on a cron and opens a PR with the `summary.json` diff.
- Accept:
  - 9312 records, 8210 groups, Σwm 9059, 0 field and wm mismatches against the legacy data;
  - the invariants from the data report hold;
  - the MD representative is always an MD passcode (fixes 优秀精灵).

**M7: Chinese docs and cleanup (0.5 d).**
- Deliverables: the README (§8); `使用说明.txt` merged in, with references to nonexistent files (`打开求解器.html`, `下载*卡图.cmd`) removed; old docs moved into `docs/`; the `.cmd` removed or replaced by a shim (decision 11).

**M8: v2 model behind a flag (open-ended).**
- Deliverables: entropy top-K root sets, the `W` table fitted natively by approximate policy iteration, a cost-based exact gate with B&B lower bounds.
- Accept: a statistically significant gain in 10k-activity common-random-number simulations; the UI labels cross-puzzle numbers "近似".

Core work M1–M7 is about 11.5 days. M2 to M4 cannot finish before 2026-10-02, which is why M0 exists.

## 5. Testing strategy
1. **Rules:**
   - TOML fixtures of (guess passcode, target passcode, mask, reveals), including both official Zarc examples (13331639 against 83257450 and 90036274) and the FNo.0 and Tzolkin cases.
   - `proptest` properties: a self-guess returns 63 under every `RulePolicy`; feedback partitions agree with the known-field filter.
2. **Golden parity:** jsonl fixtures from `legacy/js`, run with `cargo test`. They are regenerated only when a deliberate rule change is made.
3. **Reference paths:** an unpruned brute-force solver on random small pools checks dedup, no-op pruning, caps and any future B&B.
4. **WASM smoke:** `node --test`, which loads `public/engine` through `initSync` and runs golden states. Node supports wasm natively.
5. **E2E (Playwright, M4):** the https build (under `wrangler dev`) and the `file://` build both load, recommend, cancel and simulate; a missing card image returns 404 and falls back to the card back.
6. **Performance and size:** `criterion` benches in CI with loose thresholds; size budgets checked by `build-site.mjs`.

## 6. Deployment

**`wrangler.jsonc`** follows the sibling:
- `name: "md-card-decoder"`, `compatibility_date: "2026-09-25"`, no `main`;
- `assets: { directory: "./public", not_found_handling: "404-page", html_handling: "auto-trailing-slash" }`;
- logs persisted; Chinese comments, including a warning that wrangler does not read `.gitignore`.

**`_headers`:**
- `/engine/*`, `/data/*`, `/assets/*` (hashed files): `public, max-age=31536000, immutable`;
- `/wallpapers/*`: 30 days;
- `/*`: `X-Content-Type-Options: nosniff` and `Referrer-Policy`;
- no COOP/COEP. A CSP, if added, needs `'wasm-unsafe-eval'` and `blob:`.

**Size budget** (brotli; the wasm figure is unknown until the M1 spike):

| Asset | Size |
|---|---|
| HTML + CSS + JS | ~50 KB |
| wasm | budget 150 KB, to be confirmed |
| `cards` bin (22 KB) + representative passcodes (30 KB) + zh names (61 KB) | ~113 KB (today 399 KB) |
| aliases, loaded lazily | 230 KB |

The app has about 40 files, well under the 20,000-file cap.

**Card images:** each deploy replaces the whole asset manifest. If thumbnails lived in the same Worker, every CI deploy that ran without the local image folder would delete them. Options:

| Option | Cost | Notes |
|---|---|---|
| A. None online; fallback chain goes relative `card-images/` → legacy `card-images-zh/`/`-high/` folders (zip users) → card back | 0 | safest legally |
| **B. Separate static Worker `md-card-images`** (`images/wrangler.jsonc`, assets = w300 WebP, ~8,023 files, ~183 MB), deployed rarely and locally; app built with `IMAGE_BASE` | Free plan, no domain needed | recommended if images are wanted online; a cross-origin `<img>` is fine without COEP |
| C. R2 behind a custom domain | needs a Cloudflare zone | the long-term home if the pool or image sizes grow |
| D. Hotlinking ygoprodeck | – | rejected: goes against their guidance and the project's stated policy |

**Wallpapers:** off in the online build by default, replaced by a CSS or procedural background (licensing: "署名不构成授权"). The offline zip can keep WebP versions (23.85 MB → 2.18 MB), loaded only behind `matchMedia('(min-width:981px)')` and paused on `visibilitychange`. Rename `card-back.gif` to `.jpg`, since the file really is a JPEG.

**CI (GitHub Actions):**
- `ci.yml`: `dtolnay/rust-toolchain` with the wasm32 target, `rust-cache`, `taiki-e/install-action` with `wasm-bindgen@<Cargo.lock version>`, Node 24; then fmt, clippy `-D warnings`, test, build, node tests, E2E, upload artifact.
- Deploy on main through `cloudflare/wrangler-action@v4` (secrets `CLOUDFLARE_API_TOKEN`, `CLOUDFLARE_ACCOUNT_ID`).
- `release.yml` on a tag zips `public/` (optionally with WebP wallpapers).
- Workers Builds is not used: its build image has no Rust.

**Linux dev loop** (replaces the `.cmd`):
```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version <lock> --locked --root .tools   # binary in ./.tools/bin (gitignored)
npm ci && npm run doctor
npm run build        # wasm + site → public/
npm run dev          # wrangler dev :8787 (reachable from the Windows browser under WSL2)
npm test             # cargo test + node --test + golden
npm run data:update  # was 更新大师决斗卡池.cmd → mdcd data fetch/build/verify
npm run images       # sharp thumbnails → build/card-images (never in git)
npm run deploy       # build + wrangler deploy; deploy:images for option B
```
The build scripts are Node, so they also run on Windows. Download scripts must use `/usr/bin/curl`, because the anaconda `curl` first on PATH has a broken CA bundle.

**Data update:** until M6, `build-site.mjs` packs the existing `cards-data.js` into the new bin, so the Node tools keep working. After M6, Rust builds the data. Group order: keep the legacy 8210 order and **append** new groups. That keeps v4 saves and golden fixtures index-aligned, and v5 persists passcodes anyway.

## 7. Risks
- **Timing:** the rewrite lands after the event ends; M0 covers the rest of this event.
- **Unconfirmed rules** (R3 pendulum-guess symmetry, 2.2% of pairs; R5 level/rank/link cross-match, 19.7% of level matches; R7/R8 rule-level cards; R15 whether the in-game list lets you pick cards that contradict known fields):
  - `RulePolicy` gets an advanced setting;
  - a diagnostic replays logs under each policy combination and reports which ones keep the candidate set non-empty.
- **Untested on real browsers:** posting a Module to a Blob worker under `file://` (spiked in M1, bytes as fallback).
- **wasm-bindgen version drift:** CLI and crate must match exactly (Arch ships 0.2.128, crates.io has 0.2.129). Pin with `=` and let `doctor` enforce it.
- **Floating-point parity:** last-bit `log2` differences and summation order. Use tolerances, keep ascending candidate order and first-occurrence grouping.
- **Changed advice:** users will see different recommendations once depth 3 completes. Needs release notes and a label showing the depth actually completed.
- **Copyright exposure** from a public URL (card art, wallpapers). Keep the takedown notice; images default off or on the separate Worker.
- **Cloudflare limits** can change. Re-check the file cap and compression list before choosing an image option.
- **Users losing progress** because the origin changes. Export/import must ship in the same release as the online launch.

## 8. Chinese README structure
1. 简介 (what the tool does, disclaimer, non-affiliation)
2. 在线使用 (URL, browser requirements)
3. 离线使用 (download the release zip, double-click `index.html`, where to put images)
4. 活动规则与模型假设 (six fields, the pendulum and rule-level cases, hint model, **unconfirmed assumptions** R3/R5/R7/R8/R15)
5. 推荐怎么看 (strict mode, the value vector, "任选其一", depth and exact labels, analysis modes)
6. 规模测试 (seed, confidence intervals, sample-size caveat)
7. 进度迁移 (export/import, v4 import)
8. 开发 (Linux: toolchain, `npm run …`, directory layout)
9. 数据更新 (sources, lock file, verification)
10. 部署到 Cloudflare (wrangler login, `_headers`, image Worker)
11. 算法说明 (links to `docs/算法说明.md`)
12. 版权与来源 (card art, wallpapers, ygocdb, YgoMaster)
13. 更新日志

## 9. Decisions the user must make (recommended default in bold)
1. Keep `file://` double-click support: **yes** (cheap with the dual loader), or hosted plus a PWA only.
2. Card images online: **A (none) now, B (separate images Worker, w300 WebP) if wanted**, C (R2, needs a domain on Cloudflare), or D.
3. Wallpapers in the online build: **off**. The offline zip keeps WebP versions: **yes**.
4. Do M0 (legacy tool online plus the 2-line JS freeze hotfix) before the event ends: **yes**.
5. v1 behaviour: **same model, values identical, tier choice may differ (depth 3 now completes)**, with the v2 model behind a flag.
6. Build artifacts: **CI release zip, `public/` stays out of git**, or commit the built files.
7. Worker name and URL `md-card-decoder`; Free or Paid plan; **CI deploy on main**, or manual `npm run deploy` like the sibling. Is the fork on GitHub with Actions enabled?
8. Data order: **legacy order plus append**, or re-sort MD-first as a contiguous prefix.
9. Default `RulePolicy`: **current behaviour**, with rule variants exposed in advanced settings.
10. Keep the points/solve/info analysis modes and WebMCP: **yes** (WebMCP gains `record_challenge`).
11. Windows `.cmd`: **delete**, or keep a one-line shim calling `npm run data:update`.
12. Directory names: **`web/` source plus `public/` output**; rename `dist/`.
13. Rust toolchain: **track `stable` with an MSRV**, or pin 1.98.1.
14. Simulation metrics that the old docs promise (高价值题全通率, 后段匹配收益): **restore them in Rust** (cheap), or drop them from the docs.

## 10. Workflow log
- 2026-09-25: plan drafted from the engine, ui_sim, data_rules, toolchain_deploy and algo reader reports. No repo files were changed.