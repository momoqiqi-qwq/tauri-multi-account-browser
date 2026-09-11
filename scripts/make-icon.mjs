// Generates a 1024x1024 app-icon source PNG with zero dependencies (pure Node zlib).
// Output: <project>/src-tauri/app-icon.png  (feed to `npx tauri icon`)
import { deflateSync } from "node:zlib";
import { writeFileSync, mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const SIZE = 1024;
const out = resolve(__dirname, "../src-tauri/app-icon.png");

// ---------- tiny PNG encoder (8-bit RGBA) ----------
const CRC_TABLE = new Int32Array(256);
for (let n = 0; n < 256; n++) {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  CRC_TABLE[n] = c;
}
function crc32(buf) {
  let c = 0xffffffff;
  for (let i = 0; i < buf.length; i++) c = CRC_TABLE[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}
function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}
function encodePng(rgba) {
  const raw = Buffer.alloc((SIZE * 4 + 1) * SIZE);
  let p = 0;
  for (let y = 0; y < SIZE; y++) {
    raw[p++] = 0; // filter: none
    for (let x = 0; x < SIZE * 4; x++) raw[p++] = rgba[y * SIZE * 4 + x];
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(SIZE, 0);
  ihdr.writeUInt32BE(SIZE, 4);
  ihdr[8] = 8;  // bit depth
  ihdr[9] = 6;  // color type RGBA
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

// ---------- draw ----------
const rgba = new Uint8Array(SIZE * SIZE * 4);
const lerp = (a, b, t) => a + (b - a) * t;
// diagonal gradient bg: indigo-400 -> indigo-950
const c1 = [129, 140, 248], c2 = [30, 27, 75];
function inRoundRect(x, y, rx, ry, w, h, rad) {
  if (x < rx || x > rx + w || y < ry || y > ry + h) return false;
  const dx = Math.max(rx + rad - x, 0, x - (rx + w - rad));
  const dy = Math.max(ry + rad - y, 0, y - (ry + h - rad));
  return dx * dx + dy * dy <= rad * rad;
}
// account "cards" stacked diagonally (white rounded rects)
const cards = [
  { cx: SIZE / 2 + 130, cy: SIZE / 2 + 130, w: 560, h: 330 },
  { cx: SIZE / 2, cy: SIZE / 2, w: 560, h: 330 },
  { cx: SIZE / 2 - 130, cy: SIZE / 2 - 130, w: 560, h: 330 },
];
for (let y = 0; y < SIZE; y++) {
  for (let x = 0; x < SIZE; x++) {
    const t = (x + y) / (2 * SIZE);
    let r = lerp(c1[0], c2[0], t), g = lerp(c1[1], c2[1], t), b = lerp(c1[2], c2[2], t);
    let fill = false;
    for (const c of cards) {
      if (inRoundRect(x, y, c.cx - c.w / 2, c.cy - c.h / 2, c.w, c.h, 40)) fill = true;
    }
    if (fill) { r = 255; g = 255; b = 255; }
    const i = (y * SIZE + x) * 4;
    rgba[i] = r; rgba[i + 1] = g; rgba[i + 2] = b; rgba[i + 3] = 255;
  }
}

mkdirSync(dirname(out), { recursive: true });
writeFileSync(out, encodePng(rgba));
console.log("wrote", out);
