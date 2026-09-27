// Build the engine for the browser: cargo → wasm-bindgen (→ wasm-opt if installed).
// Output: build/wasm/decoder.js + decoder_bg.wasm (+ .d.ts for // @ts-check).
import fs from 'node:fs';
import path from 'node:path';
import { capture, lockedVersion, root, run } from './lib.mjs';

const outDir = path.join(root, 'build', 'wasm');
run('cargo', ['build', '-p', 'decoder-wasm', '--target', 'wasm32-unknown-unknown', '--release']);

const want = lockedVersion('wasm-bindgen');
const have = capture('wasm-bindgen', ['--version'])?.split(' ')[1];
if (have !== want) {
  console.error(`wasm-bindgen CLI ${have ?? '(not found)'} ≠ crate ${want} in Cargo.lock.`);
  console.error(`Install it with: cargo install wasm-bindgen-cli --version ${want} --locked --root .tools`);
  process.exit(1);
}
fs.rmSync(outDir, { recursive: true, force: true });
run('wasm-bindgen', [
  '--target', 'web', '--omit-default-module-path', '--out-dir', outDir, '--out-name', 'decoder',
  path.join(root, 'target', 'wasm32-unknown-unknown', 'release', 'decoder_wasm.wasm'),
]);

const wasm = path.join(outDir, 'decoder_bg.wasm');
if (capture('wasm-opt', ['--version'])) {
  // rustc ≥ 1.87 enables these features by default; wasm-opt must be told explicitly.
  run('wasm-opt', ['-O3', '--enable-bulk-memory', '--enable-nontrapping-float-to-int', '--enable-sign-ext',
    '--enable-mutable-globals', '--enable-reference-types', '--enable-multivalue', wasm, '-o', wasm]);
} else {
  console.log('wasm-opt not installed; skipping (optional).');
}
console.log(`wasm: ${(fs.statSync(wasm).size / 1024).toFixed(1)} KiB → ${path.relative(root, outDir)}`);
