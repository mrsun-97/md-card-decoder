// Build a do-nothing libasound.so.2 in .tools/stub-alsa so headless Chromium can start on systems
// without ALSA (e.g. WSL without alsa-lib). Only used by the browser tests; every call fails.
// Preferred alternative: install the real library (Arch: sudo pacman -S alsa-lib).
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { root } from './lib.mjs';

// Newest Playwright headless shell in the browser cache.
const cache = process.env.PLAYWRIGHT_BROWSERS_PATH || path.join(os.homedir(), '.cache', 'ms-playwright');
const shells = fs.readdirSync(cache).filter((d) => d.startsWith('chromium_headless_shell-'))
  .sort((a, b) => Number(b.split('-')[1]) - Number(a.split('-')[1]));
if (!shells.length) throw new Error('no headless shell found; run: npx playwright install chromium-headless-shell');
const exe = path.join(cache, shells[0], 'chrome-headless-shell-linux64', 'chrome-headless-shell');
const out = path.join(root, '.tools', 'stub-alsa');
fs.mkdirSync(out, { recursive: true });
const syms = execFileSync('nm', ['-D', '--undefined-only', exe], { encoding: 'utf8' })
  .split('\n').map((l) => l.trim().split(/\s+/).pop()).filter((s) => s && s.startsWith('snd_'));
const byVersion = new Map();
for (const s of syms) {
  const [name, ver] = s.split('@');
  if (!byVersion.has(ver)) byVersion.set(ver, []);
  byVersion.get(ver).push(name);
}
fs.writeFileSync(path.join(out, 'stub.c'), syms.map((s) => `long ${s.split('@')[0]}(void) { return -19; }`).join('\n') + '\n');
fs.writeFileSync(path.join(out, 'stub.map'), [...byVersion].map(([v, names]) => `${v} { global: ${names.join('; ')}; };`).join('\n') + '\n');
execFileSync('cc', ['-shared', '-fPIC', '-O1', '-o', path.join(out, 'libasound.so.2'), path.join(out, 'stub.c'),
  `-Wl,--version-script=${path.join(out, 'stub.map')}`, '-Wl,-soname,libasound.so.2'], { stdio: 'inherit' });
console.log(`stub with ${syms.length} symbols → ${path.relative(root, out)}/libasound.so.2`);
