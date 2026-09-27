// M1 spike: load the engine in a Worker from http:// and from file:// in headless Chromium.
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import { launchBrowser, root } from './lib.mjs';

const pub = path.join(root, 'public');
const types = { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm' };
const server = http.createServer((req, res) => {
  const file = path.join(pub, decodeURIComponent(new URL(req.url, 'http://x').pathname));
  if (!file.startsWith(pub) || !fs.existsSync(file)) { res.writeHead(404).end(); return; }
  res.writeHead(200, { 'content-type': types[path.extname(file)] || 'application/octet-stream' });
  fs.createReadStream(file).pipe(res);
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const port = server.address().port;

const browser = await launchBrowser();
let failed = false;
for (const url of [`http://127.0.0.1:${port}/spike.html`, `file://${pub}/spike.html`]) {
  const page = await browser.newPage();
  page.on('console', (m) => m.type() === 'error' && console.log('  console:', m.text()));
  await page.goto(url);
  const result = await page.waitForFunction(() => window.__result, null, { timeout: 30000 }).then((h) => h.jsonValue());
  console.log(url.split(':')[0].padEnd(5), JSON.stringify(result));
  failed ||= !result.ok;
  await page.close();
}
await browser.close();
server.close();
process.exit(failed ? 1 : 0);
