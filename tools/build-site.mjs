// Assemble public/ — the folder uploaded to Cloudflare (dashboard upload, no CLI) and zipped for
// offline use. Only generated, whitelisted files are written, so stray local folders never leak.
//
// Inputs: build/wasm (npm run build:wasm), data/generated (npm run data:build), tables/tables.bin,
//         web/ (page source), optional build/sheets (npm run images), optional legacy cards-data.js.
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import * as esbuild from 'esbuild';
import { root } from './lib.mjs';

const pub = path.join(root, 'public');
const MAX_FILES = 900; // dashboard uploads allow 1,000 files
const MAX_FILE_BYTES = 20 * 1024 * 1024; // hard limit is 25 MiB
const spike = process.argv.includes('--spike');

const hash = (buf) => crypto.createHash('sha256').update(buf).digest('hex').slice(0, 10);
const read = (...p) => fs.readFileSync(path.join(root, ...p));
const exists = (...p) => fs.existsSync(path.join(root, ...p));
function emit(rel, content) {
  const file = path.join(pub, rel);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, content);
  return rel;
}
/** Write `content` under a content-hashed name, e.g. engine/worker.<hash>.js. */
const emitHashed = (dir, base, ext, content) => emit(`${dir}/${base}.${hash(content)}${ext}`, content);
const jsString = (s) => JSON.stringify(s).replace(/<\/script/gi, '<\\/script');
const assign = (name, value) => `window.${name}=${value};\n`;

async function bundle(entry) {
  const out = await esbuild.build({
    entryPoints: [path.join(root, entry)], bundle: true, format: 'iife', minify: true, write: false,
    target: 'es2022', logLevel: 'warning', legalComments: 'none',
  });
  return out.outputFiles[0].text;
}

for (const need of [['build', 'wasm', 'decoder_bg.wasm'], ['data', 'generated', 'cards.bin'], ['data', 'generated', 'names.json']]) {
  if (!exists(...need)) {
    console.error(`missing ${need.join('/')}: run npm run data:build (or use the committed data/generated) and npm run build:wasm first`);
    process.exit(1);
  }
}
fs.rmSync(pub, { recursive: true, force: true });

// Engine: wasm (https), base64 wasm (file://), worker bundle (https) and as a string (file://).
const wasm = read('build', 'wasm', 'decoder_bg.wasm');
const workerJs = await bundle('web/engine/worker-entry.js');
const files = {
  wasm: emitHashed('engine', 'decoder_bg', '.wasm', wasm),
  wasmB64: emitHashed('engine', 'wasm-b64', '.js', assign('ENGINE_WASM_B64', jsString(wasm.toString('base64')))),
  worker: emitHashed('engine', 'worker', '.js', workerJs),
  workerSource: emitHashed('engine', 'worker-src', '.js', assign('ENGINE_WORKER_SOURCE', jsString(workerJs))),
};

// Data (classic scripts, identical in both modes).
const names = JSON.parse(read('data', 'generated', 'names.json'));
const tables = exists('tables', 'tables.bin') ? read('tables', 'tables.bin') : Buffer.alloc(0);
if (!tables.length) console.warn('tables/tables.bin missing: the page falls back to the legacy continuation formula');
const dataScripts = [
  emitHashed('data', 'cards', '.js', assign('CARDS_BIN_B64', jsString(read('data', 'generated', 'cards.bin').toString('base64')))),
  emitHashed('data', 'tables', '.js', assign('TABLES_BIN_B64', jsString(tables.toString('base64')))),
  emitHashed('data', 'names', '.js', assign('CARD_NAMES', JSON.stringify(names.display))),
];
files.aliases = emitHashed('data', 'aliases', '.js', assign('CARD_ALIASES', JSON.stringify(names.aliases)));

// Old saves (v4) refer to cards by their index in the legacy cards-data.js.
const legacyPath = ['legacy/cards-data.js'].find((p) => exists(p));
if (legacyPath) {
  const text = read(legacyPath).toString('utf8').trim().replace(/^window\.CARD_DATA=/, '').replace(/;$/, '');
  const legacy = JSON.parse(text).cards.map((c) => c.ids[0]);
  dataScripts.push(emitHashed('data', 'legacy-passcodes', '.js', assign('LEGACY_PASSCODES', JSON.stringify(legacy))));
}

// Card thumbnails packed into sprite sheets (optional; see tools/images.mjs).
if (exists('build', 'sheets', 'sheets.json')) {
  const meta = JSON.parse(read('build', 'sheets', 'sheets.json'));
  const hashedFiles = meta.files.map((f) => emitHashed('cards', path.basename(f, '.webp'), '.webp', read('build', 'sheets', f)));
  dataScripts.push(emitHashed('data', 'sheets', '.js', assign('CARD_SHEETS', JSON.stringify({ ...meta, files: hashedFiles }))));
}

// Page.
const styles = emitHashed('assets', 'styles', '.css', read('web', 'styles.css'));
const app = emitHashed('assets', 'app', '.js', await bundle('web/js/main.js'));
const scripts = [
  `<script>window.ENGINE_FILES=${JSON.stringify(files)};</script>`,
  ...dataScripts.map((s) => `<script src="${s}"></script>`),
  `<script src="${app}"></script>`,
].join('\n  ');
emit('index.html', read('web', 'index.html').toString('utf8').replace('%STYLES%', styles).replace('%SCRIPTS%', scripts));
for (const f of ['404.html', '_headers']) if (exists('web', f)) emit(f, read('web', f));
if (spike) {
  emit('engine/files.js', assign('ENGINE_FILES', JSON.stringify({ ...files, cards: dataScripts[0] })));
  emit('engine/engine-client.js', read('web', 'engine', 'engine-client.js'));
  emit('spike.html', read('web', 'spike', 'spike.html'));
}

// Upload guard.
const all = [];
(function walk(dir) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    e.isDirectory() ? walk(p) : all.push(p);
  }
})(pub);
const big = all.filter((f) => fs.statSync(f).size > MAX_FILE_BYTES);
if (all.length > MAX_FILES || big.length) {
  console.error(`public/ has ${all.length} files (max ${MAX_FILES}); oversized: ${big.join(', ') || 'none'}`);
  process.exit(1);
}
const total = all.reduce((s, f) => s + fs.statSync(f).size, 0);
console.log(`public/: ${all.length} files, ${(total / 1024 / 1024).toFixed(2)} MiB`);
