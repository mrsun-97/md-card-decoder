# PLAN — Rust/WASM rewrite of 卡片解码者 (md-card-decoder)

> Status: **planning complete, no code written.** This file is the single place to trace the
> rewrite: goals, findings, the chosen algorithm, architecture, milestones, open decisions and a
> dated workflow log (§14). The full research that backs every number here is kept in
> [`docs/plan/research/`](docs/plan/research/) (see §15 for an index).
>
> Conventions: numbers marked *measured* come from the research runs (Node 26 for the legacy JS,
> a native Rust prototype on an i7-12700K / 20 threads under WSL2). *Estimated* numbers are not
> measured. WASM is assumed to be 1.3–2× slower than native until M1 measures it. The Rust prototype
> lived in `/tmp` and was lost in a reboot; its results must be **re-measured** during M3–M6.

---

## 1. Goals and non-goals

**Goals**

1. A Rust engine (`decoder-core`) that owns game rules, belief tracking, recommendation and simulation;
   compiled natively (CLI, tests, offline fitting) and to `wasm32-unknown-unknown` (browser).
2. **A better algorithm, not a port.** The legacy JS solver is only (a) an oracle for rule semantics
   and (b) a baseline policy to beat in simulation (user direction, 2026-09-25).
3. Interactive latency: recommendation p50 ≤ 50 ms, p99 ≤ 250 ms, in a Web Worker, never blocking
   the UI (legacy: ~3.2 s at the worst opening, 7–57 s freezes in the exact tier).
4. Robustness to the model being wrong (unconfirmed game rules, excluded cards, user input errors).
5. Honest labels: proven optimal / exact within the puzzle / approximate — never a heuristic called a bound.
6. Online at `md-card-decoder.<subdomain>.workers.dev` via Cloudflare Workers static assets (same
   pattern as `../ygo-small-world`), while still working when `index.html` is double-clicked (`file://`).
7. Linux-first development (npm scripts + cargo; no `.cmd`), reproducible data pipeline, CI.
8. README regenerated in Chinese; code comments in English; config comments in Chinese like the sibling.

**Non-goals (v1)**: full-Rust UI (Leptos/Dioxus/Yew); WASM threads / COOP-COEP; server-side compute
(the Worker only serves files); hosting hi-res card art; faithful reproduction of legacy decisions.

---

## 2. What we learned about the legacy tool (key findings)

| area | finding | evidence |
|---|---|---|
| exact tier | memo key contains the set of guessed cards → state explosion; **18/34** eligible final-puzzle states hit the 160k-state limit after 7–57 s on the main thread | research/01, /05 |
| fix is free | re-guessing is a strictly dominated no-op, so dropping `guessed` from keys + merging equivalent guesses keeps values **bit-identical** (249/249 cases) and solves former limit cases in ≤ 5 ms native | research/05 |
| depth 3 | always exceeds its 24k-state budget at openings (n ≥ 290): 0.7–2.2 s thrown away, then silently falls back to depth 2 | research/05 |
| leaf value | unsolved states at the search cutoff are worth **0** → myopic; the "当前可行下界" label is wrong (the value contains a heuristic continuation) | research/01 |
| continuation | `min(r, c, (c+0.61h)/3.9)` saturates at the opening and prices a hint at 0.61 challenges; fitted tables say **1.1–1.4** | research/01, /10 |
| quick pass | O(8023·\|C\|) scoring pass (0.25–0.53 s) is replaceable by O(\|C\|+8023) histograms (0.4 ms) with identical numbers | research/05 |
| dead code | B&B certificate path unreachable; in-page simulation (`app.js:1423-1658`) dead; rule code duplicated in 5+ places | research/01, /02 |
| simulation | 20.7–26 s per simulated 9-puzzle event in JS (500 events ≈ 3 h); uses `Math.random` (not reproducible); cancel counts a partial activity | research/02, /05 |
| saturation | with 11 hints / 36 challenges a legacy-policy port solves 8.98/9 on average, P(all 9) ≈ 98.5% → headroom is in the tail and in tighter presets | research/05, /10 |
| data | the builder that produced `cards-data.js` is **missing** (only `wm` can be refreshed); it was reconstructed and reproduces all 9312 records with 0 mismatches; names are 53% of the 2 MB file | research/03 |
| saves | saved progress references cards by **array index** | research/02, /03 |
| file:// | fetch, ES modules and module workers are blocked on `file://`; the tool already works around workers with a Blob-URL worker built from a JS string | research/02, /04 |

---

## 3. Game model and rule assumptions

### 3.1 Model (formalised in research/09 §1)

- Pool U = 8023 Master Duel "behaviour groups" (cards with identical judged fields), weights `wm`
  (Σ 9059). Target prior ∝ `wm` (configurable smoothing).
- Puzzle start: one field revealed, uniform over the 6 (legacy `seedTestInitialReveal`).
- **HINT** (h > 0, unknown fields remain): reveals the value of a uniformly random unknown field
  (level shows the printed `n`; frame shows the full combo).
- **CHALLENGE g** (c > 0): feedback = 6-bit match mask + target's full frame if frame matched + target's
  printed `n` if level matched. Solved iff mask = 63; up to 3 groups can fully match one guess.
- Belief is exact: posterior = `wm` restricted to the consistent set C (observations are deterministic
  given the target; the hint's field draw is target-independent).
- Puzzles are sequential; hints/challenges are an event-wide inventory; running out of challenges ends
  the event. Because every puzzle restarts from the same prior, the event value decomposes exactly into
  a table **F_r(h, c)** (r = puzzles left) as long as targets are i.i.d. (drawing without replacement
  changes the prior by < 0.4% over 9 puzzles — negligible).
- **Resource grants (official, confirmed 2026-09-26)**: day 1 starts with 4 challenges + 3 hints; every
  later day adds 4 challenges + 1 hint at 13:00 (on first login); unused counts **accumulate** until the
  event ends. Over 9 days that is exactly **36 challenges / 11 hints** (the preset). Because waiting
  costs nothing, the pooled model is exact: the engine plans with **effective inventory = current +
  grants still to come**, and the UI separately shows what is usable right now (「当前可用 3 / 计划总量 19」,
  and 「挑战已用完，明天 13:00 再来」 when current challenges hit 0 but grants remain). Presets therefore
  store `{days, initial: (c, h), daily: (c, h)}` instead of only totals.

### 3.2 Rule assumptions (audit in research/03 §3)

| # | assumption | status | weight |
|---|---|---|---|
| R1/R2/R4 | pendulum target matches either frame; frame reveal shows the target's full frame | **confirmed** (official 边框范例1/2) | – |
| R3 | a pendulum guess matches a non-pendulum target of the same base frame (bitwise b&b, symmetric); the reveal shows the target's frame | **confirmed** (official 边框范例3: target 艾克佐迪亚 (Fusion), guess 霸王龙扎克 (Fusion/Pendulum) → reveals 「融合」) | 2.2% of weighted pairs |
| R5 | level / rank / link compared as one number (Level 4 = Rank 4 = Link 4) | **confirmed** by the user (2026-09-26) | 19.7% of level matches |
| R6–R8 | rule-level cards: match = `nm & nm` on **both** sides (printed value and rule value both count, guess side included); the reveal shows the target's **rule** level | **confirmed** (official 等级范例1–3: Tzolkin target vs FNo.0 guess matches via printed 0 → reveals 12; Tzolkin ↔ 安提霍普 match via rule 12 → reveals 12) | 7 cards |
| R9/R10 | '?' ATK/DEF equal each other; all Link DEF equal each other | **unconfirmed** (user, 2026-09-26) — keep as default, `RulePolicy` switch | 71/43/388 groups |
| R13 | hint field uniform over unknown fields; a level hint on a rule-level card shows the **rule** value | rule value: **adopted as assumption** (user); uniform field: assumed | – |
| R14 | exactly one revealed field at puzzle start (the first lightbulb column in 「记录」) | **confirmed** (user) | – |
| R15 | action set | **resolved: unrestricted.** The in-game list auto-filters by known info, but the player can reset the filter and pick inconsistent cards | – |
| R16 | hint/initial reveals do not count toward match milestones; a field counts only when a **submitted card** matches it (also if it was hinted before) | **confirmed** (user) | objective only |

**Net result: the legacy rule semantics (`b & b`, `nm & nm`, reveal = target `b` / rule `n`) are exactly
right**, so the default `RulePolicy` equals the legacy behaviour and the rule fixtures can be taken
straight from the official examples.

**Why confirming mattered (measured, research/11 §6):** had the game used a different R3/R5 than the
model, the filter would delete the true card and E[solved] would drop from **8.99 to 4.50–6.43**. With
R3/R5 confirmed, the multi-variant rule mixture is no longer needed in v1 (kept as a v2 option for the
remaining minor assumptions R9/R10); contradiction recovery for user input errors stays in v1.

All rule choices live in one `RulePolicy` that only rebuilds small compatibility tables (zero hot-loop cost).

---

## 4. Algorithm (the core of the rewrite)

Synthesised from three independent first-principles designs (research/09 exact DP, /10 approximate
DP, /11 systems & robustness). The judging/verification agents were cut off by a session limit; the
synthesis and the checks below were done by hand (§14).

### 4.1 Objective

- **Default: LEX** = lexicographic (E[puzzles solved], E[first-time matched fields], −E[actions]) —
  continuity with the legacy tool. Note that a solved puzzle always ends with 6 matched fields, so
  LEX ≈ E[solved] with two tie-breakers.
- Later, same machinery with different tables: **ALL** = P(all remaining puzzles solved) (the tail
  objective) and **PTS** = expected reward points from the event config (premium milestones etc.,
  table indexed by puzzle number). On the default preset all three mostly agree; on tight presets
  they diverge (e.g. 6 hints / 27 challenges: E[S] 7.31, P(all) 10.6%).

### 4.2 Architecture of a decision

```
logs ──► belief layer (§4.3) ──► kernels (§4.4) ──► anytime search (§4.5) ──► Recommendation (§4.8)
                                                     │  R0  greedy with learned leaf        (< 1 ms)
                                                     │  R1  depth-2 expectimax + learned leaf + F̂   (FV2)
                                                     │  R2  R1 with a widened root (only if CRN-validated)
                                                     └  exact tier overrides when it completes (§4.6)
offline (native):  fit F̂[k][h][c] and the cost-to-go leaf (§4.7)  ──►  ~30–100 KB of tables shipped
```

### 4.3 Belief layer (exact filtering + robustness)

- One candidate set under the confirmed rules (R1–R8), with a 128-bit Zobrist fingerprint. The engine
  keeps the `RulePolicy` switch so a variant (e.g. R9/R10) can be tested, and a rule-variant mixture
  (per-variant slices + posterior, research/11 §2.2) is a v2 option only if a remaining assumption is doubted.
- **Guesses are unrestricted** (R15 resolved): any MD card may be recommended, including ones that
  contradict known fields — these are often the most informative. The UI tells the user to reset the
  in-game filter when the recommended card is not consistent with the known fields. An optional
  「只推荐符合已知条件的卡」 setting restricts the action set (measured cost ≤ 0.012 solved).
- **Contradiction recovery** (`relax1`): if the candidate set goes empty, rank log entries by how much candidate
  mass removing each one restores (≈0.1 ms) and ask the user "第3次挑战的反馈可能录入有误？" —
  mis-entry is the most likely real-world contradiction. Never auto-apply.
- Unknown-card mass ⊥ (card missing from the dataset / excluded by the event) is displayed, not searched;
  warn when P(⊥) > 0.3.

### 4.4 Kernels (all exact; measured native)

| kernel | what | cost |
|---|---|---|
| SoA columns | u8 value ids per field; separate level columns: **match by `nm`, hint/reveal by printed `n`** (a prototype bug proved this trap) | 65.7 KB |
| feedback code | factorized u16 (`frame_tab·240 + attr·120 + race·60 + level_tab·4 + atk·2 + def`) from ≤16×16 tables; replaces a 128 MB 8023² matrix | 2.2 ns/pair |
| partition | dense histogram + touched list + counting sort into an arena | O(\|S\|) |
| quick metrics | P(solve), E[new matches], per-field response entropies for all 8023 guesses from per-field histograms + full-match lists (≤ 3 per guess, 8149 pairs) | 0.13–0.50 ms |
| equivalence classes | O(1) projected signature per guess (frame/level compat sets ∩ present values, value-or-MISS for equality fields, minus matched fields, plus FM(g)∩C); children inherit parent classes | 0.12–0.20 ms |
| top-K informative | exact top-K by entropy using the bound H ≤ Σ_f H(response_f) | K=64: 84–206 partitions, ~1.3 ms at n=5000 |
| frontier trick | whenever a node is scored by the one-step closed form [P(solve)·(1+W_S), …] (zero leaf) — in particular at c = 1, where only an immediate solve can matter — the best challenge lies in N(C) = ∪_{t∈C} FM(t) (≤ 3\|C\| cards) → exact over all 8023 guesses in O(\|C\|). Does **not** apply to learned-leaf nodes | – |

Memo keys never contain the guessed set; keys are u128 for n ≤ 64 (bitset + known + matched + h + c),
128-bit Zobrist otherwise; bounded transposition table (depth-preferred replacement). No `std::time`
in core (panics on wasm32): budgets are **work units**; JS owns a 240 ms wall-clock safety stop.

### 4.5 Online search — FV2 (the main tier)

- **Root actions**: legacy-style set (18 best by (P(solve), E[new matches]) ∪ 8 best by approximate
  info ∪ 8 heaviest candidates) ∪ top-K by entropy (K = 0 in v1; 16–32 once CRN shows a gain),
  deduplicated by equivalence class, plus HINT.
- **Depth-2 expectimax** with exact outcome enumeration. A solved branch is worth reward + F̂(k, h, c−1);
  an unsolved depth-1 node runs `leaf_eval`: HINT and each inherited action scored one step ahead with the
  **learned leaf** V̂ (below). Cost ≈ |A_root|·|A_leaf|·n pair evaluations.
- **Learned leaf = distributional cost-to-go.** Offline we learn P(J challenges, E hints still needed to
  solve | entropy bucket b = round(2·H(C)), hint flag) and compose it analytically with F̂:
  `V̂_S = Σ_{j,e} P(j,e|φ)·1{j≤c}·(1 + F̂_S[k][h−e][c−j])` (M and −A analogous). The resource trade-off
  therefore comes from the fitted table, not a magic ratio. 12.5 KB.
- *Measured* latency, one thread, native: 22.3 ms at n = 5000, 10–12.5 ms at n ≈ 2200, ≤ 3 ms at
  n ≤ 600, < 0.5 ms in endgames → *estimated* 29–45 ms worst case in WASM.
- *Measured* quality vs a native port of the legacy policy (paired, common random numbers):

  | preset (puzzles/hints/challenges) | legacy L: P(all), E[S] | **FV2**: P(all), E[S] | paired Δ (95%) |
  |---|---|---|---|
  | 9/11/36, 10k events | 98.51%, 8.9846 | **99.84%, 8.9984** | ΔP +1.33 ± 0.24 pts |
  | 9/11/28, 2k | 65.25%, 8.578 | **84.90%, 8.835** | ΔP +19.65 ± 2.08 pts |
  | 9/4/30, 3k | 14.37%, 7.352 | **23.17%, 7.739** | ΔS +0.387 ± 0.033 |

  Ablations: fitted table alone ≈ no gain; learned leaf with the legacy formula is *worse* (overspends
  hints); depth 3 with the learned leaf is not better and costs 10× → **v1 ships depth 2**.
- Anytime rungs: R0 (depth-1 greedy with V̂, always available) → R1 (FV2) → R2 (FV2 with widened
  root, only if validated); each rung completes atomically and the last completed rung is returned.
- MCTS was evaluated and rejected: outcome distributions are exact and cheap (≤ 57 cells per action),
  so sampling only adds variance.

### 4.6 Exact tier (proofs where they are affordable)

- **Final puzzle** (zero continuation): exact expectimax with resource caps c_eff = min(c, |C|+6−|M|),
  h_eff = min(h, #unknown) (valid **only** with zero continuation — enforced by the type system),
  inherited equivalence classes, and **branch-and-bound** with two admissible bounds:
  - P̄₁ — exact max probability of solving on the next challenge, using all remaining hints first
    (valid for one challenge only; for ≥ 2 challenges "hints first" is *not* a valid relaxation);
  - Lemma A — E[actions] ≥ 2 − P̄₁(no hints) when c ≥ 2.
  *Measured*: identical values to the unpruned solver in 260/260 cases; worst case for n ≤ 16 drops
  from 7.4 s to **64 ms**; outliers remain (n = 23, h4, c6: 17 s) → gated by the work budget, falls
  back to FV2. Optional: exact integer arithmetic (values scaled by w(C)·60^h fit in i64) for exact tie sets.
- **Non-final puzzle, small n (≤ ~7)**: exact within the puzzle using F̂ as the continuation, no caps
  (*measured* 48 ms at n = 7, h11 c36). Label 「题内精确·跨题估计」.
- **c = 1**: only an immediate solve matters → use hints while they raise the expected solve
  probability, then challenge the best card in N(C) (exact).
- The top-(c·k_max) solve bound proposed earlier is **not admissible** (adaptive trees solve up to
  k_max·(1+K+…+K^{c−1}) targets) and is banned. Every bound is property-tested against a brute-force
  reference on every node.

### 4.7 Offline fitting (native `mdcd fit`, deterministic, rayon)

1. **F̂[k][h][c] (S and M components)**: backward induction over k; for each cell simulate 64 fresh
   puzzles with CRN across cells under the policy that consults F̂[<k]; monotone repair (running max in
   c then h); clamp 0 ≤ F̂ ≤ min(k, c). *Measured* 71 s wall for the 9×12×37 grid (zero-leaf policy);
   *estimated* 3–4× under FV2; a generic grid for custom presets (k ≤ 15, h ≤ 20, c ≤ 60) ≈ 20 min.
2. **Cost-to-go leaf**: log (entropy bucket, hint flag) → realised (J, E) along simulated trajectories on
   a **mixture of presets** (9/11/36, 9/11/28, 9/4/30, 9/0/36) — a single hint-rich preset leaves the
   no-hint bins empty (the failed policy-iteration round showed this). Minimum 40 samples per bin.
3. **Policy iteration** only if a held-out 10k-event CRN evaluation shows a gain (round 1 showed none).
4. Optional sanity floor: an exact lower-bound table F^lo from a menu of simple policies (research/09
   §2.8; *measured* 8.873 / P(all) 0.894 — valid but too weak to drive decisions).
5. Tables carry a header (`data_hash`, `rules_hash`, `objective_id`, grid, seed). If they do not match the
   current pool/rules/objective the engine says 「估值表与当前设置不匹配」 and falls back; outside the grid
   values are clamped and labelled 「外推」. Refit in CI when card data changes (~10 min).

Shipped: F̂ default grid 16 KB + generic grid ~82 KB raw (~30 KB br, *estimated*) + leaf 12.5 KB.

### 4.8 Output, labels and explanations

```rust
pub struct Recommendation {
    action: Action, ties: Vec<Gid>,                 // explicit tolerance, deterministic tie-break (lowest gid)
    label: ProofLabel, rung: Rung, units: u64, truncated_by_wall: bool, stable: bool,
    hint_vs_challenge: Option<HvC>,                 // both evaluated at the same rung
    per_action: Vec<ActionStats>,                   // top 3–6: P(solve now), E[new matches], entropy, outcome cells
    posterior: Posterior,                           // top-N cards with probability, groups, mass
    forecasts: Forecasts,                           // E[challenges/hints still needed] (leaf), E[puzzles solvable] (F̂)
    rules: [(RuleId, f32); 4], unknown_mass: f32, contradiction: Option<Relax1Report>,
}
```

| situation | UI label |
|---|---|
| final puzzle, exact tier completed | 已证明最优（模型内） |
| non-final puzzle, exact within the puzzle | 题内精确·跨题估计 |
| FV2 | 近似最优（学习估值·深度2） |
| budget exhausted before FV2 | 快速近似 |
| wall-clock truncation | 已截断 |

The legacy separate "points / solve / info" modes (with heuristic hint thresholds) become **columns** in
the per-action table (immediate score, solve probability, information) instead of alternative policies;
PTS/ALL objectives replace them properly later.

### 4.9 Simulation

- Native `mdcd sim`: rayon over events, per-event seeds `mix(seed, event, puzzle, stream)` (targets on
  stream 0, hint draws on stream 1 → common random numbers across policies), integer accumulators →
  identical results at any thread count and native vs WASM.
- The legacy policy port (`baseline::legacy`, L and L3) lives in `decoder-cli` for A/B only.
- Browser 规模测试 runs `simulate_batch` on the Worker pool (target: 500 events at FV2 in well under a
  minute; legacy JS ≈ 3 h), shows the seed, excludes cancelled partial events, and restores the metrics
  the old docs promise (高价值题全通率, 后段匹配收益).

### 4.10 Later (v2+), in rough priority order

pondering + recommendation cache (measured: the realised next state is ready p50 12.8 ms after pondering
starts) · ALL/PTS objectives with an "objectives agree" badge · widened root (K = 16–32) · true
rule-mixture search (only if R9/R10 are doubted) · renewal kernel so any custom preset gets F̂ in the browser in ~1 ms ·
two-sided bounded search showing a numeric optimality gap everywhere · richer leaf features ·
root-parallel search over Workers · rollout "deep analysis" button · day-structure model if hints expire.

---

## 5. Architecture

### 5.1 Repository layout (target)

```text
md-card-decoder/
├─ Cargo.toml  Cargo.lock  rust-toolchain.toml   # workspace (resolver 3); stable + wasm32 target, rustfmt, clippy
├─ crates/
│  ├─ decoder-core/     # pure Rust, no wasm deps: data, rules, belief, kernels, search, cont, sim, rng
│  ├─ decoder-wasm/     # cdylib facade: #[wasm_bindgen] Engine; wasm-bindgen "=0.2.x" (pinned), serde-wasm-bindgen
│  └─ decoder-cli/      # native `mdcd`: data fetch|build|verify, sim, fit, bench, replay-rules, baseline::legacy
├─ web/                 # UI source (today's dist/ UI split into classic-script modules)
│  ├─ index.html 404.html _headers .assetsignore styles.css presets.js card-back.jpg
│  ├─ js/  store.js render-*.js dialogs.js images.js storage.js webmcp.js engine-client.js
│  └─ worker/engine-worker.js
├─ data/                # sources.lock.json + summary.json (committed); raw/ cache (gitignored)
├─ tables/              # fitted F̂ / leaf tables (committed, small, versioned headers)
├─ legacy/              # frozen copy of today's dist/ JS: rule oracle + baseline reference (never deployed)
├─ tests/               # golden/*.toml rule fixtures, node/*.test.mjs (wasm smoke), e2e/
├─ tools/               # build-wasm.mjs, build-site.mjs, images.mjs, doctor.mjs (Node; also run on Windows)
├─ docs/                # 算法说明.md, 规则与假设.md, plan/research/
├─ public/              # GENERATED + gitignored: the folder uploaded to Cloudflare == offline zip content
├─ .tools/              # project-local wasm-bindgen / wasm-opt binaries (gitignored)
├─ package.json  package-lock.json  wrangler.jsonc (optional, local preview)  .github/workflows/ci.yml
└─ README.md (中文)  PLAN.md
```

`dist/` goes away (`git mv dist legacy` once the new UI works). `build-site.mjs` copies a **whitelist**
from `web/` plus the generated sheets into `public/`, so stray local folders (e.g. hi-res `card-images*`) can never be uploaded.

### 5.2 `decoder-core` modules

| module | responsibility |
|---|---|
| `data` | `CardTable` (SoA u8 columns: frame, attr, race, lvl_match (nm id), lvl_reveal (printed n), atk, def, wm, wa), dictionaries, passcode map, `cards.bin` codec |
| `rules` | `RulePolicy` (confirmed defaults; switches for R9/R10 and the optional consistent-only action filter) → compat/reveal tables; `feedback(g, t) -> Fb(u16)` |
| `belief` | per-variant candidate slices, rule posterior, ⊥ mass, `relax1`, state from logs |
| `kernel` / `metrics` / `actions` | §4.4 |
| `search` | R0/R1/R2 rungs, exact tier + bounds, transposition table, work budget |
| `cont` | F̂ tables + cost-to-go leaf, table headers, fallbacks |
| `value` | LEX vectors / scalar objectives, tolerance comparison, total-order sort keys |
| `recommend` | the public pipeline and `Recommendation` |
| `sim` + `rng` | event simulation, SplitMix64/xoshiro256++, mergeable `SimStats` |

Rules of the road: `rustc-hash` maps; `libm` for transcendental functions (native = WASM bit-for-bit);
integer keys or `total_cmp` in sorts (Rust ≥ 1.81 `sort_by` may panic on a non-total epsilon comparator);
no FMA reliance; `panic = "abort"` in release.

### 5.3 Engine API (WASM boundary)

State stays in JS (existing schema + persistence); the engine rebuilds the belief from the logs on each
call (a few ms) and is otherwise stateless apart from caches. Coarse calls only.

```rust
#[wasm_bindgen] impl Engine {
    #[wasm_bindgen(constructor)] pub fn new(cards_bin: &[u8], tables_bin: &[u8]) -> Result<Engine, JsError>;
    pub fn set_policy(&mut self, policy: JsValue) -> Result<(), JsError>;           // RulePolicy + objective + prior
    pub fn candidates(&self, state: JsValue) -> Result<JsValue, JsError>;            // ids, masses, rule posterior, ⊥
    pub fn reveal_options(&self, state: JsValue, field: u8) -> Result<JsValue, JsError>;
    pub fn check_challenge(&self, state: JsValue, guess: u32, fb: JsValue) -> Result<JsValue, JsError>; // validation + contradiction report
    pub fn recommend(&mut self, state: JsValue, budget: JsValue, progress: &js_sys::Function) -> Result<JsValue, JsError>;
    pub fn simulate_batch(&mut self, cfg: JsValue, seed_hi: u32, seed_lo: u32, first: u32, count: u32) -> Result<JsValue, JsError>;
}
```

---

## 6. Frontend and WASM integration

- **Keep the vanilla JS UI and refactor it**: 104 KB of working Chinese UI; the engine boundary is ~7
  calls; classic scripts keep `file://` working. esbuild → IIFE bundles; `// @ts-check` with the
  wasm-bindgen `.d.ts` for type checking without a TS build.
- **Where the engine runs**

  | tier | where | calls |
  |---|---|---|
  | sync | one wasm instance on the main thread | `candidates`, `reveal_options`, `check_challenge` (1–5 ms) |
  | interactive | 1 Worker | `recommend` (anytime; streams completed rungs) |
  | simulation pool | N = clamp(hardwareConcurrency−1, 1, 8) Workers | `simulate_batch` |

  The main thread compiles the wasm once and posts the `WebAssembly.Module` to each Worker
  (`initSync({ module })`). Cancel = `worker.terminate()` + respawn from the cached Module (a running
  wasm call cannot see a cancel message); batches of 50–200 ms keep progress flowing.
- **One build, two loaders** chosen at runtime:

  | | `https:` | `file:` (double-click) |
  |---|---|---|
  | wasm | `WebAssembly.compileStreaming(fetch('engine/decoder_bg.<h>.wasm'))` | classic `engine/wasm-b64.<h>.js` → `WebAssembly.compile(bytes)` |
  | workers | `new Worker('engine/engine-worker.<h>.js')` | Blob URL from embedded `ENGINE_WORKER_SOURCE` (generalises today's trick) |
  | data | classic `data/cards.<h>.js` (base64 bin + zh display names) | same |
  | aliases | `data/aliases.<h>.js`, loaded on first search focus | same |

- **Legacy UI bugs fixed during the port**: stale recommendation after a state change (generation
  token), worker `onerror` leak, cancelled simulation counts a partial event, progress pulse shows
  `undefined`, candidate table silently truncates to 100 rows, search has no debounce / NFKC folding, custom presets cannot be deleted.
- **State v5**: cards by passcode + data hash (not array index); imports v4 saves via a shipped
  `legacy-index-map.json`; JSON **export/import** (moving from `file://` to https changes the storage origin).
- **New UI elements**: 「请先重置游戏内筛选」 hint when the recommended card contradicts known fields + optional consistent-only setting; contradiction dialog; forecasts
  ("本题预计还需 x 次挑战"); hint-vs-challenge explanation ("提示：本题解出概率 +x%，当前 1 提示 ≈ z 挑战");
  proof label + completed rung.
- **Keep**: test mode, mini-game, archive, presets, theme, card zoom, WebMCP tools (add `record_challenge`).

### 6.1 活动轨迹 in the in-game 「记录」 style (user request, 2026-09-26)

Replaces the legacy "newest 8 entries" list and the archive timeline with one component modelled on the
in-game record screen (reference screenshots: Q1 魔界剧团-美丽女主演, Q3 焰征龙-爆破).

```text
 [ Q1 ][ Q2 ][▶Q3◀][ Q4 ] …                       ← puzzle tabs; active tab lime; one tab per puzzle
 ┌────────┐ │ ┌──────┐   ┌──────┐   ┌──────┐   ┌──────┐   (✓)
 │ target │ │ │  💡  │ ▶ │ card │ ▶ │  💡  │ ▶ │ card │ ▶ card   ← hint column = lightbulb; challenge = thumbnail;
 │  card  │ │ └──────┘   └──────┘   └──────┘   └──────┘     ✓ badge on the solving challenge
 ├────────┤ │ [      ]   [效果 ■]   [      ]   [效果灵摆■]  ← 6 rows per column, fixed order:
 │效果灵摆■│ │ [      ]   [光    ]   [暗    ]   [暗    ■]     边框 / 属性 / 等级·阶级·连接 / 种族 / 攻击 / 守备
 │暗     ■│ │ [      ]   [4    ■]   [      ]   [4    ■]
 │4      ■│ │ [      ]   [雷族  ]   [      ]   [魔法师族]
 │恶魔族  ■│ │ [1500  ]   [1500 ■]   [      ]   [1500 ■]
 │1000   ■│ │ [      ]   [1500  ]   [      ]   [1000 ■]
 └────────┘ │  ←──────────── horizontal scroll, auto-scrolls to the newest column ───────────→
   sticky       ■ = yellow "matched" row; dark row = shown but not matched; empty dark slot = unknown
```

- **Left summary column (sticky)**: the target card (card back / unknown art until solved, the answer
  card after solving or in test mode) and the six current known values. Rows matched by a challenge are
  yellow with a faint ✓; values known only from hints/initial reveal are dark; unknown rows are empty slots.
- **Hint column**: lightbulb icon instead of a card; only the revealed field is filled (dark row, since a
  hint is not a match), the other five are empty slots.
- **Challenge column**: guessed card thumbnail; all six rows show the **guessed card's** values
  (pendulum frames as two stacked lines, e.g. 「效果 / 灵摆」); matched rows yellow, others dark.
  The full revealed target frame / rule level goes to the left summary column.
- **Solved**: ✓ badge above the solving column; the tab gets a solved marker.
- **Row icons**: frame-colour chip, attribute emblem, a combined level/rank/link glyph (★★L) in challenge
  columns and the specific glyph (★ level, ★ black rank, L link) once known, race icon, ATK sword, DEF shield.
  **All icons are our own SVG/CSS look-alikes** (extending legacy `fieldGlyphHtml`); no KONAMI UI assets are copied.
- **Extras beyond the game** (small, muted, under each column): candidates remaining after the step, and
  a ★ when the step followed the tool's recommendation.
- Interaction: click a thumbnail → zoom viewer; horizontal scroll with snap on phones (column ≈ 88 px,
  summary column stays sticky); keyboard ←/→ between tabs; the same component renders the archive
  (「打开档案」) for past activities and test/mini-game sessions.
- Data: state v5 keeps the **full per-puzzle log for every puzzle** (legacy keeps only the current puzzle's
  logs plus a 240-entry history), including the candidate mass after each step and the recommended action.

---

## 7. Data pipeline (Linux-first)

- `mdcd data fetch`: pinned sources recorded in `data/sources.lock.json` (sha256, upstream md5/commit):
  ygocdb `cards.zip` (fields + zh/en/ja names), YgoMaster `CardList.json` + `YdkIds.txt` at a **pinned
  commit** (not `master`); ygoprodeck only for cross-checks.
- `mdcd data build` restores the missing builder (reproduces today's 9312 records / 8210 groups with
  0 field and 0 `wm` mismatches — verified in research/03):
  1. ygocdb monsters with `data` and `id < 1e8`; derive b/a/r/n/atk/def (link DEF = −3, '?' = −2);
  2. printed level 0 for the 7 「规则上等级/阶级当作N」 cards (regex, cross-checked, must be exactly 7);
  3. group by (b, a, r, n, nm, atk, def) — the 6-field signature alone is **not** unique (Tzolkin vs 赤龙);
  4. `wm` = MD passcodes in the group, `wa` = all passcodes, representative = first **MD** passcode;
  5. deterministic order (MD groups first), no wall-clock timestamps → byte-identical rebuilds.
- Outputs: `cards.bin` (65.7 KB raw / 22 KB br), representative passcodes (~30 KB br), zh display names
  (~61 KB br), lazily loaded aliases (~230 KB br, NFKC-folded haystack). First load: 399 KB br → ~113 KB br,
  and no 2 MB JS literal parsed on the main thread.
- `mdcd data verify` fails CI on invariant violations (counts, domains, `def = −3 ⇔ link`, popcount(nm) ≤ 2,
  unique group key, MD representative, `wm ≤ wa ≤ 255`) and prints a diff against the previous lock.
- `npm run data:update` replaces `更新大师决斗卡池.cmd`; image sheets are built in Node (`tools/images.mjs`, sharp).
- Download scripts use `/usr/bin/curl` (the anaconda `curl` first on `PATH` has a broken CA bundle).

---

## 8. Deployment (Cloudflare Workers static assets, uploaded through the dashboard)

**Decision (user, 2026-09-26): no CLI towards Cloudflare.** The build produces one folder (`public/`,
also downloadable as a zip from CI/Releases) and the user uploads it in the Cloudflare dashboard as a
Worker with static assets (same hosting type as `../ygo-small-world`, just without wrangler).
Consequences for the design:

- **Stay under 1,000 files.** Dashboard/drag-and-drop uploads are limited to **1,000 files, 25 MiB each**
  (Pages drag-and-drop and Cloudflare Drop document this limit; assume the same for the Workers
  dashboard upload until M1 checks it). Plain static assets allow 20,000 files (Free) — irrelevant here.
  `build-site.mjs` fails the build if `public/` exceeds 900 files or any file exceeds 20 MiB.
- **Card images = low-quality WebP packed into sprite sheets** (decision D4): ~8,023 representative
  thumbnails at ~w180 q60–70 (measured ≈ 10 KB each at w180 q75 → ~80 MB total) packed **64 per sheet**
  (8×8 grid, ~650 KB per sheet) → ~126 files. `cards.bin` stores (sheet, cell) per group; the UI shows a
  thumbnail with CSS `background-image` + `background-position` (works on `file://`, no fetch), and only
  the sheets actually needed are downloaded. The zoom viewer upscales the same thumbnail (low quality is
  acceptable per the user). Sheets are generated by `tools/images.mjs` (sharp) from the zh CDN set
  (English fallback), cached locally, never committed. Missing card → card back.
- **No wallpapers** (decision D5): the wallpaper feature and files are dropped from the new tool (they
  stay in `legacy/` history only); the side panels get a CSS background. This also removes the licensing
  problem (「署名不构成授权」).
- **Do not rely on server config.** Whether `_headers` / `404.html` / `wrangler.jsonc` settings apply to a
  dashboard upload is unverified, so the app must work with Cloudflare defaults (`max-age=0,
  must-revalidate` + ETag → a re-upload takes effect immediately). Content-hashed file names keep
  browser caches correct. `_headers` (immutable caching for hashed files, `nosniff`) is still shipped as
  a harmless bonus. `.wasm` is served as `application/wasm` by extension (streaming compile works;
  the loader falls back to `WebAssembly.compile(arrayBuffer)` if the MIME type is ever wrong).
- Expected upload: ~40 app files + ~126 image sheets ≈ 170 files, ≈ 85 MB (images) + < 1 MB (app).
- `wrangler.jsonc` + `wrangler dev` stay **optional** for developers who want a local preview with
  Cloudflare semantics; any static server (`npx serve public`, `python -m http.server`) works too.
- **Upload checklist** (goes into the Chinese README): `npm run build` (or download the Release zip) →
  Cloudflare dashboard → Workers & Pages → Create → upload the `public/` folder → deploy; to update,
  create a new deployment with the new folder.
- **CI** (`.github/workflows/ci.yml`, optional): fmt, clippy `-D warnings`, `cargo test`, wasm build with
  the CLI pinned to `Cargo.lock`, `node --test`, and a `public.zip` artifact / Release asset ready to upload.
  No Cloudflare credentials in CI.
- **Git remotes**: `origin` currently points at upstream `gumiajer21/card-decoder`; add the user's fork
  as `origin` and rename upstream to `upstream` before any push.

### 8.1 Linux dev loop (replaces the `.cmd` launchers)

```sh
rustup target add wasm32-unknown-unknown                      # rustup-managed toolchain; no sudo
cargo install wasm-bindgen-cli --version <Cargo.lock> --locked --root .tools
npm ci && npm run doctor   # CLI == crate version, wasm32 target, curl CA
npm run build              # cargo → wasm-bindgen --target web --omit-default-module-path → (wasm-opt) → esbuild → public/
npm run dev                # local static server on public/ (reachable from the Windows browser under WSL2)
npm test                   # cargo test + node --test
npm run data:update        # fetch newest upstream sources + rebuild data/generated (committed)
npm run data:check         # compare with the legacy data and legacy judging formula
npm run fit                # refit tables/ (F̂ + leaf) after a data change (~6 min on 20 threads)
npm run images             # download thumbnails once (build/thumbs) and pack sprite sheets
npm run pack               # build + zip public/ → build/md-card-decoder-site.zip for the upload
npm run test:e2e           # Playwright smoke test over http:// and file://, plus phone width
```

Layout after M8: generated card data is committed in `data/generated/` (cards.bin, names.json,
summary.json) so the site builds without network access; `data/raw/` stays a local cache. The old
tool lives frozen in `legacy/` (rule oracle, v4-save import map, baseline), including its Windows
`.cmd` launcher and Node tools.

Toolchain facts: the rustwasm GitHub org was archived in 2025; wasm-bindgen and wasm-pack are maintained
under `github.com/wasm-bindgen`. We skip wasm-pack/trunk and run the tools directly. The CLI version must
equal the crate version exactly (Arch ships 0.2.128, crates.io 0.2.129 as of 2026-09-25) — pin with `=`,
`doctor` enforces it. `wasm-opt` needs explicit `--enable-bulk-memory --enable-nontrapping-float-to-int
--enable-sign-ext --enable-mutable-globals --enable-reference-types --enable-multivalue` since rustc 1.87;
it stays optional.

---

## 9. Testing and validation

1. **Rule fixtures** (TOML: guess passcode, target passcode, mask, reveals) from **all six official
   examples**: 边框范例1–3 (Zarc 13331639 vs 艾克佐迪亚 / 清透翼疾速龙, and the reverse pendulum-guess
   case) and 等级范例1–3 (Tzolkin vs FNo.0 未来皇霍普 → reveal 12; Tzolkin ↔ 绝望神 安提霍普 → reveal 12);
   plus legacy `matchMask` digest equality under the default policy (legacy JS = rule oracle).
2. **Property tests** (proptest): self-guess = 63 under every `RulePolicy`; partitions agree with the
   known-field filter; child masses sum to the parent; monotonicity V(h+1,c) ≥ V(h,c), V(h,c+1) ≥ V(h,c);
   single candidate → direct challenge; O(|C|+U) quick metrics equal the naive A·N loop; entropy upper
   bound ≥ exact; **match uses `nm`, reveal uses `n`**.
3. **Brute-force reference solver** (no memo, no dedup, no pruning, re-guesses allowed) on random small
   synthetic pools under every rule variant: every optimisation (dedup, no-op pruning, caps, B&B) must give
   identical values; every bound must hold on **every node**.
4. **Policy quality** (native, CRN): ≥ 10k events on 9/11/36 and 2–3k on stress presets (9/11/28, 9/4/30,
   9/0/36, 9/11/24, 12/14/45, one preset not in the fit mixture) and under rule variants. v1 gate: FV2 ≥
   legacy L on every preset (lower 95% bound of ΔE[S] ≥ 0), P(all) non-inferior on the default preset.
5. **Calibration**: leaf P(J ≤ j) vs realised on held-out trajectories; F̂ at puzzle starts vs realised
   future solves (|bias| < 0.02 solves).
6. **Robustness sims**: truth rule ≠ model rule, uniform-truth prior, 3% excluded groups, truth outside
   the dataset, 2% user mis-entry (tests `relax1`).
7. **Determinism**: identical `Recommendation` native vs WASM and at 1/4/20 threads (fixtures in `node --test`).
8. **Latency**: headless Chrome on a 10k-state corpus sampled from simulated trajectories, 1× and 4× CPU
   throttling; gates p50 ≤ 50 ms, p99 ≤ 250 ms; no main-thread long task > 50 ms during a recommendation.
9. **Real-play replay fixtures**: the user's in-game 「记录」 screenshots are transcribed into logs
   (e.g. Q3 target 焰征龙-爆破: 💡效果 → 小丑阁下 {边框} → 废品侍从 {边框} → 紫光之宣告者 {边框} → 💡龙族 →
   龙女仆·侍应龙女 {边框, 种族} → 雪尘龙 {边框, 种族, 攻击, 守备} → 焰征龙-爆破 ✓; Q1 target 魔界剧团-美丽女主演:
   💡攻击1500 → 太阳电池人 {边框, 等级, 攻击} → 💡暗 → 娱乐伙伴 天空的魔术师 {边框, 属性, 等级, 攻击, 守备} → ✓).
   Each replay must keep the true target in the candidate set and reproduce every mask exactly.
10. **E2E** (Playwright): the https build (local static server) and the `file://` build both load, recommend,
   cancel, simulate; a missing card image falls back to the card back.

---

## 10. Milestones

Effort in focused developer-days; each milestone ends with a working tool and a git tag.

| # | milestone | deliverables | acceptance | effort |
|---|---|---|---|---|
| M1 | toolchain + skeleton + spikes | workspace, `rust-toolchain.toml`, pinned wasm-bindgen in `.tools`, `build-wasm.mjs`, `build-site.mjs`, `doctor.mjs`; spikes: Module → Blob worker on `file://` (Chrome/Edge/Firefox), `application/wasm` under `wrangler dev`, wasm size, **WASM/native speed factor** on the partition kernel | hello-engine runs over https and `file://`; measured WASM factor recorded here | 1 |
| M2 | data + rules | `mdcd data fetch/build/verify`, `cards.bin`, `RulePolicy` + tables for all variants, rule fixtures, property tests | 0 mismatches vs legacy data; legacy rule digest equal under the default policy | 2 |
| M3 | kernels + belief + reference | partition, histograms, quick metrics, equivalence classes, top-K entropy, FM lists; belief slices + rule posterior + `relax1` + ⊥; brute-force reference solver (tests) | kernels equal naive versions; research kernel timings reproduced (±50%) | 2 |
| M4 | exact tier | final-puzzle exact with caps + B&B (P̄₁, Lemma A), small-n non-final exact with F̂, c = 1 rule, work budget | identical to reference on random pools; n ≤ 16 ≤ 100 ms native | 2 |
| M5 | simulation + offline fitting | `mdcd sim` (CRN, stress presets), `baseline::legacy` (L, L3), `mdcd fit` for F̂ (default + generic grid) and the leaf (preset mixture), table headers | FV2-vs-L results of §4.5 reproduced within CI | 3 |
| M6 | online search | rungs R0/R1 (R2 behind a flag), `Recommendation` with labels/explanations/forecasts, latency corpus | v1 policy gate (§9.4) passes; native p99 ≤ 120 ms on the corpus | 2 |
| M7 | WASM + UI | `decoder-wasm`, dual loader, Worker pool, UI refactor into `web/`, bug fixes, state v5 + export/import + v4 import, rule/contradiction/forecast UI, browser 规模测试 | feature checklist (research/02 §1) passes; 活动轨迹 matches §6.1 on desktop and phone; WASM p99 ≤ 250 ms; no main-thread task > 50 ms | 3–4 |
| M8 | packaging + CI | image sprite sheets, `npm run pack`, file-count/size guard, `_headers`, GitHub Actions producing `public.zip`; first dashboard upload done by the user | `public/` < 900 files; the uploaded site works on desktop + phone with Cloudflare defaults | 1 |
| M9 | docs | Chinese README (§11), `docs/算法说明.md`, `docs/规则与假设.md`, 使用说明 merged, old docs moved to `docs/`, `.cmd` removed | README reviewed | 1 |

**Progress**

| milestone | status | result |
|---|---|---|
| M1 | ✅ done 2026-09-26 | workspace (`decoder-core`, `decoder-wasm`, `decoder-cli`/`mdcd`), `rust-toolchain.toml`, wasm-bindgen 0.2.129 in `.tools`, `tools/{build-wasm,build-site,doctor,bench-wasm,spike-browser,build-alsa-stub}.mjs`. Measured: **WASM = 1.4× native** on the partition kernel (Node and headless Chromium agree: 172–178 ms vs 124 ms for 8023 guesses × 5000 candidates; native 3.1 ns/pair, below the prototype's 2.2 ns but within budget); wasm 51 KiB (no wasm-opt); engine loads and runs in a Worker over **http:// and file://** in Chromium (base64 wasm + Blob worker + posted `WebAssembly.Module`). Rules checked against the legacy formula on all 67.4M pairs: 0 mismatches. Not yet checked: Firefox/Safari, and whether dashboard uploads honour `_headers`. |
| M2 | ✅ done 2026-09-27 | `tools/data-fetch.mjs` (ygocdb dump + YgoMaster at a pinned commit → `data/raw/`, verified by the published md5 of `cards.json` and sha256 in `data/sources.lock.json`); `mdcd data build` restores the missing builder and reproduces the legacy data **exactly** (8210 groups, 9312 cards, 0 field / weight / passcode-set mismatches), byte-identical on rebuild, MD-first order, representative = first MD passcode, invariant checks; `mdcd data compare-legacy`, `mdcd check-rules` (0 mismatches on 67.4M pairs), `mdcd card`; rule fixtures `tests/golden/rules.toml` (6 official examples + the user's Q1/Q3 real-play logs) pass. |
| M3 | ✅ done 2026-09-27 | `belief` (exact filtering, known/matched values, `relax1` contradiction ranking), `metrics` (per-field histograms → solve mass, match mass, entropy upper bound for all 8023 guesses in ≈1 ms native; exact **top-64 by entropy in 0.5–2.4 ms** vs 58–124 ms brute force, same best guess), `actions` (exact equivalence classes; bug found by the property test: the level field must never be dropped because rule-level cards share a shown level but not a printed one), `search::reference` brute-force solver, `synth` random pools, `rng`, `value`. |
| M4 | ✅ done 2026-09-27 | `search::exact`: u64-bitset nodes, collision-free u128 memo keys, per-node reduction by actual outcome, no-op pruning, zero-continuation resource caps, branch-and-bound with P̄₁ + Lemma A/S/M bounds. Equals brute force on 300 random cases (with and without pruning, with and without continuation); bound admissibility tested; equals the unpruned solver on 57 real endgames. Measured worst case on 40 sampled endgames (native): n ≤ 12 ≤ 8 ms; n ≤ 16 ≤ 14 ms with hints, 115 ms with h0 c3; n ≤ 20 ≤ 6 ms with hints, **365 ms with h0 c3** (before B&B: up to 54 s). |
| M5 | ✅ done 2026-09-27 | `cont` (F̂ table + learned cost-to-go leaf, packed `tables.bin`), `search::horizon` (depth-limited expectimax, leaf scored from cell mass/entropy without materialising sets; equals brute force at full depth), `policy` (legacy port L, F, FV2 with exact tier), `sim` (common random numbers), `mdcd sim / compare / fit-cont / fit-leaf / bench-decide / pack-tables` (rayon). Fitted F̂ (9×12×37, 290 s on 20 threads) and the leaf (139,569 samples from 4000 events on 4 presets, 43 s). **Paired results, 2000 events each (seed 99):** 9/11/36 P(all) 98.45% → **99.80%** (+1.35 ± 0.54 pts); 9/11/28 63.8% → **85.2%** (+21.4 ± 2.1); 9/4/30 14.4% → **22.6%** (+8.2 ± 1.9; E[S] +0.40). F̂ alone gives no gain (as in the research). Bug caught: sorting with the tolerance comparator panics in Rust (non-total order) — sorts now use an exact total order. |
| M6 | ✅ engine side done 2026-09-27 | `recommend` (decision + per-action stats, hint vs best challenge, ties, top candidates, forecasts), WASM API (`belief`, `revealOptions`, `judge`, `recommend`, `simulateBatch`, `columns`, `setPolicy`; JSON boundary). Latency native: p50 1.7 ms, p99 27 ms, max 409 ms (exact tier, budget 30k states); **WASM: 43.5 ms for the n = 5000 opening**. wasm 416 KiB before wasm-opt. Node tests cover the API, contradictions, the exact endgame and simulation. |
| M7 | 🟡 core done 2026-09-27 | New page in `web/` (vanilla ES modules bundled by esbuild into one classic script): state v5 (passcodes, per-puzzle logs, grant-day planning = current + future grants, undo, export/import, v4 migration via `LEGACY_PASSCODES`), main-thread engine + recommendation Worker + simulation Worker pool, reveal/challenge/feedback flows with live contradiction checks, recommendation panel (proof label, forecasts, hint-vs-challenge, alternatives, "reset the in-game filter" note), candidate table with "show more", **活动轨迹 in the in-game 「记录」 style**, test mode / mini-game, 规模测试 dialog, settings (grant schedule, milestones, R9/R10 switches, consistent-only, pool, presets with delete), WebMCP tools. `tools/build-site.mjs` builds `public/` (14 files, 2.6 MiB). `tests/e2e/smoke.mjs` (Playwright) passes over http:// and file:// (boots, filters, recommends, solves a test-mode puzzle, no page errors). Still open: Firefox/Safari check. |
| M8 | ✅ done 2026-09-27 | Generated data committed in `data/generated/` (site builds offline); `dist/` → `legacy/` with the old `.cmd` and Node tools; `tools/images.mjs` downloaded all **8210** representative images once (zh CDN, 0 missing, 23 min) → 180×262 WebP q62 → **129 sprite sheets, 67 MiB**; `tools/serve.mjs` (`npm run dev`), `tools/pack.mjs` (`npm run pack` → `build/md-card-decoder-site.zip`, **144 files, 68 MiB**, files at the zip root, deterministic), upload guard (≤ 900 files, ≤ 20 MiB each); `.github/workflows/ci.yml` (fmt, clippy, tests, thumbnails cached by data hash, build, Playwright, zip artifact, release on tags; no Cloudflare credentials). Mobile pass: fixed a 182 px horizontal overflow at 390 px and a reveal-field reset bug (`Number('') == 0`); E2E now checks phone width. |
| M9 | ✅ done 2026-09-27 | Chinese `README.md` (features, online/offline use, play-by-play workflow merged from the old 使用说明, how to read the labels, grant planning, the in-game-filter note, 活动轨迹, saves/migration, Linux development, data updates, **step-by-step Cloudflare dashboard upload** — Workers & Pages → Create application → Get started → Drag and drop your files → `<项目名>.pages.dev`; updates via “Create a new deployment”), `docs/规则与假设.md` (confirmed rules R1–R16 with the official examples, open R9/R10/R13), `docs/算法说明.md` (model, exact tier and bounds, fitted tables, online search, validation, limits). Old docs moved to `docs/legacy/`; wallpaper credits to `legacy/`. Note: the dashboard drag-and-drop flow creates a Pages project (1,000 files, 25 MiB per file), which the package satisfies. |

Total ≈ 17–19 days. M2 and M3 can overlap; M5 needs M3–M4; M7 can start after M3 with a
stub engine. The current event ends 2026-10-02 22:59 (plus 9 days from each player's first login), so the
rewrite targets the **next** event (the old tool is already online for this one, so no interim release is needed).

---

## 11. Chinese README outline (to be written in M9)

1. 简介（工具做什么；非官方声明）
2. 在线使用（网址、浏览器要求）
3. 离线使用（下载 release 压缩包，双击 `index.html`；卡图放置位置）
4. 活动规则与模型假设（六项字段、灵摆与规则等级特例（附官方范例）、提示模型、推荐卡与游戏内筛选、**仍未确认的假设 R9/R10/R13/R14/R16**）
5. 推荐怎么看（目标函数、证明/精确/近似标签、提示与挑战的比较、预测值、「任选其一」）
6. 规模测试（种子、置信区间、样本量提醒）
7. 进度迁移（导出/导入，旧版 v4 存档导入）
8. 开发（Linux：工具链、`npm run …`、目录结构）
9. 数据更新（数据源、锁文件、校验）
10. 部署到 Cloudflare（在控制台上传 `public/` 文件夹的图文步骤；无需命令行）
11. 算法说明（链接 `docs/算法说明.md`）
12. 版权与来源（低清卡图、百鸽 ygocdb、YgoMaster）
13. 更新日志

---

## 12. Risks

| risk | mitigation |
|---|---|
| research numbers were measured on a prototype that no longer exists | M3–M6 acceptance criteria re-measure them before anything depends on them |
| remaining assumptions (R9/R10, R13/R14, R16) differ in-game | `RulePolicy` switches; contradiction recovery; v2 rule mixture; ask players (§13 Q-list) |
| fitted tables are approximations of one policy's value; policy iteration may not converge | honest labels; held-out CRN gate; tables tied to data/rules/objective hashes with explicit fallback |
| leaf features too coarse (entropy + hint flag) | calibration checks; richer features only if CRN shows a gain |
| a bound bug silently returns wrong actions | brute-force reference + per-node bound assertions in CI |
| exact-tier outliers (n ≈ 20–30) | work budget + fallback to FV2 |
| WASM slower than estimated | measured in M1; FV2 has ~5× headroom against p99 |
| float non-determinism native vs WASM | `libm`, fixed summation order, explicit tolerances |
| changed advice vs the old tool (FV2 differs on ~50% of decisions, e.g. holds hints longer) | release notes + the simulation evidence in the README |
| users lose progress (index-based saves, origin change) | state v5 by passcode, v4 import map, export/import in the same release |
| copyright exposure of a public URL (card art) | low-quality thumbnails only, no wallpapers; keep the takedown notice |
| dashboard upload limits differ from the assumed 1,000 files | verified in M1; the sprite-sheet design keeps the site at ~170 files either way |
| wasm-bindgen CLI/crate drift | `=` pin + `doctor` check |

---

## 13. Decisions for the user (recommended default in **bold**)

| # | decision | default |
|---|---|---|
| D1 | objective | **LEX (as today)**; add ALL / PTS later behind a selector |
| D2 | interim legacy release (M0) | **decided: no** — the old tool is already online; only the new tool is delivered |
| D3 | keep `file://` double-click support | **yes** (dual loader) |
| D4 | card images online | **decided: low-quality WebP thumbnails**, packed into ~126 sprite sheets (§8) |
| D5 | wallpapers | **decided: dropped** from the new tool |
| D6 | consistent-cards-only setting (R15 resolved: the game allows any card after resetting its filter) | **off by default**, available as an option |
| D7 | card order in `cards.bin` | **new deterministic MD-first order** + v4 import map (legacy order cannot be reproduced) |
| D8 | build artifacts | **CI release zip; `public/` not committed** |
| D9 | deploy mode | **decided: upload the `public/` folder in the Cloudflare dashboard, no CLI**; Worker name is the user's choice (e.g. `md-card-decoder`); set up the fork remote |
| D10 | wasm-bindgen CLI source | **`cargo install --root .tools` at the Cargo.lock version** (not pacman 0.2.128) |
| D11 | Rust toolchain | **track stable with an MSRV** (not pinned 1.98.1) |
| D12 | pondering in v1 | **no** (FV2 is already fast; v2) |
| D13 | legacy analysis modes | **fold into per-action columns**; keep WebMCP |

**Questions only players can answer** (each one changes the model; answers go into `RulePolicy` defaults):
- ~~Q1 R3, Q2 R5, Q3 R7/R8, Q4 R15~~ — answered 2026-09-26 (see §3.2).
- ~~Q5 grants, Q6 milestones~~ — answered 2026-09-26 (§3.1, §3.2).
- Q7 (R9/R10) Do two *different* '?'-ATK cards match? Do two different Link monsters match on DEF?
  — **open**; default assumption kept. Can be settled from play: note any challenge where the guess and
  the target both have '?' ATK (or are both Links) and whether that row turned yellow.
- ~~Q8, Q9~~ — Q8: assume the rule value; Q9: yes, the first lightbulb column is the free initial reveal.

---

## 14. Workflow log

| when (EDT) | step |
|---|---|
| 2026-09-25 22:30 | Request: plan a Rust/WASM rewrite (algorithm improvements, Chinese README, Cloudflare hosting like `../ygo-small-world`); no coding yet. Inline scouting of the repo, docs, data profile, toolchain (cargo 1.98.1, no wasm32 target, node 26) and the sibling's wrangler setup. |
| 22:35 | Workflow 1 (8 agents, 56 min): five parallel readers — engine, UI/simulation, data/rules, toolchain/deploy, algorithm benchmarks (built a native Rust prototype; 249/249 values match the JS) — then three plan drafts (performance-, parity-, product-first). |
| ~22:50 | User direction: the implementation does not need to follow the current code; a better algorithm is welcome. → parity is dropped as a goal; legacy stays as rule oracle + baseline. |
| 23:20 | Workflow 2 (algorithm panel): three independent first-principles designs with experiments — exact DP (B&B bounds, exact lower-bound tables), approximate DP (fitted continuation + learned leaf; CRN wins over legacy), systems/robustness (rule mixture, R15, pondering, labels). |
| 2026-09-26 ~00:10 | Judges, synthesis and adversarial verifiers of workflow 2 failed (session usage limit). |
| 03:02 | Machine rebooted; `/tmp` scratchpad (prototype, reports) wiped. Reports and designs recovered from the workflow journals under `~/.claude`. |
| 03:30 | Manual judging + synthesis (§4): FV2 chosen as the main tier (strongest evidence); exact final-puzzle B&B and rule mixture adopted from the other designs; bound proofs (P̄₁, Lemma A, Lemma S) re-checked by hand; research saved to `docs/plan/research/`; this plan written. |

| 2026-09-26 | User answered the rule questions with three more official examples: R3, R5, R6–R8 confirmed exactly as the legacy code implements them; R15 resolved (any card can be picked after resetting the in-game filter). Plan updated: rule mixture demoted to v2, guesses unrestricted by default, new questions Q7–Q9. |

| 2026-09-26 | User asked for the 活动轨迹 UI in the in-game 「记录」 style (screenshots of Q1/Q3). Added §6.1 (component spec) and real-play replay fixtures (§9.9); M7 acceptance extended. |

| 2026-09-26 | User answered Q5–Q9 and D2/D4/D5/D9: grants 4c+3h on day 1, +4c+1h daily at 13:00, accumulating (= 36/11); hints never count as matches; one initial reveal; level hints assumed to show the rule value; no interim legacy release; low-quality WebP card images; no wallpapers; deploy by uploading the folder in the Cloudflare dashboard (no CLI). Plan updated: effective-inventory planning (§3.1), dashboard-upload deployment with ≤ 1,000 files via sprite sheets (§8), M0 removed, M8 reworked. |

| 2026-09-26 | Coding started on branch `rust-rewrite` (git remote `origin` removed at the user's request). M1 completed — see §10 Progress. Test browser needs `libasound`; without sudo a stub is built by `tools/build-alsa-stub.mjs` (or install `alsa-lib`). |

Next step: the user uploads `build/md-card-decoder-site.zip` to Cloudflare; open items: Firefox/Safari check, R9/R10 confirmation from play, refit tables when the card pool changes.

---

## 15. Research index (`docs/plan/research/`)

| file | content |
|---|---|
| `01-reader-engine.md` | exact spec of the legacy decision pipeline, constants, bugs, timings |
| `02-reader-ui-simulation.md` | feature-parity checklist (A–J), storage schemas, simulation protocol, WebMCP, file:// constraints |
| `03-reader-data-rules.md` | data schema, size breakdown, rule audit R1–R16, value dictionaries, data pipeline, images/wallpapers + licensing |
| `04-reader-toolchain-deploy.md` | ygo-small-world deploy anatomy, Cloudflare limits, Rust→WASM toolchain status, CI recipe |
| `05-reader-algorithm-benchmarks.md` | JS baselines, native prototype results, distinct-action counts, ranked optimisations |
| `06/07/08-draft-*.md` | the three first-round plan drafts (performance-, parity-, product-first) |
| `09-design-exact-dp.md` | decomposition proof, admissible bounds, two-sided search, exact lower-bound tables |
| `10-design-approximate-dp.md` | FV2: fitted F̂ + cost-to-go leaf, CRN experiments, offline pipeline |
| `11-design-systems-robustness.md` | rule mixture, R15, pondering, rungs/budgets, objectives, explainability, day model |

Paths inside these files that point to `/tmp/...` refer to the lost scratchpad.
