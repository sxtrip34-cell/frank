// Draws Frank into the PNG/ICO set Tauri needs. No dependencies: the icons are
// rasterised here and encoded with node:zlib, so the app icon stays "drawn in
// code" like the character itself.
//
//   node scripts/gen-icons.mjs

import { deflateSync } from "node:zlib";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const OUT = join(dirname(fileURLToPath(import.meta.url)), "..", "src-tauri", "icons");

// ── Frank ─────────────────────────────────────────────────────────────────────
// The same look as src/character/orb.ts, shaded per pixel: a ball of cyan
// light, white-hot at the top left, one dark eye with a ring and a glint, a
// thin dark outline so the tray icon reads on a light taskbar, and a halo on
// the larger sizes.

const CYAN = [94, 219, 250]; // ORB_BASE
const WHITE = [255, 255, 255];
const NIGHT = [10, 15, 36];
const INK = [10, 14, 30];
const SS = 4; // supersampling factor

const mix = (a, b, t) => a.map((v, i) => v + (b[i] - v) * t);
const clamp = (v, a, b) => Math.max(a, Math.min(b, v));

/** Piecewise-linear radial gradient of the ball, by distance from the hot spot. */
function ballColor(k) {
  const stops = [
    [0, WHITE],
    [0.3, mix(CYAN, WHITE, 0.7)],
    [0.76, CYAN],
    [1.25, mix(CYAN, NIGHT, 0.42)],
  ];
  for (let i = 1; i < stops.length; i++) {
    const [k1, c1] = stops[i];
    const [k0, c0] = stops[i - 1];
    if (k <= k1) return mix(c0, c1, (k - k0) / (k1 - k0));
  }
  return stops[stops.length - 1][1];
}

/** Colour and coverage of one sample, or null outside everything. */
function sample(px, py, R, withHalo) {
  const d = Math.hypot(px, py);
  const outline = R * 0.06;
  if (d > R + outline) {
    if (!withHalo || d > R * 1.45) return null;
    const a = 0.55 * (1 - (d - R) / (R * 0.45)) ** 2;
    return [CYAN, a];
  }
  if (d > R) return [NIGHT, 0.85];

  let col = ballColor(Math.hypot(px + R * 0.3, py + R * 0.34) / R);
  if (d > R * 0.9) col = mix(col, mix(CYAN, WHITE, 0.55), 0.55); // rim light

  const er = R * 0.36;
  const de = Math.hypot(px, py);
  if (de <= er) {
    col = INK;
    const ring = Math.abs(de - er * 0.6);
    if (ring < er * 0.065) col = mix(CYAN, WHITE, 0.35);
    if (Math.hypot(px + er * 0.32, py + er * 0.34) < er * 0.19) col = WHITE;
  }
  return [col, 1];
}

function renderFrank(size) {
  const px = new Uint8Array(size * size * 4);
  const withHalo = size >= 64;
  // Small icons need every pixel for the ball; big ones leave room for the halo.
  const R = size * (withHalo ? 0.33 : 0.44);
  const c = size / 2;

  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      let r = 0, g = 0, b = 0, a = 0;
      for (let sy = 0; sy < SS; sy++) {
        for (let sx = 0; sx < SS; sx++) {
          const s = sample(x + (sx + 0.5) / SS - c, y + (sy + 0.5) / SS - c, R, withHalo);
          if (!s) continue;
          const [col, alpha] = s;
          r += col[0] * alpha;
          g += col[1] * alpha;
          b += col[2] * alpha;
          a += alpha;
        }
      }
      if (a === 0) continue;
      const o = (y * size + x) * 4;
      px[o] = Math.round(r / a);
      px[o + 1] = Math.round(g / a);
      px[o + 2] = Math.round(b / a);
      px[o + 3] = Math.round(clamp(a / (SS * SS), 0, 1) * 255);
    }
  }
  return px;
}

// ── PNG ───────────────────────────────────────────────────────────────────────

const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();

function crc32(buf) {
  let c = 0xffffffff;
  for (const b of buf) c = CRC_TABLE[(c ^ b) & 0xff] ^ (c >>> 8);
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

function encodePNG(size, rgba) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // RGBA
  const raw = Buffer.alloc(size * (size * 4 + 1));
  for (let y = 0; y < size; y++) {
    raw[y * (size * 4 + 1)] = 0; // filter: none
    Buffer.from(rgba.buffer, y * size * 4, size * 4).copy(raw, y * (size * 4 + 1) + 1);
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

// ── ICO (PNG-in-ICO, Vista and later) ─────────────────────────────────────────

function encodeICO(entries) {
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(entries.length, 4);
  const dir = Buffer.alloc(16 * entries.length);
  let offset = header.length + dir.length;
  entries.forEach((e, i) => {
    const o = i * 16;
    dir[o] = e.size >= 256 ? 0 : e.size;
    dir[o + 1] = e.size >= 256 ? 0 : e.size;
    dir[o + 2] = 0;
    dir[o + 3] = 0;
    dir.writeUInt16LE(1, o + 4);
    dir.writeUInt16LE(32, o + 6);
    dir.writeUInt32LE(e.png.length, o + 8);
    dir.writeUInt32LE(offset, o + 12);
    offset += e.png.length;
  });
  return Buffer.concat([header, dir, ...entries.map((e) => e.png)]);
}

// ── Go ────────────────────────────────────────────────────────────────────────

mkdirSync(OUT, { recursive: true });

const png = (size) => encodePNG(size, renderFrank(size));

const files = {
  "32x32.png": png(32),
  "128x128.png": png(128),
  "128x128@2x.png": png(256),
  "icon.png": png(512),
};
for (const [name, data] of Object.entries(files)) {
  writeFileSync(join(OUT, name), data);
  console.log(`${name} — ${data.length} bytes`);
}

const ico = encodeICO([16, 24, 32, 48, 64, 128, 256].map((size) => ({ size, png: png(size) })));
writeFileSync(join(OUT, "icon.ico"), ico);
console.log(`icon.ico — ${ico.length} bytes`);
