// Saved state (v5): cards are stored by passcode (stable across data rebuilds), every puzzle keeps
// its full event log (for the 活动轨迹 view), and resources follow the official grant schedule.

export const STORAGE_KEY = 'card-decoder-state-v5';
export const LEGACY_KEY = 'card-decoder-state-v4';
export const PRESET_KEY = 'card-decoder-custom-presets-v2';

export const BUILTIN_PRESETS = [
  {
    id: 'md-2026-09',
    name: '大师决斗 · 卡片解码者（9题 · 2026年9月）',
    config: {
      puzzles: 9,
      days: 9,
      initialChallenges: 4,
      initialHints: 3,
      dailyChallenges: 4,
      dailyHints: 1,
      premiumPuzzles: 3,
      milestones: [{ matches: 1, points: 10 }, { matches: 3, points: 10 }, { matches: 5, points: 10 }],
      solvePoints: 70,
      regularMatchPoints: 1,
      regularSolvePoints: 0,
    },
  },
];

const clampInt = (v, lo, hi, dflt) => {
  const n = Number.parseInt(v, 10);
  return Number.isFinite(n) ? Math.min(hi, Math.max(lo, n)) : dflt;
};

export function normalizeConfig(c = {}) {
  const d = BUILTIN_PRESETS[0].config;
  const puzzles = clampInt(c.puzzles, 1, 30, d.puzzles);
  return {
    puzzles,
    days: clampInt(c.days, 1, 60, d.days),
    initialChallenges: clampInt(c.initialChallenges, 0, 99, d.initialChallenges),
    initialHints: clampInt(c.initialHints, 0, 99, d.initialHints),
    dailyChallenges: clampInt(c.dailyChallenges, 0, 99, d.dailyChallenges),
    dailyHints: clampInt(c.dailyHints, 0, 99, d.dailyHints),
    premiumPuzzles: clampInt(c.premiumPuzzles, 0, puzzles, d.premiumPuzzles),
    milestones: (Array.isArray(c.milestones) ? c.milestones : d.milestones)
      .map((m) => ({ matches: clampInt(m.matches, 1, 5, 1), points: clampInt(m.points, 0, 99999, 0) }))
      .sort((a, b) => a.matches - b.matches)
      .filter((m, i, list) => i === 0 || m.matches !== list[i - 1].matches),
    solvePoints: clampInt(c.solvePoints, 0, 99999, d.solvePoints),
    regularMatchPoints: clampInt(c.regularMatchPoints, 0, 99999, d.regularMatchPoints),
    regularSolvePoints: clampInt(c.regularSolvePoints, 0, 99999, d.regularSolvePoints),
  };
}

export const totalGrants = (config) => ({
  challenges: config.initialChallenges + (config.days - 1) * config.dailyChallenges,
  hints: config.initialHints + (config.days - 1) * config.dailyHints,
});

export function freshState(config = BUILTIN_PRESETS[0].config, presetId = BUILTIN_PRESETS[0].id, settings = null) {
  const c = normalizeConfig(config);
  return {
    v: 5,
    config: c,
    presetId,
    settings: settings || { pool: 'md', unknownStatMatches: true, linkDefMatches: true, consistentOnly: false, theme: 'dark' },
    grants: { received: 1, startDate: new Date().toISOString().slice(0, 10) },
    inventory: { challenges: c.initialChallenges, hints: c.initialHints },
    current: 0,
    puzzles: [{ events: [], solved: false }],
    activityId: `activity-${Date.now()}`,
  };
}

/** Grant days received by now given the first login date (first day at login, then daily at 13:00). */
export function grantsReceivedByDate(startDate, days, now = new Date()) {
  if (!startDate) return 1;
  const [y, m, d] = startDate.split('-').map(Number);
  const start = new Date(y, m - 1, d);
  let received = 1;
  for (let day = 1; day < days; day++) {
    const at = new Date(start.getFullYear(), start.getMonth(), start.getDate() + day, 13, 0, 0);
    if (now >= at) received = day + 1;
  }
  return received;
}

/** Inventory the solver plans with: what is available now plus the grants still to come. */
export function effectiveInventory(state) {
  const c = state.config;
  const left = Math.max(0, c.days - state.grants.received);
  return {
    hints: state.inventory.hints + left * c.dailyHints,
    challenges: state.inventory.challenges + left * c.dailyChallenges,
    futureHints: left * c.dailyHints,
    futureChallenges: left * c.dailyChallenges,
  };
}

export const currentPuzzle = (state) => state.puzzles[state.current];
export const remainingAfter = (state) => Math.max(0, state.config.puzzles - state.current - 1);
export const isPremium = (state, index = state.current) => index < state.config.premiumPuzzles;

/** Points earned in a puzzle by the matched fields and solve (event reward display). */
export function puzzlePoints(state, index, matchedCount, solved) {
  const c = state.config;
  if (isPremium(state, index)) {
    return c.milestones.reduce((s, m) => s + (matchedCount >= m.matches ? m.points : 0), 0) + (solved ? c.solvePoints : 0);
  }
  return matchedCount * c.regularMatchPoints + (solved ? c.regularSolvePoints : 0);
}

/** Engine state for a puzzle: events with group ids. `gidOf(passcode)` maps passcodes. */
export function engineEvents(puzzle, gidOf) {
  return puzzle.events.map((e) => (e.t === 'reveal'
    ? { reveal: { field: e.f, value: e.v } }
    : { challenge: { guess: gidOf(e.pc), mask: e.mask, frame: e.frame ?? null, level: e.level ?? null } }));
}

export function load(dataHash) {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) {
      const s = JSON.parse(raw);
      if (s && s.v === 5) {
        s.config = normalizeConfig(s.config);
        s.settings = { pool: 'md', unknownStatMatches: true, linkDefMatches: true, consistentOnly: false, theme: 'dark', ...s.settings };
        s.grants = s.grants || { received: 1, startDate: null };
        return s;
      }
    }
  } catch { /* storage unavailable or corrupt: start fresh */ }
  return null;
}

export function save(state) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  } catch { /* private mode or quota: keep working in memory */ }
}

/** Convert a v4 (legacy tool) save. `legacyPasscode(i)` maps a legacy card index to a passcode. */
export function fromV4(old, legacyPasscode) {
  const c = normalizeConfig({
    ...BUILTIN_PRESETS[0].config,
    puzzles: old.config?.puzzles,
    premiumPuzzles: old.config?.premiumPuzzles,
    milestones: old.config?.milestones,
    solvePoints: old.config?.solvePoints,
    regularMatchPoints: old.config?.regularMatchPoints,
    regularSolvePoints: old.config?.regularSolvePoints,
  });
  const s = freshState(c);
  const fieldIndex = { border: 0, attribute: 1, race: 2, number: 3, attack: 4, defense: 5 };
  s.current = Math.max(0, Math.min(c.puzzles - 1, (old.puzzle || 1) - 1));
  s.puzzles = Array.from({ length: s.current + 1 }, (_, i) => ({ events: [], solved: i < s.current }));
  s.inventory = { hints: old.hints ?? c.initialHints, challenges: old.challenges ?? c.initialChallenges };
  s.grants = { received: c.days, startDate: null }; // v4 stored totals, so all grants are counted
  s.settings.pool = old.pool === 'all' ? 'all' : 'md';
  s.settings.theme = old.theme === 'light' ? 'light' : 'dark';
  const events = s.puzzles[s.current].events;
  for (const log of old.logs || []) {
    if (log.type === 'reveal' && log.field in fieldIndex) {
      events.push({ t: 'reveal', f: fieldIndex[log.field], v: Number(log.value), src: log.source === 'initial' ? 'initial' : 'hint' });
    } else if (log.type === 'challenge') {
      const pc = legacyPasscode(log.guess);
      if (pc == null) continue;
      events.push({ t: 'challenge', pc, mask: log.mask, frame: log.borderReveal ?? null, level: log.numberReveal ?? null });
    }
  }
  s.puzzles[s.current].solved = Boolean(old.solved);
  return s;
}

/** Undo stack of JSON snapshots. */
export class History {
  constructor(limit = 50) {
    this.limit = limit;
    this.stack = [];
  }
  push(state) {
    this.stack.push(JSON.stringify(state));
    if (this.stack.length > this.limit) this.stack.shift();
  }
  pop() {
    const s = this.stack.pop();
    return s ? JSON.parse(s) : null;
  }
  get size() {
    return this.stack.length;
  }
}

export function exportJson(state) {
  return JSON.stringify({ format: 'md-card-decoder', v: 5, exportedAt: new Date().toISOString(), state }, null, 1);
}

export function importJson(text, legacyPasscode) {
  const o = JSON.parse(text);
  if (o && o.format === 'md-card-decoder' && o.state?.v === 5) return o.state;
  if (o && o.v === 5) return o;
  if (o && Array.isArray(o.logs) && o.config) return fromV4(o, legacyPasscode);
  throw new Error('无法识别的存档格式');
}
