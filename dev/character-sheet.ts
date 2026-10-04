// Development page: Frank in every state and emote, side by side, drawn by the
// real engine. `npx vite`, then open /dev/character-sheet.html.

import { BotEngine, hexToRGB } from "../src/character/engine";
import type { BotEmoteName, BotStateName } from "../src/core/layout";

const STATES: BotStateName[] = [
  "idle", "working", "thinking", "searching", "approval", "question",
  "error", "finished", "ratelimit", "sleeping", "dizzy",
];
const EMOTES: BotEmoteName[] = ["love", "surprised", "proud", "wink", "yawn", "happy", "annoyed"];

const SIZE = 150;
const dpr = Math.min(2, window.devicePixelRatio || 1);
const grid = document.getElementById("grid")!;
const cells: { engine: BotEngine; ctx: CanvasRenderingContext2D }[] = [];

function cell(label: string, setup: (e: BotEngine) => void) {
  const canvas = document.createElement("canvas");
  canvas.width = SIZE * dpr;
  canvas.height = SIZE * dpr;
  canvas.style.width = `${SIZE}px`;
  canvas.style.height = `${SIZE}px`;
  const fig = document.createElement("figure");
  const cap = document.createElement("figcaption");
  cap.textContent = label;
  fig.append(canvas, cap);
  grid.append(fig);
  const engine = new BotEngine();
  setup(engine);
  cells.push({ engine, ctx: canvas.getContext("2d")! });
}

for (const s of STATES) cell(s, (e) => e.setState(s, true));
for (const m of EMOTES) cell(`emote: ${m}`, (e) => e.setPermanentEmote(m));
cell("pill (mini)", (e) => {
  e.isMini = true;
  e.bodyColor = hexToRGB("#7C5CFF");
});
cell("looking left", (e) => {
  e.lookX = -1;
  e.lookY = -0.3;
});

// A timer rather than requestAnimationFrame: a headless browser taking a
// screenshot of this page runs timers, but barely any animation frames.
let last = performance.now();
function frame() {
  const nowMs = performance.now();
  const dt = Math.min(0.05, (nowMs - last) / 1000);
  last = nowMs;
  for (const { engine, ctx } of cells) {
    engine.update(dt);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, SIZE, SIZE);
    engine.draw(ctx, SIZE, SIZE);
  }
}
setInterval(frame, 16);
