// 生成 Tuneforge 应用图标（纯 Node，无第三方依赖）。
//
//   node scripts/generate-icons.mjs
//
// 输出到 app/src-tauri/icons/：
//   32x32.png / 128x128.png / 128x128@2x.png / icon.png / icon.ico
//
// 设计：深色圆角方块 + 等化器柱状条（青→绿渐变），小尺寸下仍可辨识。

import { deflateSync } from "node:zlib";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const OUT_DIR = join(ROOT, "app", "src-tauri", "icons");

const SIZE = 256;

/** 圆角矩形覆盖度（0..1 抗锯齿）。 */
function roundedCoverage(x, y, w, h, radius, cx, cy) {
  // 以 (cx, cy) 为中心的圆角矩形
  const dx = Math.abs(x - cx) - (w / 2 - radius);
  const dy = Math.abs(y - cy) - (h / 2 - radius);
  const ax = Math.max(dx, 0);
  const ay = Math.max(dy, 0);
  const outside = Math.hypot(ax, ay);
  const inside = Math.min(Math.max(dx, dy), 0);
  const d = outside + inside - radius;
  return Math.min(Math.max(0.5 - d, 0), 1);
}

function mix(a, b, t) {
  return [
    Math.round(a[0] + (b[0] - a[0]) * t),
    Math.round(a[1] + (b[1] - a[1]) * t),
    Math.round(a[2] + (b[2] - a[2]) * t),
  ];
}

/** 渲染 256x256 RGBA（非预乘）。 */
function render() {
  const px = new Uint8ClampedArray(SIZE * SIZE * 4);
  const bgTop = [30, 41, 59]; // slate-800
  const bgBottom = [15, 23, 42]; // slate-900
  const barLow = [34, 211, 238]; // cyan-400
  const barHigh = [74, 222, 128]; // green-400

  // 5 根柱子：x 中心、半宽、高度比例
  const bars = [
    { cx: 60, half: 13, h: 0.32 },
    { cx: 94, half: 13, h: 0.62 },
    { cx: 128, half: 13, h: 0.92 },
    { cx: 162, half: 13, h: 0.52 },
    { cx: 196, half: 13, h: 0.74 },
  ];

  for (let y = 0; y < SIZE; y++) {
    for (let x = 0; x < SIZE; x++) {
      const i = (y * SIZE + x) * 4;
      const xc = x + 0.5;
      const yc = y + 0.5;

      // 背景圆角方块
      const bgA = roundedCoverage(xc, yc, SIZE, SIZE, 56, SIZE / 2, SIZE / 2);

      let r = 0;
      let g = 0;
      let b = 0;

      if (bgA > 0) {
        const t = yc / SIZE;
        const [br, bg, bb] = mix(bgTop, bgBottom, t);
        r = br;
        g = bg;
        b = bb;

        // 柱子
        const barBase = SIZE - 44; // 底部基线
        let barAlpha = 0;
        let barColor = [0, 0, 0];
        for (const bar of bars) {
          const h = 176 * bar.h;
          const top = barBase - h;
          const cov = roundedCoverage(xc, yc, bar.half * 2, h, bar.half, bar.cx, (top + barBase) / 2);
          if (cov > 0) {
            const t2 = (yc - top) / h;
            barColor = mix(barLow, barHigh, Math.min(Math.max(t2, 0), 1));
            barAlpha = Math.max(barAlpha, cov);
          }
        }
        if (barAlpha > 0) {
          r = Math.round(r + (barColor[0] - r) * barAlpha);
          g = Math.round(g + (barColor[1] - g) * barAlpha);
          b = Math.round(b + (barColor[2] - b) * barAlpha);
        }
      }

      px[i] = r;
      px[i + 1] = g;
      px[i + 2] = b;
      px[i + 3] = Math.round(bgA * 255);
    }
  }
  return px;
}

/** 盒式降采样（RGBA，含 alpha 加权）。 */
function resize(src, srcSize, dstSize) {
  const out = new Uint8ClampedArray(dstSize * dstSize * 4);
  const scale = srcSize / dstSize;
  for (let y = 0; y < dstSize; y++) {
    for (let x = 0; x < dstSize; x++) {
      let r = 0;
      let g = 0;
      let b = 0;
      let a = 0;
      let n = 0;
      const y0 = Math.floor(y * scale);
      const y1 = Math.max(y0 + 1, Math.floor((y + 1) * scale));
      const x0 = Math.floor(x * scale);
      const x1 = Math.max(x0 + 1, Math.floor((x + 1) * scale));
      for (let sy = y0; sy < y1; sy++) {
        for (let sx = x0; sx < x1; sx++) {
          const i = (sy * srcSize + sx) * 4;
          const av = src[i + 3] / 255;
          r += src[i] * av;
          g += src[i + 1] * av;
          b += src[i + 2] * av;
          a += av;
          n++;
        }
      }
      const o = (y * dstSize + x) * 4;
      if (a > 0) {
        out[o] = Math.round(r / a);
        out[o + 1] = Math.round(g / a);
        out[o + 2] = Math.round(b / a);
      }
      out[o + 3] = Math.round((a / n) * 255);
    }
  }
  return out;
}

// ----------------------------------------------------------------- PNG

const CRC_TABLE = (() => {
  const table = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c >>> 0;
  }
  return table;
})();

function crc32(buf) {
  let c = 0xffffffff;
  for (const byte of buf) c = CRC_TABLE[(c ^ byte) & 0xff] ^ (c >>> 8);
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

function encodePng(rgba, size) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // RGBA
  const raw = Buffer.alloc(size * (size * 4 + 1));
  for (let y = 0; y < size; y++) {
    raw[y * (size * 4 + 1)] = 0; // filter: none
    Buffer.from(rgba.buffer, rgba.byteOffset + y * size * 4, size * 4).copy(
      raw,
      y * (size * 4 + 1) + 1,
    );
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

// ----------------------------------------------------------------- ICO

function encodeIcoEntry(rgba, size) {
  const header = Buffer.alloc(40);
  header.writeUInt32LE(40, 0);
  header.writeInt32LE(size, 4);
  header.writeInt32LE(size * 2, 8); // XOR + AND 掩码
  header.writeUInt16LE(1, 12);
  header.writeUInt16LE(32, 14);
  header.writeUInt32LE(size * size * 4, 20);

  const pixels = Buffer.alloc(size * size * 4);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const src = ((size - 1 - y) * size + x) * 4; // BMP 自下而上
      const dst = (y * size + x) * 4;
      pixels[dst] = rgba[src + 2]; // B
      pixels[dst + 1] = rgba[src + 1]; // G
      pixels[dst + 2] = rgba[src]; // R
      pixels[dst + 3] = rgba[src + 3]; // A
    }
  }
  const maskRow = Math.ceil(size / 32) * 4;
  return Buffer.concat([header, pixels, Buffer.alloc(maskRow * size)]);
}

function encodeIco(images) {
  const dir = Buffer.alloc(6);
  dir.writeUInt16LE(0, 0);
  dir.writeUInt16LE(1, 2);
  dir.writeUInt16LE(images.length, 4);
  const entries = [];
  let offset = 6 + 16 * images.length;
  const blobs = [];
  for (const { size, blob } of images) {
    const e = Buffer.alloc(16);
    e[0] = size >= 256 ? 0 : size;
    e[1] = size >= 256 ? 0 : size;
    e.writeUInt16LE(1, 4);
    e.writeUInt16LE(32, 6);
    e.writeUInt32LE(blob.length, 8);
    e.writeUInt32LE(offset, 12);
    entries.push(e);
    blobs.push(blob);
    offset += blob.length;
  }
  return Buffer.concat([dir, ...entries, ...blobs]);
}

// ----------------------------------------------------------------- main

const base = render();
mkdirSync(OUT_DIR, { recursive: true });

const sizes = [16, 32, 48, 64, 128, 256];
const rendered = new Map(sizes.map((s) => [s, s === SIZE ? base : resize(base, SIZE, s)]));

for (const [size, file] of [
  [32, "32x32.png"],
  [128, "128x128.png"],
  [256, "128x128@2x.png"],
  [256, "icon.png"],
]) {
  writeFileSync(join(OUT_DIR, file), encodePng(rendered.get(size), size));
}

writeFileSync(
  join(OUT_DIR, "icon.ico"),
  encodeIco(sizes.map((s) => ({ size: s, blob: encodeIcoEntry(rendered.get(s), s) }))),
);

console.log(`图标已生成：${OUT_DIR}`);
