// End-to-end smoke test of the built site (public/) in headless Chromium, over http:// and file://.
// Usage: node tests/e2e/smoke.mjs [screenshot-dir]
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import { launchBrowser, root } from '../../tools/lib.mjs';

const pub = path.join(root, 'public');
const shots = process.argv[2];
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.wasm': 'application/wasm', '.webp': 'image/webp' };
const server = http.createServer((req, res) => {
  let p = decodeURIComponent(new URL(req.url, 'http://x').pathname);
  if (p.endsWith('/')) p += 'index.html';
  const file = path.join(pub, p);
  if (!file.startsWith(pub) || !fs.existsSync(file)) { res.writeHead(404).end(); return; }
  res.writeHead(200, { 'content-type': types[path.extname(file)] || 'application/octet-stream' });
  fs.createReadStream(file).pipe(res);
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const port = server.address().port;

const browser = await launchBrowser();
let failures = 0;
const check = (ok, msg) => { console.log(`${ok ? '  ✓' : '  ✗'} ${msg}`); if (!ok) failures++; };

for (const [label, url] of [['http', `http://127.0.0.1:${port}/`], ['file', `file://${pub}/index.html`]]) {
  console.log(label);
  const page = await browser.newPage({ viewport: { width: 1400, height: 1000 } });
  const errors = [];
  page.on('pageerror', (e) => errors.push(e.message));
  page.on('console', (m) => { if (m.type() === 'error') errors.push(m.text()); });
  await page.goto(url);
  await page.waitForFunction(() => document.querySelector('#dataStats')?.textContent.includes('判定组'), null, { timeout: 30000 });
  check(true, 'boots and loads the card data');
  // Initial reveal: frame = 效果 (2).
  await page.selectOption('#revealField', '0');
  await page.selectOption('#revealValue', '2');
  await page.click('#addRevealBtn');
  await page.waitForFunction(() => !document.querySelector('#proofLabel').hidden, null, { timeout: 30000 });
  const title = await page.textContent('#recommendTitle');
  const count = await page.textContent('#candidateCount');
  check(Number(count.replace(/,/g, '')) > 5000, `initial reveal filters candidates (${count})`);
  check(title && title !== '录入初始揭示', `recommendation shown: ${title} (${await page.textContent('#proofLabel')})`);
  if (shots && label === 'http') await page.screenshot({ path: path.join(shots, 'real-mode.png'), fullPage: true });
  // Test mode: accept the recommended card and let the page judge it, a few times.
  await page.click('#testModeBtn');
  await page.waitForFunction(() => !document.querySelector('#sessionBanner').hidden);
  for (let i = 0; i < 12; i++) {
    const solved = await page.isVisible('#solvedBanner');
    if (solved) break;
    await page.waitForFunction(() => !document.querySelector('#proofLabel').hidden || !document.querySelector('#solvedBanner').hidden, null, { timeout: 30000 });
    const isHint = await page.$eval('#recommendTitle', (el) => Boolean(el.dataset.hint));
    await page.click('#recommendTitle');
    if (!isHint) await page.click('#challengeBtn');
    await page.waitForTimeout(50);
  }
  const steps = await page.$$eval('.trail-step', (els) => els.length);
  check(steps >= 2, `活动轨迹 shows ${steps} steps`);
  check(await page.isVisible('#solvedBanner'), 'test-mode puzzle solved by following recommendations');
  if (shots && label === 'http') await page.screenshot({ path: path.join(shots, 'test-mode.png'), fullPage: true });
  check(errors.length === 0, `no page errors${errors.length ? ': ' + errors.join(' | ') : ''}`);
  await page.close();
}
await browser.close();
server.close();
process.exit(failures ? 1 : 0);
