// Card thumbnails for the site: download each group's representative card image once (zh images
// from the 百鸽 CDN, ygoprodeck small images as fallback), shrink it to a low-quality WebP, and pack
// the thumbnails into sprite sheets so the upload stays far below the 1,000-file limit.
//
//   node tools/images.mjs            download missing thumbnails, then build build/sheets/
//   node tools/images.mjs --sheets   only rebuild the sheets from the thumbnail cache
//
// Cache: build/thumbs/<passcode>.webp (≈10 KB each, gitignored). Images are KONAMI's; the site keeps
// the takedown notice (README).
import fs from 'node:fs';
import path from 'node:path';
import sharp from 'sharp';
import { root } from './lib.mjs';

const W = 180;
const H = 262; // card aspect 59:86
const COLS = 8;
const ROWS = 8;
const QUALITY = 62;
const CONCURRENCY = 6;
const thumbs = path.join(root, 'build', 'thumbs');
const sheetsDir = path.join(root, 'build', 'sheets');
fs.mkdirSync(thumbs, { recursive: true });

// Representative passcodes in group order, read from cards.bin.
function repPasscodes() {
  const b = fs.readFileSync(path.join(root, 'data', 'generated', 'cards.bin'));
  let p = 8;
  const n = b.readUInt32LE(p);
  p += 4 + 4 + 8;
  for (const size of [1, 1, 4, 2, 2, 2]) p += 2 + b.readUInt16LE(p) * size;
  p += 9 * n;
  return Array.from({ length: n }, (_, i) => b.readUInt32LE(p + 4 * i));
}

const sources = (id) => [
  `https://cdn.233.momobako.com/ygoimg/sc/${id}.webp`,
  `https://images.ygoprodeck.com/images/cards_small/${id}.jpg`,
];

async function fetchThumb(id) {
  const file = path.join(thumbs, `${id}.webp`);
  if (fs.existsSync(file) && fs.statSync(file).size > 0) return 'cached';
  for (const url of sources(id)) {
    try {
      const res = await fetch(url, { headers: { 'user-agent': 'md-card-decoder thumbnail builder' }, signal: AbortSignal.timeout(20000) });
      if (!res.ok) continue;
      const buf = Buffer.from(await res.arrayBuffer());
      await sharp(buf).resize(W, H, { fit: 'cover', position: 'top' }).webp({ quality: QUALITY }).toFile(file);
      return 'downloaded';
    } catch {
      // try the next source
    }
  }
  return 'missing';
}

const ids = repPasscodes();
if (!process.argv.includes('--sheets')) {
  const counts = { cached: 0, downloaded: 0, missing: 0 };
  const missing = [];
  let next = 0;
  let done = 0;
  const t0 = Date.now();
  await Promise.all(Array.from({ length: CONCURRENCY }, async () => {
    while (next < ids.length) {
      const id = ids[next++];
      const r = await fetchThumb(id);
      counts[r]++;
      if (r === 'missing') missing.push(id);
      if (++done % 200 === 0 || done === ids.length) {
        process.stdout.write(`\r${done}/${ids.length} · new ${counts.downloaded} · cached ${counts.cached} · missing ${counts.missing} · ${((Date.now() - t0) / 1000).toFixed(0)} s`);
      }
    }
  }));
  console.log(missing.length ? `\nmissing: ${missing.join(' ')}` : '\nall thumbnails present');
}

// Pack into sheets (group order, so the MD pool comes first).
fs.rmSync(sheetsDir, { recursive: true, force: true });
fs.mkdirSync(sheetsDir, { recursive: true });
const per = COLS * ROWS;
const cell = new Array(ids.length).fill(-1);
const present = ids.map((id, g) => [g, path.join(thumbs, `${id}.webp`)]).filter(([, f]) => fs.existsSync(f));
const files = [];
for (let s = 0; s * per < present.length; s++) {
  const chunk = present.slice(s * per, (s + 1) * per);
  const composite = chunk.map(([g, f], i) => {
    cell[g] = s * per + i;
    return { input: f, left: (i % COLS) * W, top: Math.floor(i / COLS) * H };
  });
  const name = `sheet-${String(s).padStart(3, '0')}.webp`;
  await sharp({ create: { width: COLS * W, height: ROWS * H, channels: 3, background: '#101826' } })
    .composite(composite)
    .webp({ quality: QUALITY })
    .toFile(path.join(sheetsDir, name));
  files.push(name);
}
fs.writeFileSync(path.join(sheetsDir, 'sheets.json'), JSON.stringify({ cols: COLS, rows: ROWS, w: W, h: H, files, cell }));
const bytes = files.reduce((sum, f) => sum + fs.statSync(path.join(sheetsDir, f)).size, 0);
console.log(`${files.length} sheets, ${present.length} thumbnails, ${(bytes / 1024 / 1024).toFixed(1)} MiB → build/sheets`);
