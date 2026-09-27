# Reader report: ui_sim

## Summary
I inventoried every part of the app outside the core solver: index.html, app.js (1924 lines), the worker files, presets, wallpapers, styles.css, the tools and the docs. All of the recommendation and simulation logic can move to Rust. What stays in JS is DOM glue: roughly 60 render and dialog functions, image and wallpaper handling, and storage I/O.

The UI runs only one simulation path: a classic Blob-URL Worker built from window.SIMULATION_WORKER_SOURCE. I regenerated that file in the scratchpad and it matches the committed copy byte for byte. The in-page simulation functions in app.js (simulateOneActivity, simulationPolicyDecision, pickSimulationChallenge and others, lines 1423-1658) are dead code. The docs still describe their old outputs (premium-puzzle completion rate, later-stage match points), which the worker no longer computes. The worker's policy is the same as the manual strict ("balanced") mode.

I measured a baseline in Node 26 on this machine. One strict decision near the start of a puzzle takes 0.8-1.3 s, and the depth-3 attempt always runs out of its state budget before falling back to depth 2. One simulated activity takes about 26 s, so 50 rounds is about 22 minutes and 500 rounds is about 3.6 hours.

For the rewrite, these points matter most:
- Saved logs refer to cards by their position in the cards-data.js array.
- The file:// rules (fetch is blocked, ES modules are blocked, Blob workers cannot import files) decide how WASM can be loaded.
- Card images (1-3 GB) and wallpapers (23 MB) are the main problems for hosting.
- Solver work runs on the main thread and freezes the UI.
- Search and table rendering redo work on every keystroke.

## Findings
- **In-page simulation code in app.js is dead; only the Blob worker runs**: Only the definitions exist for simulateOneActivity, pickSimulationChallenge, simulationHintEntropy, percentile, maxActivityScore and cardImageUrl. simulationPolicyDecision, filterSimCandidates and simulationStateKey are called only from simulateOneActivity. runSimulation always starts the worker. The dead version returned premiumScore/progressScore/reachedPuzzle/premiumComplete; the worker returns only solved/hintsUsed/challengesUsed/matchedItems. 使用说明.txt still promises '高价值题全通率 … 后段匹配收益' in the results, and the UI no longer produces them. [dist/app.js:1423-1658 (grep call counts: simulateOneActivity=1, pickSimulationChallenge=1, percentile=1); dist/app.js:1687-1698; 使用说明.txt 规模测试 section]
- **Worker policy matches manual strict mode**: Both use: quick pass over all weight>0 unguessed cards, sorted by [solve, newMatches, -1] with epsilon 1e-11. Exact Bellman runs only when puzzle==last, candidates<=7, hints<=4, challenges<=6, maxStates 160000, with actions deduplicated by feedback signature. Otherwise a restricted horizon runs with depth 3 then 2, maxStates 24000, actions = top 18 quick + top 8 infoApprox + top 8 weight candidates, resourceModel {0.61, 3.9}, and the fallback is quick[0]. Differences: the manual UI shows tied optimal candidates as '任选其一' and leaves the choice to the user, while the worker takes rec.action. The worker always uses strict mode; points/solve/info modes exist only in the manual UI. [dist/simulation-worker.js:20-27 vs dist/app.js:816-954,1039-1088]
- **Simulation cancel counts a partial activity**: When cancelled, activity() breaks out and returns partial counters, and onmessage still does results.push(...). The 'cancelled' summary therefore includes one truncated activity, which biases meanSolved and completion downward. [dist/simulation-worker.js:31-37]
- **Forced progress pulse lacks hintsUsed/challengesUsed**: After each round, pulse() sends current={round,puzzle,candidates:0,hints:0,challenges:0}. renderSimulationSnapshot then prints '本轮已用 undefined 提示、undefined 挑战' for at least 250 ms. The progress ratio can also overshoot before it is clamped, and the UI uses state.config.puzzles rather than the config snapshot sent to the worker. [dist/simulation-worker.js:37; dist/app.js:1663-1669]
- **Worker onerror path leaks the worker**: onerror revokes the Blob URL but never calls terminate() or sets simulationWorker=null. Terminal messages (complete/cancelled/error) do terminate and revoke correctly. [dist/app.js:1696]
- **Stale recommendation after state change during calculation**: render() calls clearRecommendation() but does not bump calculationToken. If the user records a challenge or presses undo while a calculation is running, the old result is rendered afterwards for a state that no longer exists. render() also re-enables the calculate button mid-calculation. [dist/app.js:376,388,818,835,933]
- **Solver runs synchronously on the main thread**: The quick pass yields every 80 actions, but solveExact and solveRestrictedHorizon run synchronously. Measured in Node 26 with no reveals besides the stated one: attr=DARK (2179 groups) took 297 ms for the quick pass and 1341 ms for the horizon; race=Dragon (670) took 113 + 796 ms; atk=1800 (382) took 66 + 772 ms. The depth-3 attempt always hit HORIZON_STATE_LIMIT and fell back to depth 2, so the depth-3 time is wasted. The calculate button is also enabled before any reveal, which allows an 8023x8023 quick pass. [scratchpad/perf.mjs output; dist/app.js:834-918,1079-1086]
- **Simulation throughput baseline**: 2 rounds of the 9-puzzle / 11-hint / 36-challenge preset took 52.1 s in Node 26: 84 solver calls, 0 cache hits, 66 depth-3, 17 depth-2, 1 exact, 17 hint and 67 challenge decisions. That is about 26 s per activity, so 50 rounds ≈ 22 min and 500 rounds (UI max) ≈ 3.6 h. The cache key is a string holding the full candidate list plus the sorted guessed set, rebuilt on every step (O(|C|)). [scratchpad/simrun.mjs output; dist/simulation-worker.js:19,21]
- **Saved state references cards by array index**: logs[].guess and activityHistory[].guess are indices into CARD_DATA.cards. refine-master-duel-pool.mjs changes only wm/stats, so order is preserved today. Any new data format or regenerated data that reorders groups will silently corrupt saved progress and history unless indices are kept stable or migrated by passcode (ids[0]). [dist/app.js:784,807,326,567; tools/refine-master-duel-pool.mjs:23-26]
- **Generated worker source is in sync**: I re-ran tools/build-simulation-worker.mjs in the scratchpad on copies of solver-core.js and simulation-worker.js. The output is byte-identical to the committed dist/simulation-worker-source.js. [cmp → IN_SYNC]
- **Docs reference files that are not in the repo**: 使用说明.txt says to double-click '打开求解器.html', '下载高清卡图.cmd' and '下载中文卡图.cmd'. None of them exist; only 更新大师决斗卡池.cmd does. It also says to edit wallpapers.js in 记事本 (Notepad). The error text '请重新解压完整程序' assumes zip distribution. The generator that originally built cards-data.js (grouping, names, wa) and the seeded experiment harness behind 资源价值实验报告.md are also missing, so the data cannot be rebuilt from scratch. [使用说明.txt; dist/app.js:1684; ls repo]
- **Wallpapers download even when hidden**: CSS hides .side-wallpaper at ≤980px, but rotateWallpaper still preloads with new Image() every 45 s on both sides. The 20 files total 23 MB, and individual PNGs are up to 3 MB. Mobile visitors to the hosted site would keep downloading multi-MB images they never see. [dist/styles.css:413-414; dist/app.js:211-250; du dist/wallpapers = 23M]
- **Search results show the group's representative name**: searchCards matches any alias in names[] (zh/en/ja; up to 89 aliases per group), but the result list and the input show CARDS[i].name. The image is always ids[0]. Example: searching 'Goblin King' shows '混沌死灵师'. This is logically fine because all cards in a group get identical judgements, but it is confusing. [dist/app.js:684-709,711-716; cards[0].names]
- **Candidate table silently truncates to 100 rows**: After filtering and sorting, only .slice(0,100) is rendered and nothing tells the user more rows exist. Each render re-sorts every filtered candidate with localeCompare(...,'zh-CN') in the comparator and re-normalizes every alias on every keystroke. There is no debounce and no virtualization. [dist/app.js:516-540]
- **Custom presets cannot be deleted; some UI state is not persisted**: saveCustomPreset only appends; there is no delete. recommendMode, candidate filter/sort, simulation rounds and the simulation snapshot live in memory only. testSession is never persisted, so a reload during test mode goes back to the saved real state. [dist/app.js:1317-1328,116-119]
- **Minor quirks**: resetPuzzle(keepScore) sets puzzleScore=0 in both branches. seedTestInitialReveal calls appendHistory before render, so the history 'candidates' value is the previous real state's mass. Editing puzzleNumber changes the number without resetting clues. The worker would crash on cards[undefined] if the solver ever returned a hint action when hints==0 or knownMask==63; the solver presumably never does. Math.random is unseeded everywhere, so simulations cannot be reproduced. [dist/app.js:1835-1837,1357-1366,1736-1741; dist/simulation-worker.js:33-34]

## Spec
# Feature inventory outside the solver, and how the simulation (规模测试) works

Scope: everything in `dist/` except the solver internals in `dist/solver-core.js`. File references are to `/home/parry/gitRepos/md-card-decoder/dist/app.js` unless another file is named.

---

## 1. Feature-parity checklist

### A. Top bar: event state and tools
| # | Feature | Implementation |
|---|---|---|
| A1 | Editable **题目 / 提示 / 挑战** inputs. Values are clamped to min (0 or 1) and max (puzzles; no max for hints/challenges). Each edit pushes an undo entry, then calls render() | `bindEvents` 1736-1741; `render` 354-358 |
| A2 | Read-only **高价值** (premiumScore) and **后段匹配** (progressScore) outputs | `render` 359-360 |
| A3 | **活动设置** opens the settings dialog | `openSettings` 1295 |
| A4 | **界面** theme select (dark/light). Sets `html[data-theme]` and is persisted | 1757-1761, `render` 372-373, `styles.css` 339-400 |
| A5 | **卡图** image-quality select (`high` = English HD .jpg, `zh` = Chinese .webp that falls back to HD). Persisted | 1762-1771, `refreshImageQuality` 610-617 |
| A6 | Buttons for **测试模式**, **小游戏模式** and **规模测试**. The two mode buttons are disabled while a session runs | `renderTestMode` 444-450 |

### B. Current puzzle panel (当前题状态)
| # | Feature | Implementation |
|---|---|---|
| B1 | Six field cards. Each shows the known value, its source label (初始揭示/提示揭示/挑战相符) and a matched badge | `renderFields` 392-402, `sourceLabel` 347 |
| B2 | Field glyphs. The frame glyph is a colour gradient built from the frame bits. The attribute glyph shows its label. The level glyph changes shape when the known border is Link (32) or Xyz (16). Race, ATK and DEF use SVG icons | `fieldGlyphHtml` 404-425 |
| B3 | Puzzle-score chip and candidate pill. The pill shows the **weighted card count** (sum of wm or wa) | `render` 361,367 |
| B4 | Reward track. A premium puzzle shows its milestones plus a 全中 node; a regular puzzle shows 6 × regularMatchPoints. The progress bar stops at 94% until the puzzle is solved. A summary line sits below | `renderRewards` 427-442 |
| B5 | Tab switching between **录入揭示** and **记录挑战** | `switchTab` 1242-1250 |
| B6 | Reveal entry. The field list contains unknown fields only. The value list holds values present among current candidates, with weighted counts, sorted numerically for number/atk/def and by zh collation otherwise. Source is `initial` (once per puzzle) or `hint` (needs stock) | `renderRevealControls` 476-490, `populateRevealValues` 492-506, `addReveal` 635 |
| B7 | `applyReveal` validation: the field must still be unknown; source rules apply; a clue that would leave zero candidates is rejected. On success it pushes undo, writes `known`, consumes a hint when needed, and writes a log and history entry | `applyReveal` 660-678 |
| B8 | **随机提示**. In real mode it picks a random unknown field, sets source=hint, and asks the user to enter the in-game value; no hint is consumed until they add it. In test/game mode it reveals the target's value right away | `randomHint` 646-658 |
| B9 | Card search. Matches the query against any alias (zh/en/ja) after normalization (lowercase, strip whitespace and `·・-—_/／「」『』`). Only weight>0 groups in the current pool are searched. A prefix match ranks above a substring match, then shorter names win; the top 24 are shown | `normalizeSearch` 680, `searchCards` 684-700, `showSearchResults` 702-709 |
| B10 | Selected-card panel with image, the 6 field facts, and a clear button | `selectGuess` 711-722, `clearGuess` 724-733 |
| B11 | **使用这张卡挑战**. In real mode it opens the feedback dialog; in test/game mode it auto-judges | `beginChallenge` 766-773 |
| B12 | Feedback dialog. Six toggles, each showing the guess's value for that field. If frame or level is toggled, a select appears for the full frame or the level the game revealed. Its options come from candidates compatible with the guess, and it defaults to the known value, else the guess's value | `renderFeedback` 735-746, `populateSpecialReveal` 748-764 |
| B13 | `recordChallenge`: rejects a card already challenged in this puzzle (the game does not charge for it), rejects a zero-candidate result, scores with `thresholdGain` + `solveReward`, decrements challenges, writes `known[field]={value, source:'challenge'}` (frame and level use the reveal selects), logs, writes history, shows a toast | 775-814, `thresholdGain` 340-345, `solveReward` 160 |
| B14 | Solved banner with 进入下一题 / 完成活动 | `render` 378-380, `nextPuzzleBtn` 1794-1799 |

### C. Recommendation panel (推荐行动)
| # | Feature | Implementation |
|---|---|---|
| C1 | Mode select: `balanced` (strict lexicographic), `points`, `solve`, `info`, with guide text. Changing the mode clears the result and recalculates automatically | 1723, `renderModeGuide` 1206 |
| C2 | Calculate button with a progress line (0-70% quick pass, 70-100% finalist branches) and a cancellation token | `calculateRecommendations` 816-954 |
| C3 | Result card with title, image, reason, stats chips, three metrics (期望即时得分 / 本次通关概率 / 信息增益 bit), 3 alternatives or up to 8 tied choices (任选其一), and a method note. Clicking the title or an alternative selects that card in the challenge tab and scrolls to the top | `renderRecommendation` 1153-1196, `recommendationReason` 1198, handlers 1724-1735 |
| C4 | Engine helpers that live in app.js: `boundedCertificate`, `expectedHintEntropy`, `exactActionRepresentatives`, `tryExactBellman`, `tryRestrictedPolicy`, `decideHintUsage` (heuristic, non-strict modes only), `hintAdvice`, `weightedCandidateEntropy`, `binaryEntropy`, `compareObjectiveVectors` | 956-1122, 1216 |

### D. Candidates and history panel (候选与历史)
| # | Feature | Implementation |
|---|---|---|
| D1 | Pool select, `md` or `all`, undoable. Shows a caveat text that includes the data date | 1742-1744, `render` 363-366 |
| D2 | Undo, 30 levels deep, storing full deep clones of state | `pushUndo` 252, 1772-1775 |
| D3 | Reset puzzle (confirm). Subtracts puzzleScore from the totals; in a session it reseeds an initial reveal | 1776-1785, `resetPuzzle` 1835 |
| D4 | Candidate table: alias search, field+value filter (the value list comes from candidates), five sorts (probability, name, numberAsc, attackDesc, defenseDesc), **top 100 rows only**, "显示 X / Y" counter, tooltip listing all aliases | `renderCandidates` 508-541, `refreshCandidateFilterValues` 543-559 |
| D5 | Activity trail: the newest 8 entries of `state.activityHistory`, with thumbnails, match chips and +delta | `renderHistory` 579-589, `historyItemHtml` 561-571, `hydrateCardImages` 573 |
| D6 | Archive dialog showing sessionStorage history for this window. Summary: number of activities, challenges and operations; below it the full expanded timeline. 清除旧活动档案 keeps only the current activity's history | `openHistoryArchive` 591-599, 1752-1756 |
| D7 | Footer with data stats (MD records/groups, all records, source) and 重置整个活动 (confirm, undoable, keeps config and presetId) | `render` 369-370, 1786-1793 |

### E. Settings and presets
| # | Feature | Implementation |
|---|---|---|
| E1 | Preset select: built-in `window.ACTIVITY_PRESETS` + custom presets from localStorage + "自定义当前设置" | `renderPresetOptions` 1289, `allPresets` 150, `dist/presets.js` |
| E2 | Seven numeric fields plus a milestone editor (add/remove rows; matches 1-5; deduplicated and sorted) | `fillSettingsForm` 1269, `configFromForm` 1252, `addMilestoneRow` 1282, `normalizeConfig` 31-50 |
| E3 | Save a custom preset (append only, **no delete**) | `saveCustomPreset` 1317 |
| E4 | Apply (confirm, needs at least one milestone): starts a fresh activity | `applySettings` 1302 |

### F. Test mode and mini-game
| # | Feature | Implementation |
|---|---|---|
| F1 | Start a session: snapshot the real state, draw a random weighted target, start a fresh state with the same config/pool/preset, seed one random initial reveal, clear undo. `saveState` is suppressed while the session runs | `startSession` 1341, `randomWeightedIndex` 1330, `seedTestInitialReveal` 1357, `saveState` 116 |
| F2 | Banner: in test mode the target is shown; in game mode a card back is shown until 放弃并揭晓 or the solve. Buttons: auto-judge, new target, exit | `renderTestMode` 444-474, `newTestTarget` 1368, `revealSessionTarget` 1398, `exitTestMode` 1384 |
| F3 | Auto-judge: mask = `matchMask(target, guess)`; the frame/level reveals are set from the target; then `recordChallenge` runs | `autoJudgeChallenge` 1404 |
| F4 | Next puzzle inside a session draws a new target and a new initial reveal. Exiting restores the real state but keeps theme and imageQuality from localStorage | 1797, 1384-1396 |

### G. Scale test (规模测试): see §3
`openSimulation` 1414, `runSimulation` 1672-1699, `renderSimulationSnapshot` 1660-1670, `dist/simulation-worker.js`, `dist/simulation-worker-source.js`.

### H. Media and chrome
| # | Feature | Implementation |
|---|---|---|
| H1 | Card image paths `card-images-high/{id}.jpg` and `card-images-zh/{id}.webp`, using `ids[0]`. On error it tries the next URL, then a fallback, then hides the image. `data-zoomable` flag | `cardImageUrlsForId` 175, `setCardImage` 187-209 |
| H2 | Image viewer dialog: click any zoomable image; close with Esc, the × button, or a backdrop click | `openImageViewer` 601, 1824-1832 |
| H3 | Side wallpapers: 20 images, no repeat between the two sides, preloaded crossfade, rotation every 45 s with the right side 1.4 s later. CSS hides them at ≤980px, but JS keeps loading them | `rotateWallpaper` 211, `startWallpaperCycle` 242, `dist/wallpapers.js`, `styles.css` 41-46,413 |
| H4 | Toast (3.2 s), ambient background blobs, clicking outside closes search results, responsive breakpoints at 980/700/430 px, native `confirm()` prompts | 257, 1828, `styles.css` 413-461 |

### I. WebMCP: see §4 (`registerWebMcpTools` 1877-1918)

### J. Maintainer tooling (Node)
- `tools/update-master-duel-pool.mjs` (launched on Windows by `更新大师决斗卡池.cmd`) downloads YgoMaster `CardList.json` + `YdkIds.txt`, then runs `refine-master-duel-pool.mjs`.
- `tools/refine-master-duel-pool.mjs` recomputes `wm` for each group, plus `stats.masterDuelMonsterRecords/Groups`, `generatedAt` and `source`. **Group order is preserved.**
- `tools/download-card-images.mjs --quality=low|high|zh [--dry-run]` downloads images for every id in wm>0 groups: 3 concurrent requests, 700 ms delay, resumable. `low` → `dist/card-images/`, a folder the app no longer uses.
- `tools/build-simulation-worker.mjs` concatenates solver-core + worker (with `importScripts` stripped) into `window.SIMULATION_WORKER_SOURCE`.
- **Not in the repo:** the original cards-data generator (grouping, 百鸽/YGOPRODeck names, `wa`), the seeded experiment harness for 资源价值实验报告.md, and the `打开求解器.html` / `下载*卡图.cmd` files the docs mention.

### Dead code to drop (not user-visible)
`weightedSample` (app), `simulationWeight`, `filterSimCandidates`, `simulationHintEntropy`, `pickSimulationChallenge`, `simulationStateKey`, `simulationPolicyDecision`, `simulateOneActivity`, `percentile` (app) [1423-1658], `maxActivityScore` [169], `cardImageUrl` [183].

---

## 2. Browser storage

| Key | Store | Written by | Content |
|---|---|---|---|
| `card-decoder-state-v4` | localStorage | `saveState` on **every** `render()`; skipped while `testSession` is active | the full `state` object (below) |
| `card-decoder-custom-presets-v1` | localStorage | `saveCustomPreset` | `Array<{id:"local-<ms>", name:string, config:Config}>`; append only; config is normalized only when read into the form |
| `card-decoder-theme` | localStorage | theme select | `"dark" \| "light"`; also copied into `state.theme` |
| `card-decoder-image-quality` | localStorage | quality select | `"high" \| "zh"`; also copied into `state.imageQuality` |
| `card-decoder-session-history-v1` | sessionStorage (per tab, gone on close) | `appendHistory` | `Array<HistoryRecord>`, last 500 |

### `Config` (after `normalizeConfig`)
```ts
{ puzzles: 1..99 (default 9), totalHints: 0..999 (11), totalChallenges: 0..999 (36),
  premiumPuzzles: 0..puzzles (3),
  milestones: Array<{matches:1..5, points:0..99999}> // sorted, deduplicated by matches; default [{1,10},{3,10},{5,10}]
  solvePoints: 0..99999 (70), regularMatchPoints (1), regularSolvePoints (0) }
```
Legacy config fields that are still accepted: `days`, `initialHints`, `dailyHints`, `initialChallenges`, `dailyChallenges`. If `totalHints` or `totalChallenges` is missing, the value is computed as `initial + (days-1)*daily`.

### `State` (v4)
```ts
{ config: Config, presetId: string, puzzle: int, hints: int, challenges: int,
  totalScore, premiumScore, progressScore, puzzleScore: int,
  pool: 'md'|'all',
  known: { [field in 'border'|'attribute'|'race'|'number'|'attack'|'defense']?: {value:number, source:'initial'|'hint'|'challenge'} },
  matchedMask: 0..63, // bit order: border1 attr2 race4 number8 atk16 def32
  logs: Log[],               // current puzzle only
  activityHistory: HistoryRecord[], // whole activity, capped at 240
  activityId: "activity-<ms>", initialUsed: bool, solved: bool,
  theme: 'dark'|'light', imageQuality: 'high'|'zh' }
Log = {type:'reveal', field, value:number, source:'initial'|'hint', time:"HH:MM"}
    | {type:'challenge', guess:<CARD INDEX>, mask:0..63, borderReveal:number|null, numberReveal:number|null, delta:int, time}
HistoryRecord = Log & { id:"<ms>-<rand5>", activityId, puzzle, mode:'activity'|'test'|'game', timestamp:ms, candidates:<weighted mass BEFORE the op> }
```
Encodings: `known.border.value` is the full frame bitmask (for example 132 = Fusion|Pendulum). Attribute and race values are single bits. `number` is `card.n`. ATK/DEF use −2 for `?` and −3 for "Link, no DEF".

### Migration logic (`loadState` 91-114)
1. Parse the v4 key. On failure, or if the value is not an object, start a fresh state. **Keys v1-v3 are never read.**
2. `merged = {...freshState(saved.config), ...saved, config: normalizeConfig(saved.config)}`.
3. Old daily-grant saves: if `saved.config.totalHints == null` and `saved.day != null`, then `remainingDays = days - day` and `hints = min(totalHints, saved.hints + remainingDays*dailyHints)`. Challenges are handled the same way.
4. If `activityHistory` is missing, build it from `logs` (`id:"migrated-i"`, `mode:'activity'`).
5. If `premiumScore` is missing, use `saved.totalScore`. If `progressScore` is missing, use 0.

### Notes for the rewrite
- **`guess` is an index into `CARD_DATA.cards`.** A new data format must keep group order stable, or migrate each index by passcode (`cards[i].ids[0]`).
- Moving from file:// to https changes the origin, so users start with empty storage. Plan an **export/import JSON** feature, and have v5 accept a v4 import.
- The theme and image-quality settings are stored twice (their own key and inside state). Pick one place.

---

## 3. The scale test (规模测试)

### Main-thread lifecycle (`runSimulation` 1672-1699)
1. If a run is active, the button acts as stop: `postMessage({type:'cancel'})` and the label becomes 正在停止….
2. Inputs: `rounds = clampInt(#simulationRounds, 1, 500, 50)`, `pool = #simulationPool` (defaults to `state.pool`), `config = normalizeConfig(state.config)`.
3. If `window.SIMULATION_WORKER_SOURCE` is missing, it shows "请重新解压完整程序".
4. **Worker creation:** `URL.createObjectURL(new Blob([SIMULATION_WORKER_SOURCE], {type:'text/javascript'}))` → `new Worker(blobUrl)`, a classic worker. This exists because `new Worker('simulation-worker.js')` and `importScripts` fail under file:// (opaque origin). The generated source is `solver-core.js` + `simulation-worker.js` with the `importScripts` line removed; `solver-core` attaches itself to `globalThis`, which is `self` in a worker.
5. It sends `{type:'start', cards: CARDS.map(({b,a,r,n,nm,atk,def,wm,wa})=>…), config, pool, rounds}`: 8210 small objects, structured-cloned, without names or ids.
6. `onmessage`: stores `simulationSnapshot` and calls `renderSimulationSnapshot`. On `complete`, `cancelled` or `error` it calls `terminate()`, revokes the URL, and nulls both references. `onerror`: shows the error and revokes the URL, but **does not terminate the worker**.
7. Closing the dialog does not stop the run. Reopening it re-renders the last snapshot (`openSimulation` 1414).

### Message protocol
- Main → worker: `{type:'start', cards, config, pool:'md'|'all', rounds}` and `{type:'cancel'}`.
- Worker → main:
  - `{type:'ready', completed:0, total, current, summary}`
  - `{type:'progress', completed, total, current, summary}`, throttled to one per 250 ms, plus one forced message after each round
  - `{type:'complete'|'cancelled', completed, total, summary}`
  - `{type:'error', message: stack}`
- `current = {round, puzzle, candidates, hints, challenges, hintsUsed, challengesUsed}`. The forced end-of-round pulse omits `hintsUsed` and `challengesUsed`, which shows up as "undefined" in the UI.

### How one activity is simulated (`activity`, worker lines 30-36)
- Resources start at `totalHints` and `totalChallenges` for the whole event. There is no per-day modelling.
- For each puzzle, while challenges remain and the run is not cancelled:
  - Draw a target by weight (`wm` or `wa`) from the universe (weight>0).
  - Pick the initial reveal field **uniformly from the 6 fields**. `candidates` = universe cards whose value equals the target's.
  - Loop while challenges remain and the puzzle is unsolved:
    - Send a progress pulse and **yield with `setTimeout(0)`**. This yield is the only point where a cancel message can be handled.
    - `rec = decision(...)`.
    - `stop` action: break. The puzzle stays unsolved, so the activity ends.
    - `hint` (only if hints>0 and knownMask≠63): reveal a field chosen uniformly from the **unknown** fields and filter by the target's value.
    - Otherwise `challenge`:
      - `mask = matchMask(target, guess)`
      - `matchedItems += newly matched bits`
      - `knownMask |= mask`, so matched fields cannot be drawn by a later hint
      - `challenges--`
      - `mask==63` solves the puzzle; otherwise filter by `(matchMask(card,guess)==mask) ∧ (mask&1 ⇒ card.b==target.b) ∧ (mask&8 ⇒ card.n==target.n)`.
  - An unsolved puzzle ends the whole activity.
- Result per activity: `{solved, hintsUsed, challengesUsed, matchedItems}`. **Scores are not tracked.**

### Per-step policy (`decision`, lines 20-27): the same as manual strict mode
1. Cache lookup. The key is the string `puzzle|hints|challenges|knownMask|matchedMask|candidates.join(',')|sorted guessed`, kept in an LRU-ish Map capped at 4000 entries (oldest insertion evicted).
2. Quick pass over every unguessed universe card: `solve` mass, expected new matches, `infoApprox` (sum of binary entropies per bit). Sorted by `compare([solve,newMatches,-1])` with epsilon 1e-11.
3. Exact Bellman (`solveExact`, maxStates 160000) when `puzzle==config.puzzles && |C|≤7 && hints≤4 && challenges≤6`. Actions are deduplicated by the feedback signature `mask:b?:n?` over the candidates. Config is passed as `{...config, puzzles: puzzle}`.
4. Otherwise `solveRestrictedHorizon`: first depth 3, and on `HORIZON_STATE_LIMIT` depth 2. maxStates 24000. Action set = quick top 18 ∪ infoApprox top 8 ∪ the 8 heaviest candidates. `remainingPuzzles = puzzles-puzzle+1`, `resourceModel {hintChallengeRatio:.61, equivalentCostPerSolve:3.9}`.
5. Fallback: challenge `quick[0]`.
6. Counters updated along the way: solverCalls, cacheHits, exactCalls, depth3Calls, depth2Calls, fallbackCalls, hintDecisions, challengeDecisions.

**Comparison with manual (`calculateRecommendations` in balanced mode):**
- Identical: thresholds, action selection, candidate ordering (ascending index, so floating-point sums come out the same), and the fallback. When the manual path has neither an exact nor a restricted result it recommends `finalists[0]`, which is the same as `quick[0]` because the sort is stable.
- Different:
  - (a) Manual mode shows `equivalentChoices` ties and lets the user pick; the simulation takes `rec.action`.
  - (b) Manual mode also computes `points` (milestones and solve rewards), used only in the non-strict modes and for display.
  - (c) The simulation pool can differ from `state.pool`.
  - (d) Manual code logs unexpected solver errors to the console and continues; the worker rethrows, and the run ends with `error`.

### Cancellation
- A flag is checked between decisions, so stop latency is one solver call (about 1 s early in a puzzle).
- **Bug:** the partially simulated activity is pushed into `results`, so the `cancelled` summary is biased.

### Summary statistics (`summarize`, lines 9-15)
- `count`; `meanSolved`.
- `solvedLow/High = mean ∓ 1.96·sqrt(s²/n)` (sample variance, n−1), clamped to [0, puzzles].
- `p10/p50/p90`: `sorted[round((n−1)·f)]` (nearest rank).
- `completion p` = share of activities with solved==puzzles. **Wilson 95%:** `center=(p+z²/2n)/(1+z²/n)`, `half=z·sqrt(p(1−p)/n+z²/4n²)/(1+z²/n)`, z=1.96.
- Averages: `matchedItems` (first-time matches from challenges only), `hintsUsed`, `challengesUsed`.
- `distribution` = histogram of solved counts, zero bins dropped. Plus `diagnostics` and `elapsed` in seconds.
- `renderSimulationSnapshot` shows six tiles, the histogram and a disclaimer. The progress ratio uses the live `state.config.puzzles`, not the config snapshot sent to the worker.

### Docs versus code
使用说明.txt promises 高价值题全通率, a conditional average puzzle reached, and 后段匹配收益. Those outputs came from the dead in-page `simulateOneActivity`, which returned premiumScore, progressScore, reachedPuzzle and premiumComplete. The worker does not compute them. Decide whether the rewrite restores them (cheap in Rust) or the docs drop them.

### Measured baseline (Node 26, this machine, preset 9/11/36, pool md)
- 2 activities took 52.1 s: 84 solver calls, 0 cache hits, 66 depth-3, 17 depth-2, 1 exact, 0 fallback, 17 hint and 67 challenge decisions, all 9/9 solved.
- That is about 26 s per activity: 50 rounds ≈ 22 min, 500 rounds ≈ 3.6 h.
- Rounds are independent, so a pool of N workers each running a shard of rounds scales linearly with no SharedArrayBuffer needed.
- Add a **seeded RNG** and report the seed; today `Math.random` makes runs impossible to reproduce.

---

## 4. WebMCP tools (`registerWebMcpTools` 1877-1918)
The feature is detected through `document.modelContext?.registerTool`, and every registration swallows errors. The current WebMCP drafts may expose this as `navigator.modelContext` instead (see open questions).

| name | title | inputSchema | annotations | execute → returns |
|---|---|---|---|---|
| `read_solver_state` | 读取求解器状态 | `{type:'object', properties:{}, additionalProperties:false}` | readOnlyHint:true | `solverSummary()` = `{puzzle, hints, challenges, totalScore, premiumScore, progressScore, premiumPuzzle, puzzleScore, matchedFields, solved, candidateCards (weighted), candidateGroups, known:{field: formatted label}}` |
| `add_revealed_clue` | 录入揭示字段 | `{field: enum[border,attribute,race,number,attack,defense], value:number, source: enum[initial,hint]}`, all required, no extra properties | readOnlyHint:false | `applyReveal(...)`, which throws the Chinese validation messages; on success returns `solverSummary()` |
| `calculate_best_challenge` | 计算最佳挑战 | empty object | readOnlyHint:true (it still updates the UI) | `recommendationSummary()` = `{action:'use_hint'\|'challenge', card: name, expectedImmediatePoints (3 dp), solveProbability (6 dp), informationBits (3 dp), hintInformationBits}`; throws `当前状态无法计算推荐。` |

There is no tool for recording a challenge or for test/sim modes. `value` uses the raw numeric codes (bitmasks, −2/−3).

---

## 5. Windows and file:// assumptions, and what changes

| Assumption | Where | On Linux / served over HTTP / with WASM |
|---|---|---|
| The page is opened by double-clicking `dist/index.html` (the docs name a nonexistent `打开求解器.html`) | README, 使用说明.txt | Works in Linux browsers as well. From WSL the URL is `file://wsl.localhost/...`, a different storage origin from a Windows path. For development, serve over HTTP (`wrangler dev`, `python -m http.server`). |
| All scripts are classic `<script src>` tags that expose globals (`window.CARD_DATA`, `ACTIVITY_PRESETS`, `CARD_DECODER_WALLPAPERS`, `DecoderSolver`, `SIMULATION_WORKER_SOURCE`) | index.html:134 | Needed under file:// because Chromium blocks `<script type="module">` from file:// (origin null, CORS). Over HTTP, ES modules are fine. |
| The worker is created from a Blob URL holding embedded source | app.js:1687; build-simulation-worker.mjs | Needed under file://. A Blob worker cannot `importScripts` or `import` relative file:// URLs, and its base URL is `blob:`. Over HTTP use `new Worker(new URL('./sim-worker.js', import.meta.url), {type:'module'})`. |
| **WASM under file://** | (new) | `fetch()` does not support the file: scheme in Chromium, and XHR to file:// is blocked in Chromium and in Firefox ≥68. So `instantiateStreaming(fetch('x.wasm'))` and wasm-bindgen `--target web` default init fail. A file:// build would need: wasm-bindgen `--target no-modules` (classic global), wasm bytes **base64-embedded in a classic .js**, async `WebAssembly.compile(bytes)` (Chrome forbids synchronous compile above 8 MB on the main thread), and the compiled `WebAssembly.Module` posted to the Blob worker, which calls `initSync({module})`. Modules can be structured-cloned to dedicated workers. Over HTTP none of this is needed: Cloudflare static assets serve `.wasm` as `application/wasm`, and module workers work. |
| WASM threads (rayon / SharedArrayBuffer) | (new) | These need COOP `same-origin` + COEP `require-corp` or `credentialless`. On Cloudflare that is set through `_headers`. `require-corp` breaks cross-origin card images that lack a CORP header, which applies to hotlinked ygoprodeck/百鸽 images. The recommended alternative is N independent workers (rounds are independent), which needs no cross-origin isolation. |
| Card images live at relative `card-images-high/{id}.jpg` and `card-images-zh/{id}.webp`, installed by the user from a release zip and gitignored | app.js:175-181, .gitignore, README | These cannot simply be deployed: about 9059 ids × 2 formats ≈ 18k files and 1-3 GB, close to the Workers static-asset limit of 20,000 files per version on the free plan (25 MiB per file). Options: (1) small WebP thumbnails, one format, about 9k files and 100-150 MB; (2) R2 bucket plus a custom domain; (3) hotlinking. YGOPRODeck asks API users not to hotlink, and the project docs say "不会持续热链接". The code should take a configurable `IMAGE_BASE` URL. The sibling project ygo-small-world ships a few webp files under `public/pics` with a 30-day `Cache-Control` in `_headers`. |
| Wallpapers: 23 MB, 20 files, fan art mixed with official art; the docs say to edit `wallpapers.js` in 记事本 (Notepad) | wallpapers.js, 壁纸来源与授权.txt | Public hosting widens redistribution, so review the licensing risk (the file itself says 署名不构成授权). Convert to WebP/AVIF and stop preloading when hidden (see §6). |
| `更新大师决斗卡池.cmd` (`chcp 65001`, `cd /d %~dp0`, `pause`) | repo root | Does not run on Linux. Use an npm script or shell script (`node tools/update-master-duel-pool.mjs`) or a `cargo xtask`. The tools rely on `import.meta.dirname` (Node ≥20.11), global `fetch` and `os.tmpdir()`, all fine on Node 26. Line endings are already LF. |
| Zip distribution (`*.zip` gitignored; the error text says 重新解压完整程序) | app.js:1684 | Online this becomes "resource failed to load". An offline replacement: a PWA service worker that caches the HTML, JS, WASM and data after the first visit, which removes the need for file://. |
| localStorage under file:// | app.js:6-8 | In Chromium, every file:// page shares one storage bucket. A hosted origin starts empty, so export/import is needed. |
| `document.modelContext` WebMCP | app.js:1878 | Independent of origin. The API name should be verified. |
| Native `confirm()` and `<dialog>.showModal()` | various | Work over HTTP; nothing specific to any OS. |

---

## 6. UI performance hot spots for the hosted version
Measurements are from Node 26 on this machine; browser figures will be similar or slower.

1. **Recommendation runs on the main thread.**
   - The quick pass is 8023 actions × |C| (≈12M `matchMask` calls for |C|≈1500) plus `thresholdGain` per pair: 297 ms at |C|=2179. It yields every 80 actions through setTimeout(0).
   - The finalists pass (≈340 actions × |C|) builds outcome maps.
   - `solveRestrictedHorizon` then runs **synchronously**: 0.8-1.3 s of freeze. The depth-3 attempt always hits its state limit before depth 2 runs, so that time is wasted.
   - `solveExact` can expand up to 160k states.
   - The calculate button is enabled before any reveal, which allows 8023×8023.
   - Fix: move the solver into a worker running WASM, keep the token-based cancellation, and bump the token inside `render()`.
2. **`render()` is monolithic** and runs after every change:
   - `candidateIndices` scans all 8210 groups × (known + logs).
   - Every component is rebuilt through `innerHTML`.
   - `saveState` stringifies the whole state including 240 history entries.
   - Fine for correctness, but split it into targeted updates or a small reactive store.
3. **Candidate table:**
   - Every filtered candidate (up to 8023) is sorted with `localeCompare(..., 'zh-CN')` inside the comparator; in Node this took 32 ms for the probability sort. In Chrome, `localeCompare` with a locale argument is often much slower than a cached `Intl.Collator`.
   - Precompute a name rank once, or use `new Intl.Collator('zh-CN')`.
   - Only 100 rows are rendered, with no indication; use a virtual list or explicit paging.
   - Each keystroke re-normalizes every alias of every candidate, with no debounce (about 7 ms).
4. **Card search:**
   - Each keystroke normalizes all 39,580 MD aliases (434k characters) and runs indexOf: about 6 ms per query in Node, with no debounce.
   - Fix: pre-normalize once at load, or build an n-gram or suffix index in Rust/WASM.
   - Consider showing the **matched alias** instead of the group's representative name, and optionally pinyin search.
5. **Data payload:**
   - `cards-data.js` is 2.08 MB of JS parsed synchronously. `names` are about 570k UTF-16 units of about 1.4M; the numeric fields are tiny (8210 × ~12 B ≈ 100 KB as columnar binary).
   - Online: brotli (automatic on Cloudflare); split into a compact binary or columnar core for WASM and a names/search blob loaded lazily; add a content hash to filenames for long-term caching.
6. **Wallpapers:**
   - JS preloads a 0.1-3 MB image for both sides every 45 s **even at ≤980px**, where CSS hides them. There is no `loading="lazy"`, no `matchMedia` check and no `prefers-reduced-data` handling.
   - Fix: gate on `matchMedia('(min-width: 981px)')`, convert to WebP/AVIF, and pause when the page is hidden (`visibilitychange`).
7. **Archive dialog:** it can render 500 history items, each with a card `<img>`, all eager. Add `loading="lazy"` and `decoding="async"` and virtualize.
8. **Simulation:** about 26 s per activity (§3). The cache key string costs O(|C|) per step and got 0 hits in 2 rounds. In Rust, use a 64-bit hash of a sorted candidate bitset. Run one worker per core over shards of rounds.
9. **Undo:** keeps up to 30 deep JSON clones of state. Cheap, but it could store diffs.

---

## 7. Pure logic (move to Rust) versus DOM glue (keep in JS/TS)

**Pure and portable to Rust/WASM** (no DOM; `Math.random` becomes an injected seeded RNG):
- Data model and rules: `FIELDS`, `matchMask`, `popcount`, `fieldValue`, `weightOf`, `candidateIndices`, `candidateMass`, `filterCandidates`.
- Scoring: `isPremiumPuzzle`, `solveReward`, `maxPuzzleScore`, `maxActivityScore`, `thresholdGain`.
- Config and state: `clampInt`, `normalizeConfig`, `freshState` (without the localStorage reads), the v4 migration in `loadState` (a JSON-to-struct step inside `migrate_v4()`), `resetPuzzle`, next puzzle, reset event, undo stack (optional).
- Transitions and validation: `applyReveal`, `recordChallenge` (the duplicate, zero-candidate and scoring checks), `randomHint` field choice, `randomWeightedIndex`, `seedTestInitialReveal`, `newTestTarget`, `autoJudgeChallenge` feedback generation, test-session snapshot and restore.
- Derived lists for the UI: reveal value histogram (`populateRevealValues`), `refreshCandidateFilterValues`, `populateSpecialReveal` value sets, candidate filter/sort/page, `normalizeSearch` + `searchCards` (plus an index).
- Recommendation: the whole data path of `calculateRecommendations` (quick pass, finalists, outcome entropy, policy call, tie detection), `boundedCertificate`, `expectedHintEntropy`, `exactActionRepresentatives`, `tryExactBellman`, `tryRestrictedPolicy`, `decideHintUsage`, `hintAdvice`, `weightedCandidateEntropy`, `binaryEntropy`, `compareObjectiveVectors` (keep epsilon 1e-11 and the summation order for golden-test parity).
- Simulation: `activity`, `decision`, `weightedSample`, `summarize` (Wilson, percentiles, histogram), plus the restored premium and progress metrics if wanted.
- Summaries: `solverSummary` and `recommendationSummary` as data; the formatting can stay in JS.
- Formatting that can go either way: `formatBorder`, `formatStat`, `formatValue` and `cardStats` depend on `DATA.labels`. They are pure, but keeping them in JS makes i18n easier.

**DOM glue (stays in JS/TS):**
- Rendering: `render`, `renderFields`, `fieldGlyphHtml`, `renderRewards`, `renderTestMode`, `renderRevealControls`, `renderCandidates` (markup only), `renderHistory`, `historyItemHtml`, `hydrateCardImages`, `renderRecommendation`, `recommendationReason`, `renderModeGuide`, `renderFeedback`, `renderSimulationSnapshot`.
- Media and dialogs: `setCardImage` (fallback chain), `openImageViewer`, `refreshImageQuality`, `rotateWallpaper`, `startWallpaperCycle`, `toast`, `switchTab`, the settings dialog (`openSettings`, `fillSettingsForm`, `configFromForm`, `addMilestoneRow`, `renderPresetOptions`, `applySettings`, `saveCustomPreset`), `openHistoryArchive`, `openSimulation`, `bindEvents`.
- Plumbing: worker lifecycle (`runSimulation`), storage I/O (`saveState`, `readSessionHistory`, `appendHistory` persistence, the theme and quality keys), `registerWebMcpTools`, `timeLabel`, `formatPercent`, `escapeHtml`, `escapeAttr`, `nextFrame`, `confirm()`.

**Suggested WASM boundary (coarse-grained, so few crossings):**
- `Engine::new(card_bytes)`
- `engine.candidates(state_json) -> Uint32Array`
- `engine.recommend(state, mode, budget) -> RecommendationJson`, running in a worker with a progress callback or yields
- `engine.search(query, pool, limit)`
- `engine.apply_reveal` / `engine.apply_challenge(state, …) -> Result<State, ErrCode>`
- `engine.simulate(config, pool, rounds, seed, shard) -> stream of summaries`
- `migrate_v4(json) -> State`

Error codes should map to the existing Chinese toast strings in JS.

## Risks
- Saved progress and history refer to cards by array index (logs[].guess). Any data regeneration or new binary format that reorders groups corrupts existing saves unless order is kept or entries are migrated by passcode.
- Supporting file:// double-click use and hosting on Cloudflare need different loaders (base64-embedded wasm, classic scripts, Blob worker vs ESM, streaming compile, module worker). Supporting both doubles build and test effort; a PWA service worker could replace the file:// case.
- WASM threads need COOP/COEP cross-origin isolation, which blocks hotlinked card images. Prefer multiple independent workers for the simulation.
- Card images (1-3 GB, about 18k files for both formats) cannot go into Workers static assets comfortably (20k files per version on the free plan). Hotlinking ygoprodeck conflicts with its guidance and the project's own policy. This needs a decision (thumbnails, R2, or images off by default).
- Hosting the wallpapers publicly (mix of fan art and official promo art, 23 MB) raises licensing exposure; the source file itself says attribution is not a licence.
- Parity tests against the JS engine depend on the same float summation order and the 1e-11 tie epsilon. Tied actions may resolve differently if the iteration order changes. Math.random-based simulations can only be compared statistically unless a seeded RNG is added to both implementations.
- The generator for cards-data.js (grouping, zh/en/ja names, wa) and the experiment harness are not in the repo. The Rust pipeline can only transform the existing cards-data.js, not rebuild it from upstream sources.
- Moving to an https origin gives users empty storage; without an export/import feature, in-progress event data (the event runs until 2026-10-02) would be lost.
- Simulation runtime is about 26 s per activity in JS today. If the Rust port keeps the same depth-3-then-fail pattern it will still be slow. Profiling must separate the wasted depth-3 work from real work.

## Open questions
- Must the new tool still work offline by double-clicking a local file (file://), or is hosted-only plus an optional PWA offline cache acceptable? This decides the WASM loading strategy.
- Where should card images come from in the hosted version: self-hosted thumbnails, an R2 bucket, hotlinking, or disabled by default with a user-supplied base URL?
- Should the wallpapers ship in the public deployment at all, given the licensing notes, or be reduced to a few official or self-made images, optionally converted to WebP?
- Should the simulation bring back the metrics the docs still describe (高价值题全通率, conditional puzzles reached, 后段匹配收益) or should the docs be updated to match the worker's current output?
- Should known bugs be fixed or reproduced for parity: partial activity counted on cancel, undefined hintsUsed in the forced pulse, stale recommendation after a state change mid-calculation, worker not terminated in onerror, test-mode history candidate count?
- Is WebMCP registration worth keeping, and under which API name (document.modelContext vs navigator.modelContext in current drafts)? Should record_challenge / simulation tools be added?
- Should the UI stay vanilla JS (possibly TypeScript) around a coarse WASM API, or should state ownership move into Rust (for example Leptos or Yew), which is a much larger change?
- Should the search show the matched alias rather than the group's representative name, and should pinyin or initials search be added?
- Should the data format stay a JS global (window.CARD_DATA) for tooling compatibility, or change to JSON plus a binary columnar core? If it changes, how are tools/refine-master-duel-pool.mjs and the image downloader updated or ported?
