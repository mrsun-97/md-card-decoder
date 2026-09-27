// Page controller: state changes, rendering and the engine calls behind them.
import { CardData } from './data.js';
import { Engines, fromBase64 } from './engine.js';
import { FIELDS, escapeHtml, formatValue, frameGradient, pct, valueSortKey } from './labels.js';
import * as S from './state.js';
import { renderTrail } from './ui/trail.js';
import { thumbHtml, openViewer } from './ui/images.js';
import { registerWebMcp } from './webmcp.js';

const $ = (sel) => document.querySelector(sel);
const $$ = (sel) => [...document.querySelectorAll(sel)];

let engines;
let data;
let state; // the real activity
let session = null; // { mode: 'test'|'game', state, targets: [gid], revealed: bool }
const undo = new S.History();
let belief = null; // belief of the current puzzle
let rec = null; // latest recommendation
let recToken = 0;
let selected = null; // selected guess gid
let candidateLimit = 100;
let trailTab = null;
let trailCurrent = null;
let toastTimer = null;

const active = () => (session ? session.state : state);
const puzzle = () => S.currentPuzzle(active());
const events = (p = puzzle()) => S.engineEvents(p, (pc) => data.gidOf(pc));

// ---------------------------------------------------------------------------------------------
// Boot

export async function boot() {
  try {
    const cards = fromBase64(window.CARDS_BIN_B64);
    const tables = window.TABLES_BIN_B64 ? fromBase64(window.TABLES_BIN_B64) : new Uint8Array();
    engines = await Engines.create(cards, tables);
    data = new CardData(engines.columns(), window.CARD_NAMES || []);
    state = S.load(engines.main.dataHash) || migrateV4() || S.freshState();
    if (state.grants.startDate && state.grants.auto !== false) {
      state.grants.received = Math.max(state.grants.received, S.grantsReceivedByDate(state.grants.startDate, state.config.days));
    }
    await applyPolicy();
    bind();
    render();
    registerWebMcp(api);
    loadAliasesLater();
  } catch (err) {
    const box = $('#bootError');
    box.hidden = false;
    box.textContent = `载入失败：${err.message || err}`;
    throw err;
  }
}

function migrateV4() {
  try {
    const raw = localStorage.getItem(S.LEGACY_KEY);
    if (!raw || !window.LEGACY_PASSCODES) return null;
    const s = S.fromV4(JSON.parse(raw), (i) => window.LEGACY_PASSCODES[i]);
    setTimeout(() => toast('已从旧版存档导入当前题进度。'), 500);
    return s;
  } catch {
    return null;
  }
}

function loadAliasesLater() {
  const load = () => {
    if (data.aliases || !window.ENGINE_FILES?.aliases) return;
    const s = document.createElement('script');
    s.src = window.ENGINE_FILES.aliases;
    s.onload = () => { if (window.CARD_ALIASES) data.setAliases(window.CARD_ALIASES); };
    document.head.appendChild(s);
  };
  $('#cardSearch').addEventListener('focus', load, { once: true });
  $('#candidateSearch').addEventListener('focus', load, { once: true });
}

function applyPolicy() {
  const s = state.settings;
  return engines.setPolicy({ pool: s.pool, unknownStatMatches: s.unknownStatMatches, linkDefMatches: s.linkDefMatches, consistentOnly: s.consistentOnly });
}

// ---------------------------------------------------------------------------------------------
// State changes

function commit(mutate, { keepSelection = false } = {}) {
  undo.push(active());
  mutate(active());
  if (!session) S.save(state);
  if (!keepSelection) selected = null;
  render();
}

function addReveal(f, v, src) {
  const p = puzzle();
  // Real mode records what the game already showed, so a hint is accepted even at 0 (clamped).
  const next = [...events(p), { reveal: { field: f, value: v } }];
  const b = engines.belief({ events: next });
  if (!b.ok) return toast('这条线索与已有记录矛盾，没有加入。'), false;
  commit((st) => {
    S.currentPuzzle(st).events.push({ t: 'reveal', f, v, src });
    if (src === 'hint') st.inventory.hints = Math.max(0, st.inventory.hints - 1);
  });
  return true;
}

function recordChallenge(g, fb) {
  const p = puzzle();
  const followed = rec?.action?.type === 'challenge' && rec.action.gid === g;
  const next = [...events(p), { challenge: { guess: g, mask: fb.mask, frame: fb.frame ?? null, level: fb.level ?? null } }];
  const b = engines.belief({ events: next });
  commit((st) => {
    const pz = S.currentPuzzle(st);
    pz.events.push({ t: 'challenge', pc: data.passcode(g), mask: fb.mask, frame: fb.frame ?? null, level: fb.level ?? null, rec: followed, n: b.ok ? b.count : 0 });
    st.inventory.challenges = Math.max(0, st.inventory.challenges - 1);
    if (fb.mask === 63) pz.solved = true;
  });
  if (!b.ok) toast('判定与已有记录矛盾，请检查录入（可撤销）。');
}

function nextPuzzle() {
  const s = active();
  if (s.current + 1 >= s.config.puzzles) return toast('活动已全部完成。');
  commit((st) => {
    st.puzzles.push({ events: [], solved: false });
    st.current += 1;
    // Test/mini-game: draw the new target and its initial reveal before rendering.
    if (session) startSessionPuzzle();
  });
}

// ---------------------------------------------------------------------------------------------
// Test mode / mini-game

function startSession(mode) {
  session = { mode, state: S.freshState(state.config, state.presetId, state.settings), targets: [], revealed: false };
  session.state.grants.received = state.config.days; // sessions use the whole event budget
  const total = S.totalGrants(state.config);
  session.state.inventory = { hints: total.hints, challenges: total.challenges };
  startSessionPuzzle();
  render();
}

function startSessionPuzzle() {
  const s = session.state;
  const pool = state.settings.pool;
  let total = 0;
  for (let g = 0; g < data.count; g++) total += data.weight(g, pool);
  let x = Math.random() * total;
  let target = 0;
  for (let g = 0; g < data.count; g++) {
    x -= data.weight(g, pool);
    if (x < 0) { target = g; break; }
  }
  session.targets[s.current] = target;
  session.revealed = false;
  const f = Math.floor(Math.random() * 6);
  S.currentPuzzle(s).events = [{ t: 'reveal', f, v: data.value(target, f), src: 'initial' }];
  S.currentPuzzle(s).solved = false;
}

function sessionHint() {
  const target = session.targets[session.state.current];
  const unknown = FIELDS.map((_, i) => i).filter((i) => belief?.known?.[i] == null);
  if (!unknown.length || session.state.inventory.hints <= 0) return;
  const f = unknown[Math.floor(Math.random() * unknown.length)];
  addReveal(f, data.value(target, f), 'hint');
}

function exitSession() {
  session = null;
  selected = null;
  render();
}

// ---------------------------------------------------------------------------------------------
// Rendering

function render() {
  const s = active();
  document.documentElement.dataset.theme = state.settings.theme;
  belief = engines.belief({ events: events() });
  const eff = S.effectiveInventory(s);
  $('#puzzleNumber').textContent = s.current + 1;
  $('#puzzleTotal').textContent = `/ ${s.config.puzzles}`;
  $('#hintStock').value = s.inventory.hints;
  $('#challengeStock').value = s.inventory.challenges;
  $('#grantDays').value = s.grants.received;
  $('#grantDays').max = s.config.days;
  $('#grantTotal').textContent = `/ ${s.config.days} 天`;
  $('#plannedStock').textContent = `提示 ${eff.hints} · 挑战 ${eff.challenges}`;
  $('#themeSelect').value = state.settings.theme;
  $('#puzzleTitle').textContent = `第 ${s.current + 1} 题 · 六项线索${S.isPremium(s) ? '（高价值题）' : ''}`;
  $('#undoBtn').disabled = undo.size === 0;
  renderContradiction();
  renderSession();
  renderFields();
  renderRewards();
  renderRevealControls();
  renderSelected();
  renderCandidates();
  renderPoints();
  renderTrailPanel();
  const solved = puzzle().solved;
  $('#solvedBanner').hidden = !solved;
  $('#nextPuzzleBtn').textContent = s.current + 1 >= s.config.puzzles ? '活动完成' : '进入下一题';
  $('#nextPuzzleBtn').disabled = s.current + 1 >= s.config.puzzles;
  const pool = state.settings.pool;
  $('#poolCaveat').textContent = pool === 'md'
    ? '卡池：大师决斗游戏内卡片（YgoMaster 卡表 + 百鸽数据）。活动可能另外排除少量卡；若某候选确定不会出现，请忽略它。'
    : '卡池：完整官方怪兽（仅用于候选归零时排查漏卡）。';
  $('#dataStats').textContent = `卡库 ${data.count.toLocaleString('zh-CN')} 个判定组（大师决斗 ${engines.main.mdGroups.toLocaleString('zh-CN')}）· 数据 ${engines.main.dataHash}`;
  scheduleRecommend();
}

function sourceOf(fieldIndex) {
  for (const e of puzzle().events) {
    if (e.t === 'reveal' && e.f === fieldIndex) return e.src === 'initial' ? '初始揭示' : '提示揭示';
    if (e.t === 'challenge' && e.mask & (1 << fieldIndex)) return '挑战相符';
  }
  return null;
}

function renderContradiction() {
  const box = $('#contradictionBanner');
  if (belief.ok) { box.hidden = true; return; }
  const evs = puzzle().events;
  const describe = (i) => {
    const e = evs[i];
    if (!e) return `第 ${i + 1} 条`;
    return e.t === 'reveal' ? `第 ${i + 1} 条：${FIELDS[e.f].short} = ${formatValue(e.f, e.v)}` : `第 ${i + 1} 条：挑战 ${escapeHtml(data.name(data.gidOf(e.pc)))} 的判定`;
  };
  const sug = belief.contradiction.suggestions.map(([i]) => `<button class="quiet-btn" type="button" data-drop-event="${i}">删除${describe(i)}</button>`).join('');
  box.innerHTML = `<strong>记录互相矛盾：没有任何卡片符合全部线索。</strong><p>最可能录错的是：</p><div class="contradiction-actions">${sug || '（无法定位，请撤销最近的录入）'}</div>`;
  box.hidden = false;
}

function renderSession() {
  const box = $('#sessionBanner');
  $('#testModeBtn').disabled = Boolean(session);
  $('#gameModeBtn').disabled = Boolean(session);
  $('#randomHintBtn').hidden = !session;
  if (!session) { box.hidden = true; return; }
  box.hidden = false;
  const target = session.targets[session.state.current];
  const visible = session.mode === 'test' || session.revealed || puzzle().solved;
  $('#sessionModeLabel').textContent = session.mode === 'test' ? '测试模式 · 目标卡公开' : visible ? '小游戏模式 · 答案已揭晓' : '小游戏模式 · 目标卡隐藏';
  $('#sessionTargetName').textContent = visible ? data.name(target) : '？？？';
  $('#sessionTargetStats').textContent = visible
    ? FIELDS.map((_, f) => formatValue(f, data.value(target, f))).join(' · ')
    : `根据反馈猜出目标卡；当前剩余 ${belief.ok ? belief.count : 0} 个判定组。`;
  $('#sessionTargetThumb').innerHTML = visible ? thumbHtml(data, target) : thumbHtml(data, null);
  $('#revealTargetBtn').hidden = session.mode !== 'game' || visible;
}

function renderFields() {
  const known = belief.ok ? belief.known : [null, null, null, null, null, null];
  const matched = belief.ok ? belief.matched : 0;
  $('#fieldGrid').innerHTML = FIELDS.map((f, i) => {
    const v = known[i];
    const isMatched = Boolean(matched & f.bit);
    const glyph = i === 0 && v != null
      ? `<div class="field-glyph frame-glyph" style="--frame-fill:${frameGradient(v)}"><span></span><i></i></div>`
      : `<div class="field-glyph glyph-text">${escapeHtml(f.short.slice(0, 1))}</div>`;
    return `<article class="field-card${v != null ? ' is-known' : ''}${isMatched ? ' is-matched' : ''}">${glyph}
      <div class="field-content"><span>${f.label}</span><strong>${v != null ? escapeHtml(formatValue(i, v)) : '未知'}</strong><small>${v != null ? sourceOf(i) || '已知' : '等待线索'}</small></div>
      <div class="field-state ${isMatched ? 'is-match' : v != null ? 'is-revealed' : ''}">${isMatched ? '✓ 相符' : v != null ? '已知' : '—'}</div></article>`;
  }).join('');
}

function matchedCount() {
  return belief.ok ? popcount(belief.matched) : 0;
}

const popcount = (m) => { let c = 0; for (let v = m & 63; v; v &= v - 1) c++; return c; };

function renderRewards() {
  const s = active();
  const matched = matchedCount();
  const solved = puzzle().solved;
  const premium = S.isPremium(s);
  const nodes = premium
    ? [...s.config.milestones.map((m) => ({ t: m.matches, pts: m.points, solve: false })), { t: 6, pts: s.config.solvePoints, solve: true }]
    : Array.from({ length: 6 }, (_, i) => ({ t: i + 1, pts: s.config.regularMatchPoints, solve: false }));
  const progress = solved ? 100 : Math.min(94, (matched / 6) * 100);
  const track = $('#rewardTrack');
  track.style.gridTemplateColumns = `repeat(${nodes.length}, 1fr)`;
  track.innerHTML = `<div class="track-line"><span style="width:${progress}%"></span></div>` + nodes.map((n) => {
    const earned = n.solve ? solved : matched >= n.t;
    return `<div class="reward-node${n.solve ? ' final' : ''}${earned ? ' is-earned' : ''}"><b>${n.solve ? '全中' : n.t}</b><span>+${n.pts}</span></div>`;
  }).join('');
  $('#matchedSummary').textContent = `挑战累计相符 ${matched} / 6 · ${premium ? '高价值题' : '后段题'}${solved ? ' · 本题已通过' : ''}`;
  $('#puzzleScore').textContent = S.puzzlePoints(s, s.current, matched, solved);
}

function renderPoints() {
  const s = active();
  let total = 0;
  s.puzzles.forEach((p, i) => {
    const b = i === s.current ? belief : engines.belief({ events: events(p) });
    total += S.puzzlePoints(s, i, b.ok ? popcount(b.matched) : 0, p.solved);
  });
  $('#pointsTotal').textContent = total;
}

function renderRevealControls() {
  const s = active();
  const known = belief.ok ? belief.known : [];
  const unknown = FIELDS.map((_, i) => i).filter((i) => known[i] == null);
  const sel = $('#revealField');
  const prev = sel.value;
  sel.innerHTML = unknown.map((i) => `<option value="${i}">${FIELDS[i].label}</option>`).join('');
  if (prev !== '' && unknown.includes(Number(prev))) sel.value = prev;
  const hasInitial = puzzle().events.some((e) => e.t === 'reveal' && e.src === 'initial');
  const srcSel = $('#revealSource');
  srcSel.options[0].disabled = hasInitial;
  if (hasInitial && srcSel.value === 'initial') srcSel.value = 'hint';
  if (!hasInitial) srcSel.value = 'initial';
  const off = !belief.ok || puzzle().solved || unknown.length === 0 || Boolean(session);
  $('#addRevealBtn').disabled = off;
  $('#randomHintBtn').disabled = !session || puzzle().solved || unknown.length === 0 || s.inventory.hints <= 0;
  populateRevealValues();
}

function populateRevealValues() {
  const f = Number($('#revealField').value);
  const out = $('#revealValue');
  if (!belief.ok || Number.isNaN(f) || $('#revealField').value === '') { out.innerHTML = ''; return; }
  const opts = engines.revealOptions({ events: events() }, f);
  opts.sort((a, b) => {
    const ka = valueSortKey(f, a[0]); const kb = valueSortKey(f, b[0]);
    return typeof ka === 'number' ? ka - kb : String(ka).localeCompare(String(kb), 'zh-CN');
  });
  out.innerHTML = opts.map(([raw, mass]) => `<option value="${raw}">${escapeHtml(formatValue(f, raw))} · ${mass.toLocaleString('zh-CN')}张</option>`).join('');
}

function isConsistent(g) {
  if (!belief.ok) return true;
  return belief.known.every((v, f) => v == null || data.value(g, f) === v);
}

function renderSelected() {
  const box = $('#selectedCard');
  if (selected == null) { box.hidden = true; return; }
  box.hidden = false;
  $('#selectedThumb').innerHTML = thumbHtml(data, selected);
  $('#selectedCardName').textContent = data.name(selected);
  $('#selectedCardFacts').innerHTML = FIELDS.map((f, i) => `<span><em>${f.short}</em>${escapeHtml(formatValue(i, data.value(selected, i)))}</span>`).join('');
  const guessed = puzzle().events.some((e) => e.t === 'challenge' && data.gidOf(e.pc) === selected);
  const notes = [];
  if (guessed) notes.push('本题已挑战过这张卡：重复判定不消耗次数，也不会得到新信息。');
  if (!isConsistent(selected)) notes.push('这张卡与已知条件不符：游戏内需先重置卡片列表的筛选才能选到。');
  $('#selectedCardNote').textContent = notes.join(' ');
  $('#challengeBtn').disabled = puzzle().solved || !belief.ok || guessed;
  $('#challengeBtn').textContent = session ? (session.mode === 'game' ? '提交挑战' : '自动判定这次挑战') : '使用这张卡挑战';
}

function renderCandidates() {
  const box = $('#candidateTable');
  if (!belief.ok) { box.innerHTML = '<div class="empty-inline">记录矛盾，暂无候选。</div>'; $('#candidateCount').textContent = '0'; return; }
  const pool = state.settings.pool;
  const mass = belief.mass;
  $('#candidateCount').textContent = mass.toLocaleString('zh-CN');
  $('#candidateMass').textContent = `${belief.count.toLocaleString('zh-CN')} 个判定组`;
  const q = $('#candidateSearch').value.trim();
  const ff = $('#candidateFilterField').value;
  const fv = $('#candidateFilterValue').value;
  let list = belief.candidates;
  if (q) {
    const hits = new Set(data.search(q, pool, 100000));
    list = list.filter((g) => hits.has(g));
  }
  if (ff !== '' && fv !== '') list = list.filter((g) => String(data.value(g, Number(ff))) === fv);
  list = [...list].sort((a, b) => data.weight(b, pool) - data.weight(a, pool) || data.name(a).localeCompare(data.name(b), 'zh-CN'));
  const shown = list.slice(0, candidateLimit);
  box.innerHTML = `<div class="cand-row cand-head"><span>卡名</span>${FIELDS.map((f) => `<span>${f.short}</span>`).join('')}<span>概率</span></div>`
    + shown.map((g) => `<button type="button" class="cand-row" data-select="${g}"><span class="cand-name">${escapeHtml(data.name(g))}</span>${FIELDS.map((_, f) => `<span>${escapeHtml(formatValue(f, data.value(g, f)))}</span>`).join('')}<span>${pct(data.weight(g, pool) / mass)}</span></button>`).join('')
    + (list.length > shown.length ? `<button type="button" class="quiet-btn more-btn" id="moreCandidates">显示更多（已显示 ${shown.length} / ${list.length}）</button>` : `<p class="table-foot">共 ${list.length} 个判定组</p>`);
}

function refreshCandidateFilterValues() {
  const ff = $('#candidateFilterField').value;
  const sel = $('#candidateFilterValue');
  if (ff === '' || !belief.ok) { sel.innerHTML = '<option value="">全部值</option>'; sel.disabled = true; return; }
  const f = Number(ff);
  const opts = engines.revealOptions({ events: events() }, f);
  sel.innerHTML = '<option value="">全部值</option>' + opts.map(([raw, mass]) => `<option value="${raw}">${escapeHtml(formatValue(f, raw))} · ${mass}</option>`).join('');
  sel.disabled = false;
}

function renderTrailPanel() {
  const s = active();
  // Follow the current puzzle whenever it changes; otherwise keep the tab the user picked.
  if (trailTab == null || trailTab > s.current || trailCurrent !== s.current) trailTab = s.current;
  trailCurrent = s.current;
  const beliefs = s.puzzles.map((p, i) => (i === s.current ? belief : engines.belief({ events: events(p) })));
  $('#trail').innerHTML = renderTrail({ state: s, data, beliefs, tab: trailTab, targets: session?.targets, hideTargets: session?.mode === 'game' && !session.revealed });
  const n = s.puzzles.reduce((acc, p) => acc + p.events.length, 0);
  $('#trailCount').textContent = `${s.puzzles.length} 题 · ${n} 步`;
  const scroller = $('#trail .trail-steps');
  if (scroller) scroller.scrollLeft = scroller.scrollWidth;
}

// ---------------------------------------------------------------------------------------------
// Recommendation

let recTimer = null;
function scheduleRecommend() {
  clearTimeout(recTimer);
  rec = null;
  renderRecommendation(null);
  recTimer = setTimeout(runRecommend, 30);
}

async function runRecommend() {
  const s = active();
  const token = ++recToken;
  if (!belief.ok || puzzle().solved) return renderRecommendation(null);
  const eff = S.effectiveInventory(s);
  if (eff.challenges <= 0) return renderRecommendation({ none: '挑战次数已用完。' });
  if (!puzzle().events.length) return renderRecommendation({ none: 'start' });
  $('#calcProgress').hidden = false;
  try {
    const r = await engines.recommend({ events: events(), hints: eff.hints, challenges: eff.challenges, remainingAfter: S.remainingAfter(s) });
    if (token !== recToken) return;
    rec = r;
    renderRecommendation(r);
  } catch (err) {
    if (token === recToken && err.message !== '已取消') renderRecommendation({ none: `计算失败：${err.message}` });
  } finally {
    if (token === recToken) $('#calcProgress').hidden = true;
  }
}

const METHOD_LABEL = {
  'exact-final': ['已证明最优（模型内）', '最后一题的精确最优解：在已知规则与卡池下不存在更好的策略。'],
  'exact-in-puzzle': ['题内精确 · 跨题估计', '本题内精确求解；后续题目的价值来自离线模拟拟合的估值表。'],
  horizon: ['近似最优（学习估值 · 深度2）', '两步前瞻搜索；更远的步骤用离线拟合的估值模型估算。'],
};

function renderRecommendation(r) {
  const card = $('#recommendationCard');
  const title = $('#recommendTitle');
  const label = $('#proofLabel');
  $('#recommendThumb').hidden = true;
  $('#recommendNote').hidden = true;
  $('#hintCompare').hidden = true;
  $('#alternatives').innerHTML = '';
  ['#metricPuzzle', '#metricSolve', '#metricEvent'].forEach((id) => { $(id).textContent = '—'; });
  label.hidden = true;
  title.disabled = true;
  delete title.dataset.hint;
  delete title.dataset.select;
  $('#methodNote').textContent = '推荐目标：预期解题数最多，其次首次相符项最多，再其次行动最少。';
  card.classList.add('empty-state');
  if (!r) {
    if (puzzle().solved) { title.textContent = '本题已完成'; $('#recommendReason').textContent = '进入下一题继续。'; }
    else if (!belief.ok) { title.textContent = '记录矛盾'; $('#recommendReason').textContent = '请先修正上方标出的录入错误。'; }
    else { title.textContent = '正在计算…'; $('#recommendReason').textContent = ''; }
    return;
  }
  if (r.none) {
    title.textContent = r.none === 'start' ? '录入初始揭示' : '无法推荐';
    $('#recommendReason').textContent = r.none === 'start' ? '加入本题开始时显示的字段，求解器会筛选候选并推荐下一步。' : r.none;
    return;
  }
  if (!r.ok) return renderRecommendation({ none: '记录矛盾' });
  if (!r.action) return renderRecommendation({ none: '没有可用的行动。' });
  card.classList.remove('empty-state');
  const [lab, explain] = METHOD_LABEL[r.method] || ['', ''];
  label.textContent = lab;
  label.dataset.method = r.method;
  label.hidden = false;
  const s = active();
  const solvedBefore = s.puzzles.slice(0, s.current).filter((p) => p.solved).length;
  $('#metricEvent').textContent = `${(solvedBefore + r.value[0]).toFixed(2)} / ${s.config.puzzles}`;
  if (r.forecast) $('#metricPuzzle').textContent = pct(r.forecast.pSolvePuzzle);
  const best = r.actions[0];
  if (r.action.type === 'hint') {
    title.textContent = '先使用提示';
    title.disabled = false;
    delete title.dataset.select;
    title.dataset.hint = '1';
    const unknownFields = FIELDS.filter((_, i) => belief.known[i] == null);
    let certain = `提示会从 ${unknownFields.length} 个未知字段中随机揭示一个。`;
    if (unknownFields.length === 1) {
      const fi = FIELDS.indexOf(unknownFields[0]);
      const distinct = engines.revealOptions({ events: events() }, fi).length;
      certain = `只剩「${unknownFields[0].label}」未知，提示必定揭示它，`
        + (distinct === belief.count ? `足以从 ${belief.count} 个候选中确定目标卡，之后一次挑战即可解出。` : `能把 ${belief.count} 个候选分成 ${distinct} 种情况。`);
    }
    $('#recommendReason').textContent = `${certain}${explain}`;
    $('#metricSolve').textContent = '—';
  } else {
    const g = r.action.gid;
    title.textContent = data.name(g);
    title.disabled = false;
    delete title.dataset.hint;
    title.dataset.select = g;
    $('#recommendThumb').innerHTML = thumbHtml(data, g, { zoom: true });
    $('#recommendThumb').hidden = false;
    $('#metricSolve').textContent = pct(best.pSolve);
    if (best.pSolve > 1 - 1e-9) {
      $('#recommendReason').textContent = belief.count === 1
        ? '只剩这一种可能：直接挑战这张卡即可解出本题，不需要再用提示或尝试其他卡。'
        : '剩下的候选都会被这张卡判定为六项全中：直接挑战即可解出本题。';
    } else $('#recommendReason').textContent = `${explain} 本次直接猜中 ${pct(best.pSolve)}，平均新增相符 ${best.eNewMatches.toFixed(2)} 项，信息量 ${best.entropy.toFixed(2)} bit（${best.outcomes} 种反馈）。`;
    const notes = [];
    if (!isConsistent(g)) notes.push('这张卡与已知条件不符：信息量更大，但需在游戏内重置卡片列表筛选才能选到。');
    if (r.ties.length > 1) notes.push(`另有 ${r.ties.length - 1} 个行动同样最优，任选其一。`);
    if (notes.length) { $('#recommendNote').textContent = notes.join(' '); $('#recommendNote').hidden = false; }
  }
  if (r.hintValue && r.bestChallengeValue && !(r.action.type === 'challenge' && best.pSolve > 1 - 1e-9)) {
    const d = r.hintValue[0] - r.bestChallengeValue[0];
    const box = $('#hintCompare');
    const fmt = (x) => (x >= 0.01 ? x.toFixed(2) : x.toFixed(3));
    box.textContent = Math.abs(d) < 0.0005
      ? '先用提示和直接挑战的预期解题数几乎相同（相差不到 0.001 题），按相符项和行动次数选择。'
      : d > 0 ? `先用提示：比直接挑战预期多解 ${fmt(d)} 题。` : `直接挑战：比先用提示预期多解 ${fmt(-d)} 题。`;
    box.hidden = false;
  }
  if (r.forecast) {
    $('#methodNote').textContent = `本题预计还需约 ${r.forecast.eChallenges.toFixed(1)} 次挑战、${r.forecast.eHints.toFixed(1)} 次提示（模拟估计）。候选 ${r.count} 组，熵 ${r.entropy.toFixed(2)} bit。`;
  }
  // Hide options that are pure waste: no chance to solve now and no information (e.g. when a single
  // candidate is left), and the hint when it cannot tell anything apart.
  const useful = (a) => (a.type === 'hint' ? belief.count > 1 : a.pSolve > 1e-9 || a.entropy > 1e-9);
  const sureSolve = r.action.type === 'challenge' && best.pSolve > 1 - 1e-9;
  const alts = sureSolve ? [] : r.actions.slice(1).filter(useful).slice(0, 5);
  const bestTotal = solvedBefore + r.value[0];
  $('#alternatives').innerHTML = alts.length ? `<div class="alt-head"><strong>其他选择</strong><span>右侧：选它之后，预计本活动共解出几题（推荐行动为 ${bestTotal.toFixed(3)} 题）。预计解题数相差不到 0.0001 题的选择视为同样好，此时依次比较相符项和本题预计行动次数。</span></div>` + alts.map((a) => {
    const name = a.type === 'hint' ? '使用提示' : escapeHtml(data.name(a.gid));
    const attr = a.type === 'hint' ? '' : `data-select="${a.gid}"`;
    const stats = a.type === 'hint'
      ? '随机揭示一个未知字段'
      : `直接猜中 ${pct(a.pSolve)} · 平均新增相符 ${a.eNewMatches.toFixed(2)} 项 · 信息量 ${a.entropy.toFixed(2)} bit · 本题预计行动 ${(-a.value[2]).toFixed(2)} 次`;
    const total = solvedBefore + a.value[0];
    return `<button type="button" class="alt-row" ${attr} title="选择后预计本活动共解出 ${total.toFixed(4)} 题"><strong>${name}</strong><span>${stats}</span><em>${total.toFixed(3)}<small>题</small></em></button>`;
  }).join('') : '';
}

// ---------------------------------------------------------------------------------------------
// Feedback dialog (real mode)

let feedbackMask = 0;
function openFeedback() {
  if (selected == null) return;
  feedbackMask = 0;
  const g = selected;
  $('#feedbackGrid').innerHTML = FIELDS.map((f, i) => `<button type="button" class="feedback-toggle" data-bit="${f.bit}"><span>${f.label}</span><strong>${escapeHtml(formatValue(i, data.value(g, i)))}</strong></button>`).join('');
  // Frame options: frames of candidates that the guess's frame would match.
  const cands = belief.ok ? belief.candidates : [];
  const gf = data.value(g, 0);
  const frames = [...new Set(cands.map((t) => data.value(t, 0)).filter((b) => b & gf))];
  $('#frameReveal').innerHTML = frames.map((b) => `<option value="${b}">${escapeHtml(formatValue(0, b))}</option>`).join('');
  if (frames.includes(gf)) $('#frameReveal').value = gf;
  const gm = data.col.levelMask[g];
  const levels = [...new Set(cands.filter((t) => data.col.levelMask[t] & gm).map((t) => data.value(t, 3)))].sort((a, b) => a - b);
  $('#levelReveal').innerHTML = levels.map((n) => `<option value="${n}">${n}</option>`).join('');
  if (levels.includes(data.value(g, 3))) $('#levelReveal').value = data.value(g, 3);
  updateFeedbackForm();
  $('#feedbackDialog').showModal();
}

function feedbackValue() {
  return {
    mask: feedbackMask,
    frame: feedbackMask & 1 ? Number($('#frameReveal').value) : null,
    level: feedbackMask & 8 ? Number($('#levelReveal').value) : null,
  };
}

function updateFeedbackForm() {
  $$('.feedback-toggle').forEach((b) => b.classList.toggle('is-on', Boolean(feedbackMask & Number(b.dataset.bit))));
  $('#frameRevealWrap').hidden = !(feedbackMask & 1);
  $('#levelRevealWrap').hidden = !(feedbackMask & 8);
  const fb = feedbackValue();
  const next = [...events(), { challenge: { guess: selected, mask: fb.mask, frame: fb.frame, level: fb.level } }];
  const b = engines.belief({ events: next });
  $('#feedbackCheck').textContent = b.ok
    ? `记录后剩余 ${b.count} 个判定组${fb.mask === 63 ? '（六项全部相符：本题完成）' : ''}。`
    : '⚠ 这个判定与已有记录矛盾，请再核对一次游戏画面。';
}

// ---------------------------------------------------------------------------------------------
// Settings

function customPresets() {
  try { return JSON.parse(localStorage.getItem(S.PRESET_KEY) || '[]'); } catch { return []; }
}

function openSettings() {
  const all = [...S.BUILTIN_PRESETS, ...customPresets()];
  $('#presetSelect').innerHTML = all.map((p) => `<option value="${p.id}">${escapeHtml(p.name)}</option>`).join('') + '<option value="custom">自定义当前设置</option>';
  $('#presetSelect').value = all.some((p) => p.id === state.presetId) ? state.presetId : 'custom';
  fillSettings(state.config);
  const s = state.settings;
  $('#ruleUnknownStat').checked = s.unknownStatMatches;
  $('#ruleLinkDef').checked = s.linkDefMatches;
  $('#ruleConsistentOnly').checked = s.consistentOnly;
  $('#poolSelect').value = s.pool;
  $('#settingsDialog').showModal();
}

function fillSettings(c) {
  const set = (id, v) => { $(id).value = v; };
  set('#cfgPuzzles', c.puzzles); set('#cfgDays', c.days);
  set('#cfgInitialChallenges', c.initialChallenges); set('#cfgInitialHints', c.initialHints);
  set('#cfgDailyChallenges', c.dailyChallenges); set('#cfgDailyHints', c.dailyHints);
  set('#cfgPremiumPuzzles', c.premiumPuzzles); set('#cfgSolvePoints', c.solvePoints);
  set('#cfgRegularMatchPoints', c.regularMatchPoints); set('#cfgRegularSolvePoints', c.regularSolvePoints);
  $('#milestoneRows').innerHTML = c.milestones.map((m) => milestoneRow(m)).join('');
}

const milestoneRow = (m) => `<div class="milestone-row"><label>相符 <input type="number" min="1" max="5" value="${m.matches}" data-ms="matches"> 项</label><label>奖励 <input type="number" min="0" value="${m.points}" data-ms="points"></label><button type="button" class="icon-btn" data-remove-ms aria-label="删除">×</button></div>`;

function settingsConfig() {
  const num = (id) => Number($(id).value);
  const milestones = $$('.milestone-row').map((row) => ({ matches: Number(row.querySelector('[data-ms=matches]').value), points: Number(row.querySelector('[data-ms=points]').value) }));
  return S.normalizeConfig({
    puzzles: num('#cfgPuzzles'), days: num('#cfgDays'),
    initialChallenges: num('#cfgInitialChallenges'), initialHints: num('#cfgInitialHints'),
    dailyChallenges: num('#cfgDailyChallenges'), dailyHints: num('#cfgDailyHints'),
    premiumPuzzles: num('#cfgPremiumPuzzles'), milestones, solvePoints: num('#cfgSolvePoints'),
    regularMatchPoints: num('#cfgRegularMatchPoints'), regularSolvePoints: num('#cfgRegularSolvePoints'),
  });
}

function settingsRules() {
  return {
    ...state.settings,
    unknownStatMatches: $('#ruleUnknownStat').checked,
    linkDefMatches: $('#ruleLinkDef').checked,
    consistentOnly: $('#ruleConsistentOnly').checked,
    pool: $('#poolSelect').value,
  };
}

// ---------------------------------------------------------------------------------------------
// Simulation

let simAbort = null;
async function runSimulation() {
  const rounds = Math.max(1, Math.min(20000, Number($('#simRounds').value) || 200));
  const seed = Number($('#simSeed').value) || 0;
  const c = state.config;
  const total = S.totalGrants(c);
  const preset = { puzzles: c.puzzles, hints: total.hints, challenges: total.challenges };
  simAbort = new AbortController();
  $('#runSimBtn').disabled = true;
  $('#stopSimBtn').hidden = false;
  $('#simProgress').hidden = false;
  const t0 = performance.now();
  const show = (done, m, final) => {
    $('#simProgress strong').textContent = `${done} / ${rounds}`;
    $('#simProgress span').style.width = `${(100 * done) / rounds}%`;
    const n = done || 1;
    const mean = m.solvedHist.reduce((s, v, i) => s + v * i, 0) / n;
    const variance = m.solvedHist.reduce((s, v, i) => s + v * (i - mean) ** 2, 0) / Math.max(1, n - 1);
    const all = m.solvedHist[c.puzzles] / n;
    const z = 1.96;
    const centre = (all + (z * z) / (2 * n)) / (1 + (z * z) / n);
    const half = (z * Math.sqrt((all * (1 - all)) / n + (z * z) / (4 * n * n))) / (1 + (z * z) / n);
    const maxBar = Math.max(...m.solvedHist, 1);
    $('#simResults').innerHTML = `<div class="sim-stats">
      <div><span>平均解题</span><strong>${mean.toFixed(3)}</strong><em>± ${(z * Math.sqrt(variance / n)).toFixed(3)}</em></div>
      <div><span>全部 ${c.puzzles} 题通关</span><strong>${pct(all)}</strong><em>[${pct(centre - half)}, ${pct(centre + half)}]</em></div>
      <div><span>平均用提示</span><strong>${(m.hints / n).toFixed(2)}</strong><em>/ ${total.hints}</em></div>
      <div><span>平均用挑战</span><strong>${(m.challenges / n).toFixed(2)}</strong><em>/ ${total.challenges}</em></div></div>
      <div class="sim-hist">${m.solvedHist.map((v, i) => `<div><i style="height:${(100 * v) / maxBar}%"></i><span>${i}</span><em>${v}</em></div>`).join('')}</div>
      <p class="modal-note">${final ? '完成' : '进行中'} · ${done} 次活动 · ${((performance.now() - t0) / 1000).toFixed(1)} 秒 · 种子 ${seed}。每次活动的目标卡和提示由种子决定，可复现。</p>`;
  };
  try {
    const m = await engines.simulate(preset, rounds, seed, (done, merged) => show(done, merged, false), simAbort.signal);
    show(m.done, m, true);
  } catch (err) {
    if (err.message !== '已取消') toast(`模拟失败：${err.message}`);
  } finally {
    $('#runSimBtn').disabled = false;
    $('#stopSimBtn').hidden = true;
    $('#simProgress').hidden = true;
  }
}

// ---------------------------------------------------------------------------------------------
// Events

function selectCard(g) {
  selected = g;
  switchTab('challenge');
  renderSelected();
  $('#searchResults').hidden = true;
  $('#cardSearch').value = '';
  window.scrollTo({ top: 0, behavior: 'smooth' });
}

function switchTab(name) {
  $$('.tab').forEach((t) => { const on = t.dataset.tab === name; t.classList.toggle('is-active', on); t.setAttribute('aria-selected', on); });
  $('#revealPane').hidden = name !== 'reveal';
  $('#challengePane').hidden = name !== 'challenge';
}

function toast(msg) {
  const t = $('#toast');
  t.textContent = msg;
  t.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { t.hidden = true; }, 3200);
}

function download(name, text) {
  const url = URL.createObjectURL(new Blob([text], { type: 'application/json' }));
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

function bind() {
  $$('.tab').forEach((t) => t.addEventListener('click', () => switchTab(t.dataset.tab)));
  $$('dialog [data-close]').forEach((b) => b.addEventListener('click', () => b.closest('dialog').close()));
  $('#revealField').addEventListener('change', populateRevealValues);
  $('#addRevealBtn').addEventListener('click', () => {
    const f = Number($('#revealField').value);
    const v = Number($('#revealValue').value);
    if (Number.isNaN(f) || $('#revealValue').value === '') return;
    addReveal(f, v, $('#revealSource').value);
  });
  $('#randomHintBtn').addEventListener('click', () => session && sessionHint());
  let searchTimer = null;
  $('#cardSearch').addEventListener('input', () => {
    clearTimeout(searchTimer);
    searchTimer = setTimeout(() => {
      const q = $('#cardSearch').value;
      const hits = data.search(q, state.settings.pool);
      const box = $('#searchResults');
      box.hidden = !q.trim();
      box.innerHTML = hits.length
        ? hits.map((g) => `<button type="button" data-select="${g}"><strong>${escapeHtml(data.name(g))}</strong><span>${FIELDS.map((_, f) => escapeHtml(formatValue(f, data.value(g, f)))).join(' · ')}</span></button>`).join('')
        : '<div class="empty-inline">没有找到</div>';
    }, 120);
  });
  document.addEventListener('click', (e) => {
    const sel = e.target.closest('[data-select]');
    if (sel && !sel.disabled) return selectCard(Number(sel.dataset.select));
    const zoom = e.target.closest('[data-zoom]');
    if (zoom) return openViewer(data, Number(zoom.dataset.zoom));
    const drop = e.target.closest('[data-drop-event]');
    if (drop) return commit((st) => { S.currentPuzzle(st).events.splice(Number(drop.dataset.dropEvent), 1); });
    const tab = e.target.closest('[data-trail-tab]');
    if (tab) { trailTab = Number(tab.dataset.trailTab); return renderTrailPanel(); }
    if (e.target.id === 'moreCandidates') { candidateLimit += 200; return renderCandidates(); }
    if (!e.target.closest('.search-block')) $('#searchResults').hidden = true;
  });
  $('#clearSelectedCard').addEventListener('click', () => { selected = null; renderSelected(); });
  $('#challengeBtn').addEventListener('click', () => {
    if (selected == null) return;
    if (session) return recordChallenge(selected, engines.judge(selected, session.targets[session.state.current]));
    openFeedback();
  });
  $('#feedbackGrid').addEventListener('click', (e) => {
    const b = e.target.closest('.feedback-toggle');
    if (!b) return;
    feedbackMask ^= Number(b.dataset.bit);
    updateFeedbackForm();
  });
  $('#frameReveal').addEventListener('change', updateFeedbackForm);
  $('#levelReveal').addEventListener('change', updateFeedbackForm);
  $('#feedbackForm').addEventListener('submit', (e) => {
    e.preventDefault();
    recordChallenge(selected, feedbackValue());
    $('#feedbackDialog').close();
  });
  $('#nextPuzzleBtn').addEventListener('click', nextPuzzle);
  $('#undoBtn').addEventListener('click', () => {
    const prev = undo.pop();
    if (!prev) return;
    if (session) session.state = prev; else { state = prev; S.save(state); }
    render();
  });
  $('#resetPuzzleBtn').addEventListener('click', () => {
    if (!confirm('清除本题所有记录？已消耗的次数会退回。')) return;
    commit((st) => {
      const p = S.currentPuzzle(st);
      for (const e of p.events) {
        if (e.t === 'challenge') st.inventory.challenges += 1;
        if (e.t === 'reveal' && e.src === 'hint') st.inventory.hints += 1;
      }
      p.events = [];
      p.solved = false;
    });
    if (session) {
      startSessionPuzzle();
      render();
    }
  });
  $('#resetEventBtn').addEventListener('click', () => {
    if (!confirm('重置整个活动？当前进度会清除（可撤销）。')) return;
    undo.push(state);
    state = S.freshState(state.config, state.presetId, state.settings);
    S.save(state);
    render();
  });
  const editStock = (id, key) => $(id).addEventListener('change', () => commit((st) => { st.inventory[key] = Math.max(0, Number($(id).value) || 0); }, { keepSelection: true }));
  editStock('#hintStock', 'hints');
  editStock('#challengeStock', 'challenges');
  $('#grantDays').addEventListener('change', () => commit((st) => {
    st.grants.received = Math.max(1, Math.min(st.config.days, Number($('#grantDays').value) || 1));
    st.grants.auto = false;
  }, { keepSelection: true }));
  $('#themeSelect').addEventListener('change', () => { state.settings.theme = $('#themeSelect').value; S.save(state); render(); });
  $('#testModeBtn').addEventListener('click', () => startSession('test'));
  $('#gameModeBtn').addEventListener('click', () => startSession('game'));
  $('#exitSessionBtn').addEventListener('click', exitSession);
  $('#newTargetBtn').addEventListener('click', () => {
    undo.push(session.state);
    const p = S.currentPuzzle(session.state);
    for (const e of p.events) {
      if (e.t === 'challenge') session.state.inventory.challenges += 1;
      if (e.t === 'reveal' && e.src === 'hint') session.state.inventory.hints += 1;
    }
    startSessionPuzzle();
    render();
  });
  $('#revealTargetBtn').addEventListener('click', () => { session.revealed = true; render(); });
  $('#recommendTitle').addEventListener('click', () => {
    const t = $('#recommendTitle');
    if (t.dataset.hint) {
      if (session) return sessionHint();
      switchTab('reveal');
      $('#revealSource').value = 'hint';
      return toast('在游戏中使用提示后，录入游戏揭示的字段和值。');
    }
    if (t.dataset.select != null) selectCard(Number(t.dataset.select));
  });
  $('#candidateSearch').addEventListener('input', () => { candidateLimit = 100; renderCandidates(); });
  $('#candidateFilterField').addEventListener('change', () => { refreshCandidateFilterValues(); candidateLimit = 100; renderCandidates(); });
  $('#candidateFilterValue').addEventListener('change', () => { candidateLimit = 100; renderCandidates(); });
  $('#settingsBtn').addEventListener('click', openSettings);
  $('#presetSelect').addEventListener('change', () => {
    const p = [...S.BUILTIN_PRESETS, ...customPresets()].find((x) => x.id === $('#presetSelect').value);
    if (p) fillSettings(S.normalizeConfig(p.config));
  });
  $('#addMilestoneBtn').addEventListener('click', () => $('#milestoneRows').insertAdjacentHTML('beforeend', milestoneRow({ matches: 1, points: 10 })));
  $('#milestoneRows').addEventListener('click', (e) => { if (e.target.closest('[data-remove-ms]')) e.target.closest('.milestone-row').remove(); });
  $('#savePresetBtn').addEventListener('click', () => {
    const name = $('#customPresetName').value.trim();
    if (!name) return toast('请输入预设名称。');
    const list = customPresets();
    list.push({ id: `local-${Date.now()}`, name, config: settingsConfig() });
    localStorage.setItem(S.PRESET_KEY, JSON.stringify(list));
    openSettings();
    toast('已保存本机预设。');
  });
  $('#deletePresetBtn').addEventListener('click', () => {
    const id = $('#presetSelect').value;
    const list = customPresets();
    if (!list.some((p) => p.id === id)) return toast('只能删除本机保存的预设。');
    localStorage.setItem(S.PRESET_KEY, JSON.stringify(list.filter((p) => p.id !== id)));
    openSettings();
  });
  $('#applyRulesBtn').addEventListener('click', async () => {
    state.settings = settingsRules();
    S.save(state);
    await applyPolicy();
    $('#settingsDialog').close();
    render();
  });
  $('#settingsForm').addEventListener('submit', async (e) => {
    e.preventDefault();
    if (!confirm('开始新活动？当前进度会清除（可撤销）。')) return;
    undo.push(state);
    const id = $('#presetSelect').value;
    state = S.freshState(settingsConfig(), id === 'custom' ? 'custom' : id, settingsRules());
    S.save(state);
    await applyPolicy();
    $('#settingsDialog').close();
    render();
  });
  $('#simulationBtn').addEventListener('click', () => {
    const c = state.config;
    const t = S.totalGrants(c);
    $('#simSummary').textContent = `当前设置：${c.puzzles} 题，全活动 ${t.hints} 次提示、${t.challenges} 次挑战；使用与推荐相同的策略，在后台多线程运行。`;
    $('#simulationDialog').showModal();
  });
  $('#simulationForm').addEventListener('submit', (e) => { e.preventDefault(); runSimulation(); });
  $('#stopSimBtn').addEventListener('click', () => { simAbort?.abort(); engines.stopSimulation(); });
  $('#exportBtn').addEventListener('click', () => download(`card-decoder-${new Date().toISOString().slice(0, 10)}.json`, S.exportJson(state)));
  $('#importBtn').addEventListener('click', () => $('#importFile').click());
  $('#importFile').addEventListener('change', async () => {
    const file = $('#importFile').files[0];
    if (!file) return;
    try {
      const s = S.importJson(await file.text(), (i) => window.LEGACY_PASSCODES?.[i]);
      undo.push(state);
      state = s;
      state.config = S.normalizeConfig(state.config);
      S.save(state);
      await applyPolicy();
      session = null;
      render();
      toast('已导入进度。');
    } catch (err) {
      toast(`导入失败：${err.message}`);
    }
    $('#importFile').value = '';
  });
  document.addEventListener('keydown', (e) => { if (e.key === 'Escape') $$('dialog[open]').forEach((d) => d.close()); });
}

// ---------------------------------------------------------------------------------------------
// Automation API (WebMCP)

const api = {
  summary() {
    const s = active();
    const eff = S.effectiveInventory(s);
    return {
      mode: session ? session.mode : 'activity',
      puzzle: s.current + 1,
      puzzles: s.config.puzzles,
      inventory: s.inventory,
      planned: { hints: eff.hints, challenges: eff.challenges },
      candidates: belief.ok ? belief.count : 0,
      known: belief.ok ? Object.fromEntries(FIELDS.map((f, i) => [f.key, belief.known[i] == null ? null : formatValue(i, belief.known[i])])) : null,
      solved: puzzle().solved,
    };
  },
  async recommendation() {
    await runRecommend();
    if (!rec || !rec.action) return { action: null };
    return {
      action: rec.action.type === 'hint' ? '使用提示' : `挑战 ${data.name(rec.action.gid)}（卡片密码 ${data.passcode(rec.action.gid)}）`,
      method: METHOD_LABEL[rec.method]?.[0],
      expectedPuzzlesThisEvent: rec.value[0],
    };
  },
  reveal(fieldKey, value, source) {
    const f = FIELDS.findIndex((x) => x.key === fieldKey);
    if (f < 0) throw new Error('unknown field');
    return addReveal(f, value, source === 'initial' ? 'initial' : 'hint');
  },
  challenge(passcode, matched, frame, level) {
    const g = data.gidOf(passcode);
    const mask = matched.reduce((m, k) => m | (FIELDS.find((x) => x.key === k)?.bit || 0), 0);
    recordChallenge(g, { mask, frame: frame ?? null, level: level ?? null });
    return api.summary();
  },
};
