// Minimal static server for previewing public/ locally (npm run dev → http://localhost:8787).
// Under WSL2 the Windows browser can open the same address.
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import { root } from './lib.mjs';

const pub = path.join(root, 'public');
const port = Number(process.env.PORT || 8787);
const types = {
  '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8',
  '.wasm': 'application/wasm', '.webp': 'image/webp', '.json': 'application/json', '.svg': 'image/svg+xml',
};

export function createServer(dir = pub) {
  return http.createServer((req, res) => {
    let p = decodeURIComponent(new URL(req.url, 'http://x').pathname);
    if (p.endsWith('/')) p += 'index.html';
    const file = path.join(dir, p);
    if (!file.startsWith(dir) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) {
      const notFound = path.join(dir, '404.html');
      res.writeHead(404, { 'content-type': types['.html'] });
      res.end(fs.existsSync(notFound) ? fs.readFileSync(notFound) : 'not found');
      return;
    }
    res.writeHead(200, { 'content-type': types[path.extname(file)] || 'application/octet-stream', 'cache-control': 'no-cache' });
    fs.createReadStream(file).pipe(res);
  });
}

if (import.meta.url === `file://${process.argv[1]}`) {
  if (!fs.existsSync(path.join(pub, 'index.html'))) {
    console.error('public/ is empty: run npm run build first');
    process.exit(1);
  }
  createServer().listen(port, () => console.log(`serving public/ at http://localhost:${port}`));
}
