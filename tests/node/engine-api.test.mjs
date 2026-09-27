// The browser engine API end to end (JSON in, JSON out), using the real data and fitted tables.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';

const root = path.resolve(import.meta.dirname, '..', '..');
const wasmDir = path.join(root, 'build', 'wasm');
const cardsBin = path.join(root, 'data', 'generated', 'cards.bin');
const tablesBin = path.join(root, 'tables', 'tables.bin');
const ready = [path.join(wasmDir, 'decoder.js'), cardsBin, tablesBin].every((p) => fs.existsSync(p));

async function engine() {
  const { initSync, Engine } = await import(path.join(wasmDir, 'decoder.js'));
  initSync({ module: fs.readFileSync(path.join(wasmDir, 'decoder_bg.wasm')) });
  return new Engine(fs.readFileSync(cardsBin), fs.readFileSync(tablesBin));
}

test('belief, reveal options, judge and recommend', { skip: !ready && 'build data, tables and wasm first' }, async () => {
  const e = await engine();
  const cols = JSON.parse(e.columns());
  const gidOf = (passcode) => cols.passcode.indexOf(passcode);
  // Real-play Q3: target 焰征龙－爆破 (53804307), initial reveal 效果.
  const target = gidOf(53804307);
  assert.ok(target >= 0);
  const state = { events: [{ reveal: { field: 0, value: 2 } }], hints: 11, challenges: 36, remainingAfter: 8 };
  const b = JSON.parse(e.belief(JSON.stringify(state)));
  assert.equal(b.ok, true);
  assert.ok(b.count > 4000 && b.candidates.includes(target));
  const opts = JSON.parse(e.revealOptions(JSON.stringify(state), 1));
  assert.ok(opts.length >= 6 && opts.every(([raw, mass, count]) => mass > 0 && count > 0));
  // Judge 小丑阁下 (72992744) against the target: only the frame matches.
  const j = JSON.parse(e.judge(gidOf(72992744), target));
  assert.equal(j.mask, 1);
  assert.equal(j.frame, 2);
  const t0 = performance.now();
  const r = JSON.parse(e.recommend(JSON.stringify(state)));
  const ms = performance.now() - t0;
  assert.equal(r.ok, true);
  assert.ok(['hint', 'challenge'].includes(r.action.type));
  assert.equal(r.method, 'horizon');
  assert.ok(r.actions.length >= 2);
  assert.ok(r.forecast && r.forecast.pSolvePuzzle > 0.5);
  console.log(`  recommend at the n=${b.count} opening: ${ms.toFixed(1)} ms in wasm → ${JSON.stringify(r.action)} (${r.method})`);
});

test('contradictions are reported with suggestions', { skip: !ready && 'build first' }, async () => {
  const e = await engine();
  const state = { events: [{ reveal: { field: 3, value: 12 } }, { reveal: { field: 4, value: 100 } }, { reveal: { field: 5, value: 5000 } }] };
  const b = JSON.parse(e.belief(JSON.stringify(state)));
  assert.equal(b.ok, false);
  assert.ok(Number.isInteger(b.contradiction.event));
  assert.ok(b.contradiction.suggestions.length > 0);
});

test('exact tier on a small final puzzle', { skip: !ready && 'build first' }, async () => {
  const e = await engine();
  const cols = JSON.parse(e.columns());
  const gidOf = (passcode) => cols.passcode.indexOf(passcode);
  const target = gidOf(53804307);
  // Q3 log up to 雪尘龙: few candidates left.
  const events = [{ reveal: { field: 0, value: 2 } }];
  for (const pc of [72992744, 78922939, 94689635, 88453933, 67675300]) {
    const j = JSON.parse(e.judge(gidOf(pc), target));
    events.push({ challenge: { guess: gidOf(pc), mask: j.mask, frame: j.frame, level: j.level } });
  }
  const b = JSON.parse(e.belief(JSON.stringify({ events })));
  assert.ok(b.ok && b.candidates.includes(target));
  const r = JSON.parse(e.recommend(JSON.stringify({ events, hints: 1, challenges: 3, remainingAfter: 0 })));
  assert.equal(r.method, 'exact-final');
  console.log(`  endgame: ${b.count} candidates → ${JSON.stringify(r.action)}, value ${r.value.map((x) => x.toFixed(3)).join(', ')}`);
});

test('simulateBatch runs in wasm', { skip: !ready && 'build first' }, async () => {
  const e = await engine();
  const t0 = performance.now();
  const out = JSON.parse(e.simulateBatch(JSON.stringify({ puzzles: 9, hints: 11, challenges: 36 }), 0, 1, 0, 3));
  assert.equal(out.solvedHist.reduce((a, b) => a + b, 0), 3);
  console.log(`  3 simulated events in wasm: ${(performance.now() - t0).toFixed(0)} ms, histogram ${out.solvedHist.join(',')}`);
});
