// 活动轨迹 in the in-game 「记录」 style (PLAN.md §6.1): puzzle tabs, a sticky summary column with
// the target and its known fields, then one column per step — a lightbulb for reveals (initial or
// hint), the guessed card for challenges — with six rows each; matched rows are yellow.
import { FIELDS, ORDER, escapeHtml, formatValue, frameParts } from '../labels.js';
import { thumbHtml } from './images.js';

const ROW_ICONS = ['▣', '◉', '♞', '★', '⚔', '⛨'];

function valueText(f, raw) {
  if (raw == null) return '';
  if (f === 0) return frameParts(raw).map(escapeHtml).join('<br>');
  return escapeHtml(formatValue(f, raw));
}

function rows(values, matchedMask, shownMask) {
  return ORDER.map((f) => {
    const field = FIELDS[f];
    const on = shownMask & field.bit;
    const cls = !on ? 'row empty' : matchedMask & field.bit ? 'row hit' : 'row miss';
    return `<div class="${cls}"><i>${ROW_ICONS[f]}</i><b>${on ? valueText(f, values[f]) : ''}</b></div>`;
  }).join('');
}

/** "−2.35 bit" for the uncertainty a step removed (entropy before minus after). */
function bitsHtml(entropies, i) {
  const before = entropies?.[i];
  const after = entropies?.[i + 1];
  if (before == null || after == null) return '';
  const d = Math.max(0, before - after);
  return `<span class="bits" title="不确定性 ${before.toFixed(2)} → ${after.toFixed(2)} bit">−${d.toFixed(2)} bit</span>`;
}

export function renderTrail({ state, data, beliefs, tab, entropies, targets, hideTargets }) {
  const tabs = state.puzzles.map((p, i) => {
    const cls = ['trail-tab', i === tab ? 'is-active' : '', p.solved ? 'is-solved' : ''].join(' ');
    return `<button type="button" class="${cls}" data-trail-tab="${i}">Q${i + 1}${p.solved ? ' ✓' : ''}</button>`;
  }).join('');
  const p = state.puzzles[tab];
  const b = beliefs[tab];
  const known = b?.ok ? b.known : [null, null, null, null, null, null];
  const matched = b?.ok ? b.matched : 0;
  const knownMask = known.reduce((m, v, f) => (v == null ? m : m | (1 << f)), 0);
  const solvedBy = p.solved ? [...p.events].reverse().find((e) => e.t === 'challenge' && e.mask === 63) : null;
  let target = null;
  if (targets && targets[tab] != null && !hideTargets) target = targets[tab];
  else if (solvedBy) target = data.gidOf(solvedBy.pc);
  const summary = `<div class="trail-summary">${thumbHtml(data, target, { zoom: target != null })}<div class="trail-rows">${rows(known, matched, knownMask)}</div></div>`;
  const steps = p.events.map((e, i) => {
    const arrow = i > 0 ? '<div class="trail-arrow">▶</div>' : '';
    if (e.t === 'reveal') {
      const vals = []; vals[e.f] = e.v;
      const tag = e.src === 'initial' ? '初始' : '提示';
      return `${arrow}<div class="trail-step reveal"><div class="step-head"><div class="bulb" title="${tag}揭示">💡</div><em>${tag}</em></div><div class="trail-rows">${rows(vals, 0, 1 << e.f)}</div><div class="step-extra"><span></span>${bitsHtml(entropies, i)}</div></div>`;
    }
    const g = data.gidOf(e.pc);
    const vals = FIELDS.map((_, f) => data.value(g, f));
    const extras = [e.rec ? '<span class="followed" title="按推荐挑战">★</span>' : '', e.n != null && e.mask !== 63 ? `<span>余 ${e.n}</span>` : ''].join('');
    return `${arrow}<div class="trail-step challenge${e.mask === 63 ? ' solved' : ''}"><div class="step-head">${e.mask === 63 ? '<div class="solved-badge">✓</div>' : ''}${thumbHtml(data, g, { zoom: true, small: true })}<em title="${escapeHtml(data.name(g))}">${escapeHtml(data.name(g))}</em></div><div class="trail-rows">${rows(vals, e.mask, 63)}</div><div class="step-extra">${extras}</div><div class="step-extra"><span></span>${bitsHtml(entropies, i)}</div></div>`;
  }).join('');
  const empty = p.events.length ? '' : `<div class="empty-inline">${p.skipped ? '这一题在开始使用本工具之前完成，没有记录。' : '本题还没有记录。'}</div>`;
  return `<div class="trail-tabs" role="tablist">${tabs}</div><div class="trail-body">${summary}<div class="trail-steps">${steps}${empty}</div></div>`;
}
