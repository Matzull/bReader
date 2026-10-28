// Demo: scan an image file or the camera with breader (plain ES module, no bundler).
// Build the package (`node crates/breader-wasm/build.mjs web`), serve crates/breader-wasm over
// HTTP (e.g. `python3 -m http.server -d crates/breader-wasm`) and open /demo/.
import init, { Reader, version } from "../pkg/breader.js";

const $ = (id) => document.getElementById(id);
const canvas = $("canvas");
const ctx = canvas.getContext("2d", { willReadFrequently: true });
const video = $("video");
const status = $("status");
const results = $("results");

let reader;
let stream = null;
let busy = false;

function makeReader() {
  reader?.free();
  reader = new Reader({ effort: $("effort").value });
}

function show(symbols, ms) {
  status.textContent = `${symbols.length} symbol(s) in ${ms.toFixed(1)} ms (breader ${version()})`;
  results.replaceChildren();
  ctx.lineWidth = 3;
  ctx.strokeStyle = "#0c0";
  for (const s of symbols) {
    const li = document.createElement("li");
    const text = s.text ?? `<${s.data.length} bytes>`;
    li.textContent = `${s.symbology} [${s.content}]: ${s.gs1Hri ?? text}`;
    results.append(li);
    ctx.beginPath();
    s.corners.forEach((c, i) => (i ? ctx.lineTo(c.x, c.y) : ctx.moveTo(c.x, c.y)));
    ctx.closePath();
    ctx.stroke();
  }
}

/** Scans what is on the canvas. */
function scanCanvas() {
  const image = ctx.getImageData(0, 0, canvas.width, canvas.height);
  const t0 = performance.now();
  const symbols = reader.scanImageData(image);
  show(symbols, performance.now() - t0);
}

$("file").addEventListener("change", async (event) => {
  const file = event.target.files[0];
  if (!file) return;
  stopCamera();
  const bitmap = await createImageBitmap(file);
  canvas.width = bitmap.width;
  canvas.height = bitmap.height;
  ctx.drawImage(bitmap, 0, 0);
  bitmap.close();
  scanCanvas();
});

function stopCamera() {
  stream?.getTracks().forEach((t) => t.stop());
  stream = null;
  $("camera").textContent = "Start camera";
}

async function loop() {
  if (!stream) return;
  if (!busy && video.readyState >= 2) {
    busy = true;
    canvas.width = video.videoWidth;
    canvas.height = video.videoHeight;
    ctx.drawImage(video, 0, 0);
    scanCanvas();
    busy = false;
  }
  requestAnimationFrame(loop);
}

$("camera").addEventListener("click", async () => {
  if (stream) return stopCamera();
  try {
    stream = await navigator.mediaDevices.getUserMedia({ video: { facingMode: "environment" } });
  } catch (e) {
    status.textContent = `Camera unavailable: ${e.message}`;
    return;
  }
  video.srcObject = stream;
  await video.play();
  $("camera").textContent = "Stop camera";
  requestAnimationFrame(loop);
});

$("effort").addEventListener("change", makeReader);

await init();
makeReader();
status.textContent = `Ready (breader ${version()}). Choose an image or start the camera.`;
