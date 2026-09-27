// Download the upstream card sources into data/raw/ and record them in data/sources.lock.json.
//
//   node tools/data-fetch.mjs            re-download exactly what the lock file pins (verifies sha256)
//   node tools/data-fetch.mjs --update   take the newest ygocdb dump + YgoMaster commit, rewrite the lock
//
// Sources: ygocdb (百鸽) full card dump — fields and zh/en/ja names; YgoMaster CardList.json +
// YdkIds.txt — which passcodes exist in Master Duel.
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import zlib from 'node:zlib';
import { root } from './lib.mjs';

const rawDir = path.join(root, 'data', 'raw');
const lockFile = path.join(root, 'data', 'sources.lock.json');
const update = process.argv.includes('--update');
const sha256 = (b) => crypto.createHash('sha256').update(b).digest('hex');

async function get(url, as = 'buffer') {
  const res = await fetch(url, { headers: { 'user-agent': 'md-card-decoder data pipeline' } });
  if (!res.ok) throw new Error(`HTTP ${res.status} for ${url}`);
  return as === 'json' ? res.json() : as === 'text' ? res.text() : Buffer.from(await res.arrayBuffer());
}

/** Extract the single `cards.json` entry from a zip archive (stored or deflated). */
function unzipCardsJson(zip) {
  let p = 0;
  while (p + 30 <= zip.length && zip.readUInt32LE(p) === 0x04034b50) {
    const method = zip.readUInt16LE(p + 8);
    const flags = zip.readUInt16LE(p + 6);
    let compSize = zip.readUInt32LE(p + 18);
    const nameLen = zip.readUInt16LE(p + 26);
    const extraLen = zip.readUInt16LE(p + 28);
    const name = zip.toString('utf8', p + 30, p + 30 + nameLen);
    const start = p + 30 + nameLen + extraLen;
    if (flags & 8) compSize = findCentralSize(zip, name);
    const data = zip.subarray(start, start + compSize);
    if (name.endsWith('.json')) return method === 0 ? data : zlib.inflateRawSync(data);
    p = start + compSize;
  }
  throw new Error('cards.json not found in zip');
}
function findCentralSize(zip, wanted) {
  for (let p = zip.length - 22; p >= 0; p--) {
    if (zip.readUInt32LE(p) !== 0x06054b50) continue;
    let q = zip.readUInt32LE(p + 16);
    const n = zip.readUInt16LE(p + 10);
    for (let i = 0; i < n; i++) {
      const size = zip.readUInt32LE(q + 20);
      const nameLen = zip.readUInt16LE(q + 28);
      const name = zip.toString('utf8', q + 46, q + 46 + nameLen);
      if (name === wanted) return size;
      q += 46 + nameLen + zip.readUInt16LE(q + 30) + zip.readUInt16LE(q + 32);
    }
  }
  throw new Error(`no central directory entry for ${wanted}`);
}

const lock = fs.existsSync(lockFile) ? JSON.parse(fs.readFileSync(lockFile, 'utf8')) : null;
if (!update && !lock) {
  console.error('data/sources.lock.json missing; run with --update to create it');
  process.exit(1);
}

let commit = lock?.ygomaster?.commit;
if (update) {
  const info = await get('https://api.github.com/repos/pixeltris/YgoMaster/commits/master', 'json');
  commit = info.sha;
  console.log(`YgoMaster master = ${commit} (${info.commit.committer.date})`);
}
const ygoBase = `https://raw.githubusercontent.com/pixeltris/YgoMaster/${commit}/YgoMaster/Data`;
const zip = await get('https://ygocdb.com/api/v0/cards.zip');
// The published md5 (a quoted string) is that of the unzipped cards.json.
const md5 = (await get('https://ygocdb.com/api/v0/cards.zip.md5', 'text')).trim().replace(/^"|"$/g, '');
const cards = unzipCardsJson(zip);
const actualMd5 = crypto.createHash('md5').update(cards).digest('hex');
if (md5 !== actualMd5) throw new Error(`cards.json md5 ${actualMd5} ≠ published ${md5}`);
const cardList = await get(`${ygoBase}/CardList.json`);
const ydkIds = await get(`${ygoBase}/YdkIds.txt`);

const files = { 'ygocdb-cards.json': cards, 'CardList.json': cardList, 'YdkIds.txt': ydkIds };
if (!update) {
  // The ygocdb dump has no versioned URL; a changed hash means upstream moved on.
  for (const [name, buf] of Object.entries(files)) {
    if (lock.files[name] !== sha256(buf)) {
      console.error(`${name} differs from the lock (${sha256(buf).slice(0, 12)} ≠ ${lock.files[name].slice(0, 12)}).`);
      console.error('Upstream changed. Run `node tools/data-fetch.mjs --update` to accept the new data.');
      process.exit(1);
    }
  }
}
fs.mkdirSync(rawDir, { recursive: true });
for (const [name, buf] of Object.entries(files)) fs.writeFileSync(path.join(rawDir, name), buf);
if (update) {
  const newLock = {
    note: 'Pinned upstream sources for `mdcd data build`. Update with: node tools/data-fetch.mjs --update',
    fetchedAt: new Date().toISOString().slice(0, 10),
    ygocdb: { url: 'https://ygocdb.com/api/v0/cards.zip', cardsJsonMd5: actualMd5 },
    ygomaster: { repo: 'pixeltris/YgoMaster', commit },
    files: Object.fromEntries(Object.entries(files).map(([n, b]) => [n, sha256(b)])),
  };
  fs.writeFileSync(lockFile, JSON.stringify(newLock, null, 2) + '\n');
}
for (const [name, buf] of Object.entries(files)) console.log(`${name.padEnd(18)} ${(buf.length / 1024).toFixed(0).padStart(6)} KiB`);
