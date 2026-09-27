// Card data for the page: raw field columns (from the engine), display names, lazily loaded
// search aliases, passcode ↔ group maps and the thumbnail sprite index.
import { normalizeSearch } from './labels.js';

export class CardData {
  constructor(columns, names) {
    this.col = columns;
    this.names = names; // display name per group
    this.count = columns.frame.length;
    this.byPasscode = new Map();
    columns.passcode.forEach((pc, g) => this.byPasscode.set(pc, g));
    this.aliases = null; // normalised alias strings per group, loaded on demand
  }

  gidOf(passcode) {
    const g = this.byPasscode.get(passcode);
    if (g === undefined) throw new Error(`卡片密码 ${passcode} 不在当前卡库中`);
    return g;
  }

  passcode(g) { return this.col.passcode[g]; }
  name(g) { return this.names[g] ?? `#${this.col.passcode[g]}`; }

  /** Raw value of field `f` (0..5) for group `g`. */
  value(g, f) {
    const c = this.col;
    return [c.frame, c.attribute, c.race, c.level, c.atk, c.def][f][g];
  }

  weight(g, pool) { return pool === 'all' ? this.col.wa[g] : this.col.wm[g]; }

  /** Attach aliases (array of arrays per group) for search. */
  setAliases(aliases) {
    this.aliases = aliases.map((list, g) => [this.names[g], ...list].map(normalizeSearch));
  }

  /** Groups whose names match `query` (prefix matches first), limited to `pool`. */
  search(query, pool, limit = 24) {
    const q = normalizeSearch(query);
    if (!q) return [];
    const lists = this.aliases || this.names.map((n) => [normalizeSearch(n)]);
    const hits = [];
    for (let g = 0; g < this.count; g++) {
      if (this.weight(g, pool) === 0) continue;
      let best = -1;
      for (const a of lists[g]) {
        const i = a.indexOf(q);
        if (i === 0) { best = 0; break; }
        if (i > 0 && best < 0) best = 1;
      }
      if (best >= 0) hits.push([best, this.names[g].length, g]);
    }
    hits.sort((a, b) => a[0] - b[0] || a[1] - b[1] || a[2] - b[2]);
    return hits.slice(0, limit).map((h) => h[2]);
  }
}
