// Node test for the nodejs build of breader-wasm. Build first with `node crates/breader-wasm/build.mjs`,
// then run `node crates/breader-wasm/tests/node.mjs`.
//
// The fixture is a QR code (version 3, level M) given as its module matrix, so the test needs
// neither an image decoder nor a binary file: it is drawn here into luma, RGB(A) and YUV buffers.
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const require = createRequire(import.meta.url);
const breader = require(join(here, "..", "pkg-node", "breader.js"));

const TEXT = "https://example.com/breader";
const MODULES = [
  "#######.#.#....##.###.#######",
  "#.....#.#####.##.##.#.#.....#",
  "#.###.#..#..####..#...#.###.#",
  "#.###.#.###.#...##..#.#.###.#",
  "#.###.#..#.#.#...#.#..#.###.#",
  "#.....#...#...#....#..#.....#",
  "#######.#.#.#.#.#.#.#.#######",
  "........#..####..####........",
  "#.##.###..##.##.####..#..#.##",
  "######...##.#..##.###.###...#",
  "..#####.....#.##.##.......##.",
  "..##.#.###...###....#.###...#",
  "...##.##..#.....##.#.....##..",
  "##..#..#..#..#...#.##.#...###",
  "##...##.#.###.#...####.#..###",
  "#..##..#...#.#.#....##..#..#.",
  "#.#...######......####.###.#.",
  "..###....#.###....#.#..#.###.",
  "#.#.###.##..#....##.#...#.#..",
  ".....#..#.#.#..###.#.#.##.#..",
  ".#######.###.#.#....#######..",
  "........####.###.##.#...#####",
  "#######.#.##.#..#.###.#.##.#.",
  "#.....#.##.##...##..#...##.#.",
  "#.###.#..#.#..##....#####.###",
  "#.###.#.####.##.##.#....##.#.",
  "#.###.#.##....###..#...#..#.#",
  "#.....#....#.#..#...#.####.#.",
  "#######.#####..###.##.#....#.",
];

/** Renders the matrix with a 4-module quiet zone, `scale` pixels per module, as luma. */
function render(scale, dark = 20, light = 235) {
  const n = MODULES.length;
  const size = (n + 8) * scale;
  const luma = new Uint8Array(size * size).fill(light);
  for (let y = 0; y < n; y++) {
    for (let x = 0; x < n; x++) {
      if (MODULES[y][x] !== "#") continue;
      for (let dy = 0; dy < scale; dy++) {
        for (let dx = 0; dx < scale; dx++) {
          luma[((y + 4) * scale + dy) * size + (x + 4) * scale + dx] = dark;
        }
      }
    }
  }
  return { luma, size };
}

/** Grey pixels with `channels` equal channels (alpha 255 when there are 4). */
function expand(luma, channels) {
  const out = new Uint8Array(luma.length * channels);
  for (let i = 0; i < luma.length; i++) {
    for (let c = 0; c < channels; c++) out[i * channels + c] = c === 3 ? 255 : luma[i];
  }
  return out;
}

function checkQr(symbols, size, label) {
  assert.equal(symbols.length, 1, `${label}: one symbol`);
  const s = symbols[0];
  assert.equal(s.symbology, "qr", label);
  assert.equal(s.text, TEXT, label);
  assert.ok(s.data instanceof Uint8Array, label);
  assert.equal(new TextDecoder().decode(s.data), TEXT, label);
  assert.equal(s.content, "url", label);
  assert.equal(s.gs1Hri, null, label);
  assert.equal(s.aimId, "]Q1", label);
  assert.equal(s.corners.length, 4, label);
  for (const c of s.corners) {
    assert.ok(c.x >= 0 && c.x <= size && c.y >= 0 && c.y <= size, `${label}: corner inside`);
  }
  assert.ok(s.quality >= 0 && s.quality <= 1, label);
  assert.equal(typeof s.orientation, "number", label);
}

assert.match(breader.version(), /^\d+\.\d+\.\d+/);

// scan: every format
const { luma, size } = render(4);
const reader = new breader.Reader();
checkQr(reader.scan(luma, size, size), size, "luma8 default");
checkQr(reader.scan(luma, size, size, "luma8"), size, "luma8");
checkQr(reader.scan(expand(luma, 3), size, size, "rgb8"), size, "rgb8");
checkQr(reader.scan(expand(luma, 4), size, size, "rgba8"), size, "rgba8");
checkQr(reader.scan(expand(luma, 4), size, size, "bgra8"), size, "bgra8");
const yuv = new Uint8Array(luma.length + (size * size) / 2).fill(128);
yuv.set(luma);
for (const f of ["nv12", "nv21", "i420"]) checkQr(reader.scan(yuv, size, size, f), size, f);

// scanImageData with an ImageData-like object (a Uint8ClampedArray, as a canvas gives)
const rgba = new Uint8ClampedArray(expand(luma, 4));
checkQr(reader.scanImageData({ data: rgba, width: size, height: size }), size, "imageData");

// options
const fast = new breader.Reader({ effort: "fast", symbologies: ["qr"], maxSymbols: 1 });
checkQr(fast.scan(luma, size, size), size, "fast, qr only");
assert.deepEqual(new breader.Reader({ symbologies: ["ean13", "code128"] }).scan(luma, size, size), []);
const inverted = luma.map((v) => 255 - v);
assert.equal(new breader.Reader({ effort: "fast", tryInvert: true }).scan(inverted, size, size).length, 1);
const full = new breader.Reader({
  effort: "exhaustive",
  eanAddon: "optional",
  code39FullAscii: true,
  code39Checksum: false,
});
checkQr(full.scan(luma, size, size), size, "exhaustive");
assert.deepEqual(reader.scan(new Uint8Array(64 * 64).fill(255), 64, 64), []);

// errors are JS Error objects with clear messages
function fails(fn, pattern, label) {
  assert.throws(fn, (e) => e instanceof Error && pattern.test(e.message), label);
}
fails(() => new breader.Reader({ effort: "slow" }), /unknown effort "slow"/, "effort");
fails(() => new breader.Reader({ symbologies: ["qr", "nope"] }), /unknown symbology "nope"/, "symbology");
fails(() => new breader.Reader({ symbologies: [] }), /must not be empty/, "empty symbologies");
fails(() => new breader.Reader({ symbologies: "qr" }), /symbologies must be an array/, "symbologies type");
fails(() => new breader.Reader({ maxSymbols: -1 }), /maxSymbols/, "maxSymbols");
fails(() => new breader.Reader({ tryInvert: "yes" }), /tryInvert must be a boolean/, "tryInvert");
fails(() => new breader.Reader({ eanAddon: "maybe" }), /eanAddon/, "eanAddon");
fails(() => new breader.Reader({ timeBudgetMs: 50 }), /timeBudgetMs is not supported/, "timeBudgetMs");
fails(() => new breader.Reader({ efort: "fast" }), /unknown option "efort"/, "unknown key");
fails(() => new breader.Reader(5), /plain object/, "options type");
fails(() => reader.scan(luma, size, size, "bgr8"), /unknown pixel format "bgr8"/, "format");
fails(() => reader.scan(luma, 0, size), /width must be a positive integer/, "width");
fails(() => reader.scan(new Uint8Array(10), 8, 8), /./, "short buffer");
fails(() => reader.scanImageData({ data: [1, 2], width: 1, height: 1 }), /image\.data/, "bad data");
fails(() => reader.scanImageData({ data: rgba }), /image\.width/, "no width");

// the reader still works after errors
checkQr(reader.scan(luma, size, size), size, "after errors");

// rough timing, informational
const rounds = 20;
const t0 = performance.now();
for (let i = 0; i < rounds; i++) reader.scan(luma, size, size);
const per = (performance.now() - t0) / rounds;
console.log(`ok: ${size}x${size} QR scan ${per.toFixed(1)} ms (balanced, Node ${process.version})`);
