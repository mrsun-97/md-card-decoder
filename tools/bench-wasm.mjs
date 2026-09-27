// Measure the WASM/native speed factor on the partition kernel (M1 spike).
// Usage: node tools/bench-wasm.mjs   (after `npm run build:wasm` and `mdcd data from-legacy`)
import fs from 'node:fs';
import path from 'node:path';
import { root } from './lib.mjs';

const { initSync, Engine } = await import(path.join(root, 'build', 'wasm', 'decoder.js'));
initSync({ module: fs.readFileSync(path.join(root, 'build', 'wasm', 'decoder_bg.wasm')) });
const engine = new Engine(fs.readFileSync(path.join(root, "build", "data", "cards.bin")), new Uint8Array());
console.log(`groups ${engine.groups}, MD ${engine.mdGroups}, data hash ${engine.dataHash}`);

// Same states as `mdcd bench`: candidates sharing one shown value, rebuilt from cards.bin columns.
const bin = fs.readFileSync(path.join(root, 'build', 'data', 'cards.bin'));
const cols = readColumns(bin);
const md = [...Array(cols.nMd).keys()];
const states = [
  ['frame=effect', md.filter((g) => cols.dict.frame[cols.frame[g]] === 2)],
  ['level=4', md.filter((g) => cols.levelN[g] === 4)],
  ['attribute=dark', md.filter((g) => cols.dict.attribute[cols.attribute[g]] === 32)],
  ['atk=1500', md.filter((g) => cols.dict.atk[cols.atk[g]] === 1500)],
];
for (const [name, cands] of states) {
  const arr = Uint16Array.from(cands);
  engine.benchPartitionAll(arr); // warm-up
  const reps = cands.length > 1000 ? 3 : 20;
  const t0 = performance.now();
  for (let i = 0; i < reps; i++) engine.benchPartitionAll(arr);
  const ms = (performance.now() - t0) / reps;
  console.log(`${name.padStart(15)}: n=${String(cands.length).padStart(5)}  all-guess partition ${ms.toFixed(2).padStart(7)} ms  ${(ms * 1e6 / (cols.nMd * cands.length)).toFixed(2)} ns/pair`);
}

function readColumns(b) {
  const dv = new DataView(b.buffer, b.byteOffset, b.byteLength);
  let p = 8;
  const n = dv.getUint32(p, true); p += 4;
  const nMd = dv.getUint32(p, true); p += 4 + 8;
  const dict = {};
  const read = (name, size, get) => {
    const c = dv.getUint16(p, true); p += 2;
    dict[name] = Array.from({ length: c }, (_, i) => get(p + i * size));
    p += c * size;
  };
  read('frame', 1, (o) => dv.getUint8(o));
  read('attribute', 1, (o) => dv.getUint8(o));
  read('race', 4, (o) => dv.getUint32(o, true));
  read('levelNm', 2, (o) => dv.getUint16(o, true));
  read('atk', 2, (o) => dv.getInt16(o, true));
  read('def', 2, (o) => dv.getInt16(o, true));
  const col = () => { const c = b.subarray(p, p + n); p += n; return c; };
  const frame = col(), attribute = col(), race = col(), level = col(), levelN = col(), atk = col();
  return { n, nMd, dict, frame, attribute, race, level, levelN, atk };
}
