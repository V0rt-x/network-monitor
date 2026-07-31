// Generates the source PNG the Tauri icon pipeline consumes:
//   node scripts/make-icon.mjs && npx tauri icon scripts/icon-source.png
// Hand-rolled so the repo needs no image toolchain; the generated icons are committed.
import { deflateSync } from 'node:zlib';
import { writeFileSync } from 'node:fs';
import { Buffer } from 'node:buffer';

const SIZE = 1024;
const BACKGROUND = [15, 23, 42, 255]; // slate-900
const BAR = [56, 189, 248, 255]; // sky-400
const BAR_DIM = [30, 64, 118, 255];

/** Four "signal strength" bars, three lit and one dim — the app measures a link's health. */
const BARS = [
  { x: 224, w: 96, h: 192, lit: true },
  { x: 384, w: 96, h: 352, lit: true },
  { x: 544, w: 96, h: 512, lit: true },
  { x: 704, w: 96, h: 672, lit: false },
];

const BASELINE = 800;
const RADIUS = 160; // corner radius of the rounded-square background

const pixels = Buffer.alloc(SIZE * SIZE * 4);

const inRoundedSquare = (x, y) => {
  const cx = Math.min(Math.max(x, RADIUS), SIZE - 1 - RADIUS);
  const cy = Math.min(Math.max(y, RADIUS), SIZE - 1 - RADIUS);
  const dx = x - cx;
  const dy = y - cy;
  return dx * dx + dy * dy <= RADIUS * RADIUS;
};

const colorAt = (x, y) => {
  if (!inRoundedSquare(x, y)) return [0, 0, 0, 0];
  for (const bar of BARS) {
    if (x >= bar.x && x < bar.x + bar.w && y <= BASELINE && y > BASELINE - bar.h) {
      return bar.lit ? BAR : BAR_DIM;
    }
  }
  return BACKGROUND;
};

for (let y = 0; y < SIZE; y += 1) {
  for (let x = 0; x < SIZE; x += 1) {
    const [r, g, b, a] = colorAt(x, y);
    const i = (y * SIZE + x) * 4;
    pixels[i] = r;
    pixels[i + 1] = g;
    pixels[i + 2] = b;
    pixels[i + 3] = a;
  }
}

// PNG scanlines are prefixed with a filter byte (0 = none).
const raw = Buffer.alloc(SIZE * (SIZE * 4 + 1));
for (let y = 0; y < SIZE; y += 1) {
  raw[y * (SIZE * 4 + 1)] = 0;
  pixels.copy(raw, y * (SIZE * 4 + 1) + 1, y * SIZE * 4, (y + 1) * SIZE * 4);
}

const crcTable = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
const crc32 = (buf) => {
  let c = 0xffffffff;
  for (const byte of buf) c = crcTable[(c ^ byte) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
};

const chunk = (type, data) => {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, 'ascii'), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([length, body, crc]);
};

const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(SIZE, 0);
ihdr.writeUInt32BE(SIZE, 4);
ihdr[8] = 8; // bit depth
ihdr[9] = 6; // colour type: RGBA

const png = Buffer.concat([
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
  chunk('IHDR', ihdr),
  chunk('IDAT', deflateSync(raw, { level: 9 })),
  chunk('IEND', Buffer.alloc(0)),
]);

writeFileSync(new URL('./icon-source.png', import.meta.url), png);
console.log(`wrote scripts/icon-source.png (${SIZE}x${SIZE})`);
