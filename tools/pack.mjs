// Zip public/ into build/md-card-decoder-site.zip for the Cloudflare dashboard upload (or offline
// use: unzip and double-click index.html). A small zip writer, so no external tool is needed.
import fs from 'node:fs';
import path from 'node:path';
import zlib from 'node:zlib';
import { root } from './lib.mjs';

const pub = path.join(root, 'public');
const out = path.join(root, 'build', 'md-card-decoder-site.zip');

const CRC_TABLE = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
function crc32(buf) {
  let c = 0xffffffff;
  for (const b of buf) c = CRC_TABLE[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function files(dir, base = '') {
  return fs.readdirSync(dir, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name)).flatMap((e) => {
    const rel = base ? `${base}/${e.name}` : e.name;
    return e.isDirectory() ? files(path.join(dir, e.name), rel) : [rel];
  });
}

if (!fs.existsSync(path.join(pub, 'index.html'))) {
  console.error('public/ is empty: run npm run build first');
  process.exit(1);
}
const local = [];
const central = [];
let offset = 0;
// Fixed timestamp (1980-01-01) so identical inputs give identical zips.
const DOS_TIME = 0;
const DOS_DATE = (0 << 9) | (1 << 5) | 1;
for (const rel of files(pub)) {
  const data = fs.readFileSync(path.join(pub, rel));
  const name = Buffer.from(rel, "utf8");
  const stored = /\.(webp|png|jpg|zip)$/i.test(rel);
  const body = stored ? data : zlib.deflateRawSync(data, { level: 9 });
  const crc = crc32(data);
  const h = Buffer.alloc(30);
  h.writeUInt32LE(0x04034b50, 0); h.writeUInt16LE(20, 4); h.writeUInt16LE(0x0800, 6);
  h.writeUInt16LE(stored ? 0 : 8, 8); h.writeUInt16LE(DOS_TIME, 10); h.writeUInt16LE(DOS_DATE, 12);
  h.writeUInt32LE(crc, 14); h.writeUInt32LE(body.length, 18); h.writeUInt32LE(data.length, 22);
  h.writeUInt16LE(name.length, 26); h.writeUInt16LE(0, 28);
  local.push(h, name, body);
  const c = Buffer.alloc(46);
  c.writeUInt32LE(0x02014b50, 0); c.writeUInt16LE(20, 4); c.writeUInt16LE(20, 6); c.writeUInt16LE(0x0800, 8);
  c.writeUInt16LE(stored ? 0 : 8, 10); c.writeUInt16LE(DOS_TIME, 12); c.writeUInt16LE(DOS_DATE, 14);
  c.writeUInt32LE(crc, 16); c.writeUInt32LE(body.length, 20); c.writeUInt32LE(data.length, 24);
  c.writeUInt16LE(name.length, 28); c.writeUInt32LE(offset, 42);
  central.push(c, name);
  offset += h.length + name.length + body.length;
}
const cd = Buffer.concat(central);
const end = Buffer.alloc(22);
end.writeUInt32LE(0x06054b50, 0);
end.writeUInt16LE(central.length / 2, 8); end.writeUInt16LE(central.length / 2, 10);
end.writeUInt32LE(cd.length, 12); end.writeUInt32LE(offset, 16);
fs.mkdirSync(path.dirname(out), { recursive: true });
fs.writeFileSync(out, Buffer.concat([...local, cd, end]));
console.log(`${path.relative(root, out)}: ${central.length / 2} files, ${(fs.statSync(out).size / 1024 / 1024).toFixed(2)} MiB`);
