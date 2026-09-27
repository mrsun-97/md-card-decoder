// Check the development environment. `node tools/doctor.mjs` (or `npm run doctor`).
import fs from 'node:fs';
import path from 'node:path';
import { capture, lockedVersion, root } from './lib.mjs';

let problems = 0;
const ok = (msg) => console.log(`  ✓ ${msg}`);
const bad = (msg, fix) => { problems++; console.log(`  ✗ ${msg}${fix ? `\n      → ${fix}` : ''}`); };
const warn = (msg) => console.log(`  ! ${msg}`);

console.log('Rust');
const rustc = capture('rustc', ['--version']);
rustc ? ok(rustc) : bad('rustc not found', 'install rustup');
const targets = capture('rustup', ['target', 'list', '--installed']) ?? '';
targets.includes('wasm32-unknown-unknown')
  ? ok('wasm32-unknown-unknown target')
  : bad('wasm32 target missing', 'rustup target add wasm32-unknown-unknown');

console.log('wasm-bindgen');
const want = fs.existsSync(path.join(root, 'Cargo.lock')) ? lockedVersion('wasm-bindgen') : null;
const have = capture('wasm-bindgen', ['--version'])?.split(' ')[1];
if (!want) warn('Cargo.lock has no wasm-bindgen yet (run cargo build first)');
else if (have === want) ok(`CLI ${have} matches Cargo.lock`);
else bad(`CLI ${have ?? '(missing)'} ≠ crate ${want}`, `cargo install wasm-bindgen-cli --version ${want} --locked --root .tools`);
capture('wasm-opt', ['--version']) ? ok('wasm-opt (optional)') : warn('wasm-opt not installed (optional)');

console.log('Node');
ok(`node ${process.version}`);
fs.existsSync(path.join(root, 'node_modules', 'esbuild')) ? ok('esbuild') : bad('esbuild missing', 'npm ci');

console.log('Browser tests (optional)');
try {
  const b = await (await import('./lib.mjs')).launchBrowser();
  ok(`headless Chromium ${b.version()}`);
  await b.close();
} catch (e) {
  const msg = String(e.message || e);
  if (msg.includes("Executable doesn't exist")) warn('Playwright browser missing → npx playwright install chromium-headless-shell');
  else if (msg.includes('libasound')) warn('libasound missing → sudo pacman -S alsa-lib (or: node tools/build-alsa-stub.mjs)');
  else warn(`cannot launch Chromium: ${msg.split('\n')[0]}`);
}

console.log('Network tools');
const curl = capture('which', ['curl']);
if (curl && curl.includes('anaconda')) warn(`curl resolves to ${curl} (broken CA bundle); scripts use /usr/bin/curl`);

console.log(problems ? `\n${problems} problem(s) found.` : '\nAll required tools present.');
process.exit(problems ? 1 : 0);
