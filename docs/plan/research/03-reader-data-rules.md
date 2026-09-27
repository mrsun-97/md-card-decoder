# Reader report: data_rules

## Summary
I audited dist/cards-data.js, the tools/*.mjs pipeline, the rule code (matchMask, the feedback keys and the known-field filter in app.js and solver-core.js) and the assets. I rebuilt the data from today's upstream sources (ygocdb cards.zip, YgoMaster CardList.json and YdkIds.txt). All 9312 records reproduce with 0 field mismatches and 0 wm mismatches, and names match in 8209 of 8210 groups. The script that originally built cards-data.js is not in the repo, and refine-master-duel-pool.mjs only rewrites wm, so the base dataset is frozen and new cards cannot be added today. The rule code matches both official pendulum examples. Three unconfirmed assumptions carry real weight: 19.7% of level-field matches are across level/rank/link, 2.2% of weighted pairs depend on how a pendulum guess matches a non-pendulum target, and the in-game list's auto-filter may restrict which cards can be guessed. Moving to a column-per-field binary table plus separately loaded name files cuts first-load data from 399 KB to about 113 KB brotli, and the 23.8 MB of wallpapers re-encode to 2.18 MB WebP. The wallpapers are mostly unlicensed fan art or KONAMI art, so they should not ship by default in the online build.

## Findings
- **Original ygocdb->cards-data.js builder is missing, but fully reconstructible**: No script in the repo reads 百鸽/ygocdb. refine-master-duel-pool.mjs only recomputes wm (plus stats and generatedAt) and never adds or changes groups, so new Master Duel monsters cannot enter the pool. I rebuilt it from https://ygocdb.com/api/v0/cards.zip. Keep entries with data.type&1 and id<1e8 (this drops 17 pre-release 100268xxx ids); 9329-17=9312 records. Rules: b=frame(type); a=attribute; r=race; n=level&0xff; atk as-is; def=-3 if LINK else def; nm=(1<<n)|(1<<printed), where printed=0 for the 7 cards whose text says '规则上…当作'. names=dedupe of [md_name, sc_name, cn_name, nwbbs_n, cnocg_n, en_name, jp_name] per card, joined over the group in ascending cid order; name=names[0]. Result: 0 field mismatches and names equal in 8209/8210 groups (千年绝镜吸收鸟 has since gained the upstream alias 千年吸收绝对鸟). Recomputing wm from the current YgoMaster files gives 0 differences. [scratchpad/a6.js and a7.js output: 'field mismatches vs reconstruction 0', 'names order A 8209 / 8210'; a5.js: 'reproduced wm diffs 0']
- **Group key must include the printed level (6-field signature is not unique)**: 8210 unique 7-tuples (b,a,r,n,nm,atk,def) but only 8209 unique 6-tuples. 究极玛雅・卓尔金 (nm=4097) and 赤龙 克察尔科亚特尔 (nm=4096) share (8,32,8192,12,0,0). They are different groups, yet they solve each other (mask 63 both ways). They behave differently only against the FNo.0 group (nm=3). Equivalence classes should therefore be computed per rule policy. [a1.js: 'unique sig6 8209 dups [8|32|8192|12|0|0]'; a3/example: Tzolkin<->Quetzalcoatl mask 111111]
- **Pendulum frame: symmetric b&b is consistent with both official examples, but a pendulum guess on a non-pendulum target is unconfirmed**: Zarc (b=132) guessed with 幻之召唤神 艾克佐迪亚 (b=4) gives mask 000011, and with 清透翼疾速龙 (b=136, which is Synchro/Pendulum, not Pendulum Effect as transcribed) gives mask 000101. Both are frame matches, as the official examples say. The official rule covers only a pendulum target. With b&b, a pendulum guess like 纠罪巧－骄狮 Aizaβ.LEON (130) also matches a plain Effect target like 库里波 (2). If the game instead lets a pendulum guess match only pendulum targets, 2.20% of weighted (target,guess) pairs change (1,413,520 group pairs). [a3.js: 'guess pendulum & target non-pendulum & share base frame (M1 match, M2 no): 1805152 2.200%']
- **Level/Rank/Link share one number field and cross-match (large unconfirmed assumption)**: nm&nm ignores the kind, so a Level 4 card (沉默魔术师 LV4) matches Rank 4 (编号39 希望皇霍普) and Link 4 (枪管上膛龙). Cross-kind matches are 19.66% of all level-field matches (2.93% of all weighted pairs): R<->V 727,536, L<->V 448,823 and L<->R 26,222 in each direction. The in-game field label '等级/阶级/连接' suggests a plain number comparison, but the rules do not say so. [a3.js: 'cross-kind level/rank/link matches 2405162 19.66% of level matches']
- **Multi-bit nm = 7 'rule level differs from printed' cards in 4 groups; guess-side and printed-0 matching are assumptions**: FNo.0 group ids 65305468, 43490025, 41522092 (nm=3, n=1); 闪光编号0 霍普・ZEXAL 52653092 (nm=3); 翼日编号0 未来龙皇霍普 26973555 (nm=3); Tzolkin + Bishbaalkin 1686814, 90884403 (nm=4097, n=12). Printed rank/level is 0, rule value is 1/12. ygocdb stores the rule level and ygoprodeck stores 0. The code (a) reveals n (the rule value) and never 0, (b) lets the guess card's printed 0 count, so FNo.0 guessed against Tzolkin matches through printed 0 vs printed 0, and (c) shows Rank 0 cards as '1'. Official wording supports only 'the correct card may match either value'. Only 20 weighted pairs reveal a different n, so the impact is tiny, but it is not modelled explicitly. [a1.js nm != 1<<n list; ygocdb search '规则上，这张卡的等级/阶级当作' returns exactly these 7 ids]
- **Frame reveals full combo (confirmed), level reveals rule n (assumed); '?'=-2 and link DEF=-3 compare equal to themselves**: The feedback key is (mask, target.b if frame matched, target.n if level matched). The official example confirms the frame reveal ('融合/灵摆' = 132). The known-field filter uses exact equality on b and n. -2 ('?') occurs in ATK for 71 groups and DEF for 43; -3 marks exactly the 388 link groups (def=-3 iff b==32, verified). '?'=='?' and -3==-3 must match at least on a self-guess, or those targets could never be solved. Treating two different '?' cards or two different links as matching is the natural reading but is still unconfirmed. [solver-core.js:132, app.js:318-331, app.js:1407-1410; a2.js 'def=-3 && b!=32 0']
- **In-game list auto-filter may restrict the action set**: The solver lets you challenge any not-yet-guessed MD group, including cards that contradict known fields. If the in-game list's auto-filter by known info cannot be turned off, many recommended challenges are not actually selectable. The code has no switch for this. [app.js:822-823 (actions = all wm>0 groups not guessed); solver-core solveExact/solveRestrictedHorizon actions]
- **Saved progress references group array indices**: state.logs[].guess stores the index into CARDS, and it is persisted in localStorage 'card-decoder-state-v4'. refine keeps the order, but any rebuild that reorders groups (and the current order cannot be reproduced; it is not sorted by wm or wa) would silently corrupt saved games. The new format should persist passcodes plus a data version. [app.js:5, app.js:780-782, a4.js 'sorted by (wm desc, wa desc) false']
- **Size: names are 53% of the 2.08 MB file; column-per-field u8 table is 65,680 B (22 KB br)**: Current file: 2,080,381 B raw, 555,338 B gzip, 399,136 B brotli. JSON bytes by part: names 1,099,712 (52.9%), name 243,157 (11.7%), ids 148,422 (7.1%), the 9 numeric fields with keys 571,669 (27.5%). Proposed: cards.bin with 6 u8 value-ids + wm + wa, one column per field, is 65,680 B raw, 26,240 gzip, 22,327 br (row-major would be 27,277 br). Dictionaries are 1,076 B. Representative passcodes are 32,840 B (29,658 br). Display names are 169,266 B (61,136 br). The alias search haystack is 767,726 B (230,099 br) and loads only when search is first used. [scratchpad/a4.js and name-split measurement output]
- **Wallpapers: 23.85 MB → 2.18 MB WebP (≤1200px q72) / 1.61 MB AVIF; downloaded even on mobile**: The 20 files are mostly 1200x1200 RGB PNGs of 1.4–3.1 MB. The side panels are at most 560 CSS px wide and are hidden below 980 px, but rotateWallpaper() preloads through new Image() whatever the viewport, every 45 s. card-back.gif is really a JPEG (468x676, 66 KB) and would get the wrong Content-Type. [imgtest/img.js output; styles.css:41,414; app.js:210-249]
- **Card images: UI uses only ids[0]; downloader fetches 9,125 files incl. 66 non-MD; one MD group has a non-MD representative**: setCardImage uses card.ids[0] only, so 8,023 images (MD groups) or 8,210 (all groups) are enough. The downloader takes every id in wm>0 groups (9,125, 66 of them not in MD). 优秀精灵's ids[0] is not an MD passcode. The zh CDN has images for all 8,210 representatives, averaging 136 KB (680x986): 1.09 GB for the MD groups. Measured WebP thumbnails: w300 about 22.9 KB (183 MB total), w440 about 39 KB (313 MB); AVIF w440 about 31.5 KB (252 MB). [app.js:175-190; tools/download-card-images.mjs:17-19; a5.js; imgtest/img.js]
- **Rule logic duplicated in 5+ places; search lacks NFKC**: matchMask exists in app.js:305, solver-core.js:33 and the generated worker source. Feedback keys are rebuilt separately at app.js:895, 1032, 1514 and 1563, and in solver-core.js:132 and 229. populateSpecialReveal (app.js:753) re-implements the frame/level match. Rust should keep one rules module. normalizeSearch does not NFKC-fold, and 2,953 aliases contain full-width characters: in 1,372 groups the NFKC-folded alias does not appear in the raw alias list. Example: 'zexal' finds 20 MD groups raw and 21 with NFKC. [grep counts; nfkc measurement]
- **Docs reference files that do not exist; pool source freshness**: 使用说明.txt mentions 打开求解器.html, 下载高清卡图.cmd and 下载中文卡图.cmd, none of which are in the repo. YgoMaster CardList.json/YdkIds.txt were last changed at commit 0e2078237d46c2450f3a18697b8a756df256b8e9 (2026-09-04), and the script fetches from master without pinning. ygocdb has no versioned URL but publishes cards.zip.md5 (4ccbfbdd77bca4d9762bb0efeae6ea3e at audit time). [ls of repo; GitHub API query; https://ygocdb.com/api/v0/cards.zip.md5]

## Spec
# Audit: card data model, data pipeline, rule implementation (md-card-decoder)

Throwaway scripts used for every number below are in
`/tmp/claude-1000/-home-parry-gitRepos-md-card-decoder/cb7e6cca-357d-4211-9aca-7cbb5c5749b0/scratchpad/` (`a1.js`…`a8.js`, `imgtest/img.js`; raw upstream downloads `cl.dat`, `ydk.dat`, `meta.dat`, `ygocdb/cards.json`). No repo file was modified.

---

## 1. Schema of `dist/cards-data.js`

```text
window.CARD_DATA = {
  generatedAt: "2026-09-25T09:58:43.282Z",   // overwritten by refine script (wall clock), not the snapshot date
  stats: { allMonsterRecords: 9312, masterDuelMonsterRecords: 9059, behaviorGroups: 8210, masterDuelGroups: 8023 },
  labels: { frame: {bit: zh}, attribute: {bit: zh}, race: {bit: zh} },
  cards: Group[8210],
  source: { masterDuelPool: "YgoMaster CardList.json + YdkIds.txt", note: "…活动仍可能额外排除少量卡片。" }
}
```

Every group has exactly these keys, and all numbers are JS numbers.

| key | meaning | domain (all 8210 groups) | derivation (reconstructed, verified) |
|---|---|---|---|
| `b` | frame bitmask | {1,2,4,8,16,32,64,129,130,132,136,144,192}: 13 values, all present in MD | from the ygopro `type`: LINK→32, XYZ→16, SYNCHRO→8, FUSION→4, RITUAL→64, NORMAL→1, else 2 (effect); plus 128 if PENDULUM. Max 192, so it fits in u8 |
| `a` | attribute bit (raw ygopro) | 1 地, 2 水, 4 炎, 8 风, 16 光, 32 暗, 64 神: 7 values | `data.attribute` |
| `r` | race bit (raw ygopro) | 26 values, 1…33554432 (幻想魔); MD uses 25 (创造神 only in wm=0) | `data.race` |
| `n` | rule level / rank / link rating | 1…13 (13 = rank only, 2 groups); link 1…6 | `data.level & 0xff` (pendulum scales sit in the high bytes, e.g. 0x1010005) |
| `nm` | level match mask | 15 values; 1<<n except 4 groups | `(1<<n) \| (1<<printed)`, where printed = 0 for the 7 cards with '规则上，这张卡的等级/阶级当作N' |
| `atk` | ATK | 83 values; -2 = '?' (71 groups); others 0…5000 (not all multiples of 50: 920, 1380, 1610, 2510) | `data.atk` (ygopro already uses -2 for '?') |
| `def` | DEF | 77 values; -2 = '?' (43 groups); -3 = link has no DEF (exactly the 388 b==32 groups) | LINK ? -3 : `data.def` (for links the raw def is the arrow bitmask) |
| `wm` | MD weight | 0…21; 187 groups are 0; sum 9059 | number of `ids` in the YgoMaster passcode set (refine script) |
| `wa` | all-pool weight | 1…22; sum 9312 | always equals `ids.length` (verified for 8210/8210) |
| `ids` | passcodes | 9312 unique, 2511…99991455, no alt-art passcodes | ygocdb `id` per card, ordered by ascending cid (735/735 multi-id groups) |
| `names` | search aliases | 40,259 strings (21,455 zh, 8,986 en, 9,818 ja); 21 strings appear in more than one group | per card `dedupe([md_name, sc_name, cn_name, nwbbs_n, cnocg_n, en_name, jp_name])`, joined over the group |
| `name` | display name | always `names[0]`; 7 are ASCII (official names such as 'The big SATURN'; 'Anotherverse Dragon' has an English md_name) | first alias |

The multi-bit `nm` groups (all 7 rule-level cards):

| group | ids | b | n (rule) | printed | nm | wm |
|---|---|---|---|---|---|---|
| 翼日编号0 未来皇霍普 | 65305468, 43490025, 41522092 | 16 | 1 | 0 | 3 | 3 |
| 闪光编号0 霍普・ZEXAL | 52653092 | 16 | 1 | 0 | 3 | 1 |
| 翼日编号0 未来龙皇霍普 | 26973555 | 16 | 1 | 0 | 3 | 1 |
| 究极玛雅・卓尔金 (+ 究极幻神 比希巴尔金) | 1686814, 90884403 | 8 | 12 | 0 | 4097 | 2 |

**Groups**: 8210 = unique (b,a,r,n,nm,atk,def). The 6-field signature without nm has 8209 unique values, because Tzolkin collides with 赤龙 克察尔科亚特尔.
**Ordering**: cannot be reproduced (not sorted by wm or wa; the MD groups are not a contiguous prefix; the first wm=0 group is at index 1343). Most likely it was sorted by an older weight.
**Record set**: ygocdb has 14,325 entries (44 without `data`) and 9,329 monsters; the 17 pre-release ids ≥ 100,000,000 are excluded, leaving 9,312. No tokens are included. `ot` values in the set: 1, 2, 3, 9, 11.
**Missing builder**: the ygocdb→cards-data build script is not in the repo. `refine-master-duel-pool.mjs` only changes `wm`, `stats.masterDuel*`, `generatedAt` and `source`. Rebuilding from today's `cards.zip` gives 0 field mismatches in 9312 records, names identical in 8209/8210 groups (upstream alias drift) and 0 `wm` differences.

## 2. Size breakdown and compact encoding

JSON bytes of each key, summed over groups (key + value + comma):

| part | bytes | share |
|---|---|---|
| names | 1,099,712 | 52.9% |
| name | 243,157 | 11.7% |
| ids | 148,422 | 7.1% |
| b, a, r, n, nm, atk, def, wm, wa (with keys) | 571,669 | 27.5% |
| braces | 16,420 | 0.8% |

Measured compression (Node zlib, gzip level 9, brotli quality 11 with lgwin 24):

| artifact | raw | gzip | brotli |
|---|---|---|---|
| **cards-data.js today** | 2,080,381 | 555,338 | 399,136 |
| names JSON (array of arrays) | 1,034,033 | 364,152 | 269,854 |
| ids JSON | 99,163 | 43,880 | 37,005 |
| numeric JSON | 235,060 | 39,278 | 34,675 |
| **cards.bin: 8 u8 columns, one column per field** (6 field value-ids + wm + wa, 8210 rows) | 65,680 | 26,240 | 22,327 |
| same data, row-major | 65,680 | 30,824 | 27,277 |
| value dictionaries (JSON) | 1,076 | 386 | 312 |
| representative passcode, u32 per group | 32,840 | 30,568 | 29,658 |
| all ids as u32 (count = wa) | 37,248 | 34,695 | 33,634 |
| display names (zh), newline-joined | 169,266 | 71,775 | 61,136 |
| aliases without the display name (\x01 inside a group, \n between groups) | 767,726 | 303,798 | 230,099 |
| NFKC-normalized, deduplicated search haystack | 849,334 | 327,829 | 252,034 |

**Proposal**
- `cards.v{N}.{hash}.bin` for the solver: header (magic `MDCD`, format version, data-version hash, n_groups, n_md, one dictionary length per field), then the dictionaries (raw i16 value per id plus reveal tables), then 8 u8 columns. All weights fit in u8 (max 22). Put the MD groups first as a contiguous prefix, sorted by (wm desc, wa desc, min cid asc), so the MD pool is `0..8023`. First-load cost is about 22 KB br. It can be embedded in the wasm with `include_bytes!` or fetched at runtime; runtime fetch allows data-only updates.
- `meta.{hash}.json` or `.bin`: representative passcode per group (the first MD passcode, not blindly `ids[0]`) plus the full id lists (about 30–34 KB br).
- `names-zh.{hash}.txt`: display names, about 61 KB br, loaded with the table.
- `aliases.{hash}.txt`: loaded on first focus of the search box (about 230 KB br). Search is a linear substring scan of one NFKC + lowercase + punctuation-stripped haystack with a u16 offset→group table. That is about 0.85 MB per keystroke, well under a millisecond with `memchr::memmem` in wasm or `indexOf` in JS, so no n-gram index is needed. Fold with NFKC: 2,953 aliases contain full-width characters.
- First-load data drops to about 113 KB br (22 + 0.3 + 30 + 61), compared with 399 KB br (555 KB gzip) today, and the page no longer parses a 2 MB JS literal on the main thread.
- Caveat: Cloudflare compresses at the edge by content type, and `application/octet-stream` may be sent uncompressed. At 65 KB that is acceptable; alternatively embed the table in the wasm. Check which types are compressed at deploy time.

## 3. Rule-correctness audit

Current code (identical in app.js:305, solver-core.js:33 and the generated worker source):
`frame: t.b & g.b; attr ==; race ==; level: t.nm & g.nm; atk ==; def ==`.
The feedback key is `(mask, target.b if frame matched, target.n if level matched)`.
Hints and initial reveals split candidates by the exact `b` and by `n`. The known-field filter checks `card.b == value` and `card.n == value`.

| # | Rule aspect | Code behaviour | Status | Impact (MD-weighted pairs, 82,065,481 total) |
|---|---|---|---|---|
| R1 | Pendulum target, guess with the same base frame | 艾克佐迪亚 Fusion (4) vs 霸王龙扎克 (132) → frame matches (mask 000011) | **Confirmed** by the official example | – |
| R2 | Pendulum target, pendulum guess with a different base | 清透翼疾速龙 (136; the data says Synchro/Pendulum, not 'Pendulum Effect') vs Zarc (132) → matches via bit 128 (mask 000101) | **Confirmed** | – |
| R3 | **Pendulum guess, non-pendulum target with the same base** | e.g. Aizaβ.LEON (130) vs 库里波 (2) → match; 德拉克尼亚的海龙骑兵 (129) vs 被封印的右足 (1) → match | **Assumption.** The rule text covers only a pendulum target | 2.20% of weighted pairs (1,413,520 group pairs) would flip |
| R4 | Frame reveal | shows the target's full combo (132 → 融合/灵摆) | **Confirmed** | – |
| R5 | **Level vs Rank vs Link** | numbers compared regardless of kind: 沉默魔术师 LV4 (L4) matches 编号39 希望皇霍普 (R4) and 枪管上膛龙 (Link-4) | **Assumption** (the field label '等级/阶级/连接' suggests it) | 19.66% of level matches, 2.93% of all pairs |
| R6 | Rule-level cards, target side | FNo.0 target matches guesses at level/rank 1 or printed 0; Tzolkin target matches 12 or printed 0 | **Confirmed in spirit** ('guessing either counts') | – |
| R7 | Rule-level cards, guess side | the guess's printed 0 also counts: FNo.0 guess vs Tzolkin target → level match (mask 111000) | **Assumption** | 20 weighted pairs reveal a different n |
| R8 | Level reveal value | reveals the rule `n` (FNo.0 → 1, Tzolkin → 12); printed 0 can never be entered in the UI | **Assumption** (the game might show 0) | 7 cards |
| R9 | '?' ATK/DEF (-2) | '?' equals '?' only | Required for self-match; matching across different cards is **assumed** | 71 ATK / 43 DEF groups |
| R10 | Link DEF (-3) | every link matches every link on DEF, never a non-link | Required for self-match; cross-link match is **assumed**. DEF adds no information beyond frame for links | 388 groups |
| R11 | Several cards match all 6 fields | solved when mask == 63, whatever the group (Tzolkin and 赤龙 solve each other) | Correct | – |
| R12 | Re-judging the same card | excluded (`guessed` set; UI refuses) | Correct | – |
| R13 | Hint | picks a uniformly random unknown field and reveals full b or rule n | Assumption stated by the tool | – |
| R14 | Initial reveal / next-puzzle prior | exactly one uniformly random field; the target is drawn in proportion to wm | **Assumption** | – |
| R15 | **Action set** | any MD group not yet guessed, including ones inconsistent with known fields | **Open**: 'the in-game list auto-filters by known info' may make those unselectable | potentially large |
| R16 | Scoring | fields revealed by hints or the initial reveal don't count; a later challenge match on a hinted field counts as a first-time match | **Assumption** (in the objective, not in the rules) | – |

**Make the rules configurable and testable (Rust).** Put a single `rules` module behind a `RulePolicy` struct:
- `frame: Intersect (current) | PendulumGuessNeedsPendulumTarget`
- `level: Numeric (current) | SameKind` (level vs rank vs link)
- `alt_level: BothSides (current) | TargetOnly`
- `level_reveal: Rule (current) | Printed`
- `unknown_stat: QEqQ (current) | SelfOnly`
- `link_def: NoneEqNone (current)`
- `actions: Any (current) | ConsistentWithKnown`

Precompute compatibility tables from the policy (below), so the policy costs nothing in the hot loop. Compute behaviour-equivalence classes per policy at load time: under `alt_level=TargetOnly`, Tzolkin and 赤龙 merge, giving 8209 classes.

**Tests:**
1. Golden fixtures (TOML) of (guess passcode, target passcode, expected mask, reveals). Include the two official examples: Zarc 13331639 against 83257450 and 90036274.
2. Property: self-guess gives 63 for every group under every policy.
3. Property: feedback partitions agree with the known-field filter.
4. Parity with the old JS matchMask under the default policy.
5. A 'policy consistency' diagnostic: replay a user's recorded logs under every policy combination and report which ones keep the candidate set non-empty. Real play then tells us the true rules (R3, R5, R7, R8).

## 4. Field cardinalities and per-field value-id dictionaries

| field | distinct (all / MD) | id type | match | reveal |
|---|---|---|---|---|
| frame | 13 / 13 | u8 | `frame_compat[g] >> t & 1` (u16 rows) | target frame id (4 bits) |
| attribute | 7 / 7 | u8 | equal | – |
| race | 26 / 25 | u8 | equal | – |
| level | 15 nm values; 33 (kind, n, nm) tuples | u8 | `level_compat[g] >> t & 1` (u64 rows if kind-aware ids) | `level_reveal[id]` = n (4 bits) |
| atk | 83 / 83 | u8 | equal | – |
| def | 77 / 77 | u8 | equal | – |

The largest field has 83 values, so every per-field compatibility row fits in a u128 if a uniform formula is wanted. Feedback code as a u16: `mask | (frame_tid+1)<<6 | n<<10`, which is 14 bits (the current JS uses `mask | b<<6 | (n+1)<<14`). For a bitset solver, precompute one bitset of targets per (field, value id): 221 values × 129 u64 words ≈ 228 KB.

**Frame compatibility** (row = guess id; the cells list the target ids that match):

| id | b | label | groups | MD wt | current (b&b) | alt: pendulum guess needs pendulum target |
|---|---|---|---|---|---|---|
| 0 | 1 | 通常 | 661 | 671 | 0,7 | 0,7 |
| 1 | 2 | 效果 | 5102 | 5796 | 1,8 | 1,8 |
| 2 | 4 | 融合 | 520 | 540 | 2,9 | 2,9 |
| 3 | 8 | 同步 | 500 | 514 | 3,10 | 3,10 |
| 4 | 16 | 超量 | 549 | 571 | 4,11 | 4,11 |
| 5 | 32 | 连接 | 388 | 450 | 5 | 5 |
| 6 | 64 | 仪式 | 138 | 138 | 6,12 | 6,12 |
| 7 | 129 | 通常/灵摆 | 38 | 38 | 0,7–12 | 7–12 |
| 8 | 130 | 效果/灵摆 | 281 | 304 | 1,7–12 | 7–12 |
| 9 | 132 | 融合/灵摆 | 13 | 13 | 2,7–12 | 7–12 |
| 10 | 136 | 同步/灵摆 | 8 | 8 | 3,7–12 | 7–12 |
| 11 | 144 | 超量/灵摆 | 6 | 10 | 4,7–12 | 7–12 |
| 12 | 192 | 仪式/灵摆 | 6 | 6 | 6,7–12 | 7–12 |

**Level compatibility** (current, kind-agnostic; ids by nm): id0 nm=2 (n1) matches {0,1}; id1 nm=3 (FNo.0, n1) matches {0,1,13}; ids 2–11 (nm 4…2048, n 2…11) each match only themselves; id12 nm=4096 (n12) matches {12,13}; id13 nm=4097 (Tzolkin, n12) matches {1,12,13}; id14 nm=8192 (n13, rank only) matches {14}.
For the kind-aware policy, level ids are the 33 distinct (kind, n, nm) tuples: level 1–12 plus 12', rank 1–13 plus 1', link 1–6. Reveals partition by n (13 values), not by level id.

Label dictionaries: frame, attribute and race labels come straight from `labels`. atk/def labels: -2 → '?', -3 → '无/—'. Keep the raw i16 values in the dictionary for display.

## 5. Data pipeline plan (Linux)

**Sources**
- ygocdb full dump `https://ygocdb.com/api/v0/cards.zip`: 2.45 MB zip, 13.75 MB JSON, with an md5 at `/api/v0/cards.zip.md5`. Provides fields and zh/en/ja names.
- YgoMaster `CardList.json` (13,941 internal ids) and `YdkIds.txt` (17,991 passcode→internal lines → 17,637 mapped passcodes). Pin commit `0e2078237d46c2450f3a18697b8a756df256b8e9` (2026-09-04) instead of `master`.
- ygoprodeck API `db.ygoprodeck.com/api/v7/cardinfo.php`, used only to cross-check (printed level 0, type, atk/def).
- Image CDNs: momobako `cdn.233.momobako.com/ygoimg/sc/{id}.webp` plus `/metadata` (15,020 entries); ygoprodeck `images.ygoprodeck.com/images/cards{,_small}/{id}.jpg`.

**Layout (proposed)**
- `crates/md-data`: the schema, bin encode/decode and rule tables, shared with the solver as the single source of truth.
- `xtask`: `cargo xtask data fetch|build|verify|diff`.
- `data/sources.lock.json` (committed): URL, sha256, upstream md5 or commit, fetched-at.
- `data/raw/` (gitignored cache).
- `web/public/data/*.{hash}.*` (generated) plus `data/summary.json` (committed, so data changes show up in PRs).
- Image and wallpaper processing stays in Node (`tools/images.mjs` with sharp; the prebuilt sharp 0.34 installed and ran fine here). Node is needed for wrangler anyway.
- Replace `更新大师决斗卡池.cmd` with `npm run data:update` → `cargo xtask data fetch && cargo xtask data build && cargo xtask data verify`.

**Build algorithm** (reproduces today's data):
1. Keep ygocdb entries with `data` and `type&1` and `id<1e8`.
2. Frame, attribute, race, n, atk and def as in section 1.
3. printed = 0 when the Chinese desc matches `/规则上，这张卡的(等级|阶级)当作\d+(星|阶)使用/`, cross-checked against ygoprodeck `level==0`; must be exactly these 7 ids.
4. Group by (b,a,r,n,printed,atk,def); ids in cid order.
5. wm = |ids ∩ MD passcodes|, wa = |ids|.
6. Names in the order md, sc, cn, nwbbs, cnocg, en, jp, deduplicated. Display name: md_name if it contains CJK, else sc/cn, else md/en.
7. Deterministic sort; MD prefix.
8. No wall-clock timestamps inside artifacts: generatedAt = the source lock date, so rebuilds are byte-identical.

**Validation (CI-failing checks, current expected values)**
- Counts: 14,325 entries / 9,329 monsters / 17 pre-release excluded / 9,312 records / 8,210 groups / 8,023 MD groups / Σwm 9,059 / Σwa 9,312. Print differences against the previous lock as a diff report, not a hard failure.
- Unique 7-tuple group key. Report the 6-field collisions (currently 1).
- Invariants:
  - b ∈ the 13-value set; a is a single bit of 7; r is a single bit of 26.
  - 1 ≤ n ≤ 13 (13 only for rank); link n ≤ 6.
  - popcount(nm) ≤ 2, and a second bit only for rule-level cards.
  - def == -3 ⇔ b == 32; atk/def ∈ {-2} ∪ [0, 5000].
  - wm ≤ wa ≤ 255; every value-id count ≤ 255.
- Parity with legacy `dist/cards-data.js`: 0 field mismatches and 0 wm differences; list name differences (currently 1).
- Passcode uniqueness; the representative is an MD passcode for every MD group (fixes 优秀精灵).
- Rule golden fixtures and the property tests from section 3.
- Freshness warning when the YgoMaster commit is older than the event start (2026-09-23 23:00) or when newer MD monster passcodes are missing from ygocdb.
- Persist user state by passcode plus data hash, never by array index.

## 6. Card images and wallpapers for online deployment

**Display sizes** (styles.css): recommendation ≤292 CSS px, card-art 216, selected 180, history 44, viewer ≤620. Only `ids[0]` is ever shown, so one image per group is enough: 8,023 (MD) or 8,210 (all).

**Card image measurements**
- zh CDN coverage is 8,210/8,210 representatives. Average 136 KB at 680×986: 1.09 GB for MD representatives, 1.24 GB for all 9,059 MD cards.
- ygoprodeck `cards` (813×1185) runs 86–184 KB; `cards_small` (268×391) runs 25–41 KB.

Thumbnails measured on 24 zh samples (per image → total for 8,023):

| width | WebP q75 | AVIF q50 |
|---|---|---|
| 180 | 10.4 KB → 83 MB | 9.7 KB → 78 MB |
| 300 | 22.9 KB → 183 MB | 19.8 KB → 159 MB |
| 440 | 39.0 KB → 313 MB | 31.5 KB → 252 MB |
| 600 | 57.5 KB → 461 MB | 44.3 KB → 355 MB |

**Wallpapers** (20 files, 23,847,957 B). Columns: size / dimensions / WebP ≤1200 q72 / AVIF ≤1200 q50 / WebP ≤800.

| file | size | dims | WebP ≤1200 | AVIF ≤1200 | WebP ≤800 |
|---|---|---|---|---|---|
| 01 banner jpg | 248,069 | 1600×464 | 95,062 | 106,585 | 50,300 |
| 02 DMG official png | 1,854,457 | 1200² | 140,880 | 94,597 | 81,306 |
| 03 red-eyes-4k jpg | 556,572 | 1280² | 238,544 | 195,944 | 132,398 |
| 04 keyart jpg | 304,595 | 1472×1104 | 135,040 | 97,877 | 81,228 |
| 05 guardian jpg | 101,905 | 1400×700 | 53,920 | 43,264 | 30,054 |
| 06 DMG ocg jpg | 270,800 | 1280×720 | 116,026 | 90,855 | 62,838 |
| 07 izunanie jpg | 641,930 | 1212² | 67,066 | 42,528 | 41,238 |
| 08 red-eyes-fire png | 1,862,251 | 1200² | 114,644 | 70,601 | 69,298 |
| 09 cosmos png | 884,057 | 800² | 50,024 | 34,989 | 49,308 |
| 10 gods jpg | 191,522 | 1200×675 | 114,048 | 88,966 | 69,802 |
| 11 stardust jpg | 375,901 | 1920×1080 | 61,472 | 46,643 | 29,664 |
| 12 black-rose jpg | 1,689,885 | 1080² | 108,230 | 73,748 | 76,402 |
| 13 bloom png | 1,980,954 | 1200² | 112,302 | 72,274 | 71,982 |
| 14 fire jpg | 2,495,523 | 2000×1567 | 93,136 | 70,269 | 47,288 |
| 15 lightning png | 1,927,240 | 1200² | 113,032 | 75,001 | 68,656 |
| 16 gold png | 1,878,032 | 1200² | 108,040 | 70,531 | 65,330 |
| 17 akiza png | 3,064,261 | 1920×1080 | 67,004 | 49,665 | 37,750 |
| 18 stardust-cosmos png | 1,452,110 | 1920×1080 | 195,252 | 152,728 | 90,536 |
| 19 red-eyes png | 1,857,582 | 1200² | 104,394 | 66,125 | 65,230 |
| 20 DMG jpg | 210,311 | 1280×721 | 89,522 | 66,909 | 50,924 |
| **total** | **23.85 MB** | | **2.18 MB** | **1.61 MB** | **1.27 MB** |

`card-back.gif` is really a JPEG (468×676, 65,966 B); at WebP w440 it is 38,740 B. Rename it so the Content-Type is right.

**Options**
1. **Workers static assets** (same as ygo-small-world: `wrangler.jsonc` with an `assets.directory`, no `main`, `_headers`). The page, wasm, data and WebP wallpapers deploy together; roughly 8,023 thumbnails plus about 40 files stays under the per-version file limit (20k Free / 100k Paid; 25 MiB per file; check the current docs). Deploys upload only changed hashes. Don't commit images to git; generate them in CI or locally before `wrangler deploy`. Set `Cache-Control: public, max-age=31536000, immutable` on hashed `/data/*` and `/cards/*`.
2. **R2 bucket + custom domain**: keeps images out of app deploys, with free egress (a 10 GB free tier covers a 250–450 MB set plus full-size copies if wanted). Production needs a Cloudflare-managed domain (r2.dev is rate-limited) and a bulk uploader (rclone over the S3 API).
3. **Hotlink with lazy loading**: no hosting, but ygoprodeck explicitly asks not to hotlink (also noted in the ygo-small-world README), and momobako's policy is unknown. Use it only as a fallback or for the zoom viewer.
4. **Recommended hybrid**:
   - Phase 1: static assets for app, data and WebP wallpapers, plus w300–w440 WebP thumbnails, one per group, named by representative passcode.
   - Image fallback chain: local thumbnail → momobako zh → ygoprodeck → card back. Use `loading=lazy decoding=async`.
   - The zoom viewer loads full size on demand.
   - Move thumbnails to R2 later if deploy size becomes a problem.
   - Gate wallpaper loading with `matchMedia('(min-width: 981px)')` and `prefers-reduced-data`; today even phones that hide the panels preload a PNG of up to 3 MB every 45 s.

**Licensing** (from 壁纸来源与授权.txt): the project states it is non-commercial and unaffiliated, and says outright '署名与本声明不构成授权'. No license was obtained, and files are removed on request.
- 5 entries are official or KONAMI-labelled art: 02, 04, 05, 12 (credited to 高桥和希), 15.
- 5 come from aggregator pages with no clear author: 10 bilibili, 11 getwallpapers, 14 wallpapersafari, 17 alphacoders, 18 tophinhanhdep.
- The rest are fan art by named artists: 01 WOLFBLADE111, 03 Doriado229, 06/20 nhociory, 07 Izunanie, 08/13/16/19 TheHungTD, 09 flamemirenchers, 14 Van Ken003.
- The card back is KONAMI's design, taken from a shop page (stop2shop.com).

A public, crawlable URL is redistribution and a higher risk than a downloaded zip. Default the online build to no third-party wallpapers (CSS or procedural background, keeping the attribution page), and add wallpapers back only with the artists' permission. Card images are KONAMI copyright; fan tools commonly self-host thumbnails, but that decision belongs to the user.

## Risks
- The base dataset is frozen: refine-master-duel-pool.mjs cannot add new monsters, and the original builder is missing. Monsters released after the 2026-09-24 snapshot are silently absent until the new pipeline exists.
- Saved progress stores CARDS array indices (localStorage 'card-decoder-state-v4'). Any rebuild that reorders groups corrupts saved games unless state switches to passcodes plus a data-version hash.
- Level/Rank/Link cross-matching (19.7% of level matches) and pendulum guess vs non-pendulum target (2.2% of pairs) are unverified. If the game differs, candidate filtering can drop the true card to zero candidates and recommendations are biased.
- If the in-game list auto-filter restricts which cards can be guessed, the solver recommends challenges the player cannot actually select.
- Upstream drift: ygocdb names already changed in 1 group, YgoMaster is fetched from master unpinned, and ygocdb has no versioned dump URL. Builds are not reproducible unless sources are cached and hashed.
- Rule logic is duplicated in at least 5 JS places. During migration the Rust rules must be checked against the JS for parity, or old and new outputs will disagree.
- Public hosting of unlicensed fan-art wallpapers and a full KONAMI card-image set is a takedown or legal exposure that the offline tool did not have.
- Cloudflare may not edge-compress application/octet-stream; the .bin data may be sent uncompressed unless embedded in the wasm or given a compressible content type.
- Workers static-asset limits (file count per version, per-file size) and R2 free-tier numbers are from memory and must be checked against current Cloudflare docs before choosing where to host.
- The YgoMaster CardList can include not-yet-released or event-excluded cards (the source note itself warns that the event may exclude some), so the wm prior carries some error.

## Open questions
- R3: Does a pendulum guess (e.g. Effect/Pendulum) count as a frame match against a non-pendulum Effect target? The official examples only cover a pendulum target.
- R5: Does a Level 4 guess match a Rank 4 or Link 4 target in the '等级/阶级/连接' field, or must the kind also agree?
- R7/R8: For the 7 rule-level cards (Rank-0 FNo.0/S0 series, printed-Level-0 Tzolkin/Bishbaalkin), does the game reveal the rule value (1/12) or the printed 0? Does the guess card's printed value count?
- R15: Can the in-game card list's auto-filter by known info be turned off, i.e. can the player challenge a card that contradicts already revealed fields?
- R9/R10: Do two different '?'-ATK cards match each other, and do two different Link monsters match on DEF?
- R14: Does every puzzle start with exactly one uniformly random revealed field, and is the target drawn uniformly over MD monster cards?
- R16: Do fields revealed by hints or the initial reveal count toward the match milestones, and does a later challenge match on an already hinted field count as a first-time match?
- Should the online build ship any wallpapers, given the author states no license was obtained? Should card thumbnails be self-hosted (Workers static assets vs R2) or loaded from CDNs on demand?
- Thumbnail width budget: w300 (~183 MB) or w440 (~313 MB WebP / ~252 MB AVIF) for 8,023 images?
- Should the data pipeline be a Rust xtask (shares schema code with the solver) or stay in Node (already needed for wrangler and sharp)? The recommendation is Rust for data and Node for images.
