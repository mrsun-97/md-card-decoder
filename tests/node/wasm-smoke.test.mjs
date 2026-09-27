// The browser build of the engine loads the same data as the native CLI.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';

const root = path.resolve(import.meta.dirname, '..', '..');
const wasmDir = path.join(root, 'build', 'wasm');
const cardsBin = path.join(root, 'data', 'generated', 'cards.bin');
const ready = fs.existsSync(path.join(wasmDir, 'decoder.js')) && fs.existsSync(cardsBin);

test('wasm engine loads cards.bin with the native data hash', { skip: !ready && 'run npm run data:legacy && npm run build first' }, async () => {
  const { initSync, Engine } = await import(path.join(wasmDir, 'decoder.js'));
  initSync({ module: fs.readFileSync(path.join(wasmDir, 'decoder_bg.wasm')) });
  const engine = new Engine(fs.readFileSync(cardsBin), new Uint8Array());
  const native = execFileSync('cargo', ['run', '--release', '-q', '-p', 'decoder-cli', '--', 'bench', cardsBin], { cwd: root, encoding: 'utf8' });
  const hash = native.match(/data hash ([0-9a-f]{16})/)[1];
  assert.equal(engine.dataHash, hash);
  assert.equal(engine.groups, 8210);
  assert.equal(engine.mdGroups, 8023);
});

test('wasm engine rejects corrupted data', { skip: !ready && 'build first' }, async () => {
  const { initSync, Engine } = await import(path.join(wasmDir, 'decoder.js'));
  initSync({ module: fs.readFileSync(path.join(wasmDir, 'decoder_bg.wasm')) });
  const bytes = new Uint8Array(fs.readFileSync(cardsBin));
  bytes[bytes.length - 1] ^= 1;
  assert.throws(() => new Engine(bytes, new Uint8Array()), /hash mismatch/);
});
