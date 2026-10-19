// Generates a 1024x1024 RGBA PNG app icon without external dependencies.
// Usage: node scripts/generate-icon.mjs
import { deflateSync } from "node:zlib";
import { writeFileSync, mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";

const SIZE = 1024;
const OUT = resolve(process.cwd(), "src-tauri/icons/source.png");

function crc32(buf) {
  let c;
  const table = [];
  for (let n = 0; n < 256; n++) {
    c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c >>> 0;
  }
  let crc = 0xffffffff;
  for (const byte of buf) crc = table[(crc ^ byte) & 0xff] ^ (crc >>> 8);
  return (crc ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length, 0);
  const typeBuf = Buffer.from(type, "ascii");
  const crcBuf = Buffer.alloc(4);
  crcBuf.writeUInt32BE(crc32(Buffer.concat([typeBuf, data])), 0);
  return Buffer.concat([length, typeBuf, data, crcBuf]);
}

const raw = Buffer.alloc(SIZE * (SIZE * 4 + 1));
for (let y = 0; y < SIZE; y++) {
  const rowStart = y * (SIZE * 4 + 1);
  raw[rowStart] = 0; // filter: none
  for (let x = 0; x < SIZE; x++) {
    const i = rowStart + 1 + x * 4;
    // Rounded-square gradient badge with a lighter "server stack" glyph.
    const dx = Math.max(Math.abs(x - SIZE / 2) - SIZE * 0.34, 0);
    const dy = Math.max(Math.abs(y - SIZE / 2) - SIZE * 0.34, 0);
    const corner = Math.hypot(dx, dy);
    const radius = SIZE * 0.16;
    const inside = corner < radius;
    const t = (x + y) / (SIZE * 2);
    const glyph =
      (Math.abs(x - SIZE / 2) < SIZE * 0.24 && Math.abs(y - SIZE * 0.38) < SIZE * 0.05) ||
      (Math.abs(x - SIZE / 2) < SIZE * 0.24 && Math.abs(y - SIZE * 0.5) < SIZE * 0.05) ||
      (Math.abs(x - SIZE / 2) < SIZE * 0.24 && Math.abs(y - SIZE * 0.62) < SIZE * 0.05);
    const r = glyph ? 235 : Math.round(80 + 60 * t);
    const g = glyph ? 240 : Math.round(60 + 30 * t);
    const b = glyph ? 245 : Math.round(200 - 40 * t);
    const alpha = inside ? 255 : 0;
    raw[i] = r;
    raw[i + 1] = g;
    raw[i + 2] = b;
    raw[i + 3] = alpha;
  }
}

const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(SIZE, 0);
ihdr.writeUInt32BE(SIZE, 4);
ihdr[8] = 8; // bit depth
ihdr[9] = 6; // colour type RGBA
ihdr[10] = 0;
ihdr[11] = 0;
ihdr[12] = 0;

const png = Buffer.concat([
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
  chunk("IHDR", ihdr),
  chunk("IDAT", deflateSync(raw, { level: 9 })),
  chunk("IEND", Buffer.alloc(0)),
]);

mkdirSync(dirname(OUT), { recursive: true });
writeFileSync(OUT, png);
console.log(`wrote ${OUT} (${png.length} bytes)`);
