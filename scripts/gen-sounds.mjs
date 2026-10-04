// Synthesizes Frank's 28 sounds into sounds/*.wav — no samples, no
// dependencies: sine partials, envelopes and glides, written as 16-bit mono
// WAV. A ball of light should sound like one: glassy, bell-like, soft.
//
//   node scripts/gen-sounds.mjs

import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const OUT = join(dirname(fileURLToPath(import.meta.url)), "..", "sounds");
const RATE = 44100;
const TAU = Math.PI * 2;

/** Note name → frequency: "A4" = 440 Hz. */
function hz(name) {
  const m = /^([A-G])(#?)(\d)$/.exec(name);
  const semis = { C: -9, D: -7, E: -5, F: -4, G: -2, A: 0, B: 2 }[m[1]] + (m[2] ? 1 : 0);
  return 440 * 2 ** ((semis + (Number(m[3]) - 4) * 12) / 12);
}

/**
 * One voice. `f0`→`f1` glides over the voice's length (exponentially); the
 * envelope is a short attack then an exponential decay; `partials` are
 * [ratio, level] pairs on top of the fundamental, for a bell-like timbre.
 */
function voice(buf, {
  at = 0, dur, f0, f1 = f0, gain = 0.5, attack = 0.004, decay = 6,
  partials = [[1, 1], [2.01, 0.25], [3.02, 0.08]], vib = 0, vibRate = 6, tremolo = 0,
}) {
  const start = Math.floor(at * RATE);
  const n = Math.floor(dur * RATE);
  const phases = partials.map(() => 0);
  for (let i = 0; i < n && start + i < buf.length; i++) {
    const t = i / RATE;
    const p = i / n;
    const f = f0 * (f1 / f0) ** p * (1 + vib * Math.sin(TAU * vibRate * t));
    const env = Math.min(1, t / attack) * Math.exp(-decay * t) * (1 - p) ** 0.5;
    const trem = 1 - tremolo * (0.5 + 0.5 * Math.sin(TAU * 7 * t));
    let s = 0;
    partials.forEach(([ratio, level], k) => {
      phases[k] += (TAU * f * ratio) / RATE;
      s += Math.sin(phases[k]) * level;
    });
    buf[start + i] += s * env * trem * gain;
  }
}

/** Soft filtered noise, for whooshes and thumps. */
function noise(buf, { at = 0, dur, gain = 0.2, attack = 0.01, decay = 10, smooth = 0.9 }) {
  const start = Math.floor(at * RATE);
  const n = Math.floor(dur * RATE);
  let lp = 0;
  let seed = 12345;
  for (let i = 0; i < n && start + i < buf.length; i++) {
    seed = (seed * 1103515245 + 12345) & 0x7fffffff;
    const white = seed / 0x3fffffff - 1;
    lp = lp * smooth + white * (1 - smooth);
    const t = i / RATE;
    const env = Math.min(1, t / attack) * Math.exp(-decay * t) * (1 - i / n);
    buf[start + i] += lp * env * gain * 4;
  }
}

const BELL = [[1, 1], [2.76, 0.22], [5.4, 0.06]];
const SOFT = [[1, 1], [2, 0.12]];
const PURE = [[1, 1]];

/** Every sound: its length, and what plays in it. */
const SOUNDS = {
  peek: [0.32, (b) => { voice(b, { dur: 0.18, f0: hz("E6"), gain: 0.35, partials: BELL }); voice(b, { at: 0.09, dur: 0.22, f0: hz("A6"), gain: 0.3, partials: BELL }); }],
  greet: [0.62, (b) => ["C6", "E6", "G6", "C7"].forEach((n, i) => voice(b, { at: i * 0.09, dur: 0.4, f0: hz(n), gain: 0.28, decay: 7, partials: BELL }))],
  open: [0.28, (b) => { voice(b, { dur: 0.26, f0: hz("A4"), f1: hz("E6"), gain: 0.3, decay: 5, partials: SOFT }); noise(b, { dur: 0.22, gain: 0.08, decay: 8 }); }],
  close: [0.26, (b) => { voice(b, { dur: 0.24, f0: hz("E6"), f1: hz("A4"), gain: 0.28, decay: 6, partials: SOFT }); noise(b, { dur: 0.2, gain: 0.06, decay: 10 }); }],
  hover: [0.06, (b) => voice(b, { dur: 0.05, f0: hz("C7"), gain: 0.18, decay: 40, partials: PURE })],
  blip: [0.09, (b) => voice(b, { dur: 0.08, f0: 1180, gain: 0.3, decay: 30, partials: SOFT })],
  tick: [0.04, (b) => voice(b, { dur: 0.03, f0: 2400, gain: 0.22, decay: 80, partials: PURE })],
  slap: [0.16, (b) => { voice(b, { dur: 0.14, f0: 210, f1: 120, gain: 0.55, decay: 18, partials: SOFT }); noise(b, { dur: 0.06, gain: 0.35, decay: 50, smooth: 0.5 }); }],
  annoyed: [0.4, (b) => { voice(b, { dur: 0.16, f0: hz("D5"), gain: 0.3, vib: 0.02, partials: SOFT }); voice(b, { at: 0.15, dur: 0.24, f0: hz("A4"), f1: hz("G4"), gain: 0.3, vib: 0.03, partials: SOFT }); }],
  dizzy: [0.75, (b) => voice(b, { dur: 0.72, f0: hz("A5"), f1: hz("A4"), gain: 0.3, decay: 2.5, vib: 0.08, vibRate: 9, partials: SOFT })],
  work: [0.2, (b) => voice(b, { dur: 0.18, f0: hz("G4"), gain: 0.25, decay: 12, partials: SOFT })],
  think: [0.36, (b) => voice(b, { dur: 0.34, f0: hz("E5"), gain: 0.22, decay: 5, tremolo: 0.5, partials: SOFT })],
  search: [0.48, (b) => [0, 1, 2, 3].forEach((i) => voice(b, { at: i * 0.1, dur: 0.12, f0: hz(i % 2 ? "B6" : "G6"), gain: 0.16, decay: 20, partials: BELL }))],
  finish: [0.75, (b) => { ["G5", "C6", "E6", "G6"].forEach((n, i) => voice(b, { at: i * 0.07, dur: 0.5, f0: hz(n), gain: 0.24, decay: 6, partials: BELL })); voice(b, { at: 0.3, dur: 0.35, f0: hz("C8"), gain: 0.06, decay: 10, partials: PURE }); }],
  error: [0.45, (b) => { voice(b, { dur: 0.2, f0: hz("E4"), gain: 0.35, partials: SOFT }); voice(b, { at: 0.18, dur: 0.26, f0: hz("C4"), gain: 0.35, partials: SOFT }); }],
  approval: [0.5, (b) => { voice(b, { dur: 0.22, f0: hz("A5"), gain: 0.32, partials: BELL }); voice(b, { at: 0.2, dur: 0.28, f0: hz("A5"), gain: 0.32, partials: BELL }); }],
  question: [0.4, (b) => { voice(b, { dur: 0.16, f0: hz("G5"), gain: 0.3, partials: BELL }); voice(b, { at: 0.14, dur: 0.24, f0: hz("D6"), gain: 0.3, partials: BELL }); }],
  approve: [0.26, (b) => voice(b, { dur: 0.24, f0: hz("C6"), f1: hz("G6"), gain: 0.3, decay: 8, partials: BELL })],
  rate: [0.55, (b) => ["E5", "C5", "A4"].forEach((n, i) => voice(b, { at: i * 0.13, dur: 0.22, f0: hz(n), gain: 0.28, partials: SOFT }))],
  gulp: [0.3, (b) => voice(b, { dur: 0.28, f0: 520, f1: 140, gain: 0.45, decay: 8, partials: SOFT })],
  pop: [0.1, (b) => voice(b, { dur: 0.09, f0: 900, f1: 380, gain: 0.45, decay: 25, partials: PURE })],
  send: [0.3, (b) => { voice(b, { dur: 0.26, f0: hz("E5"), f1: hz("E7"), gain: 0.22, decay: 6, partials: PURE }); noise(b, { dur: 0.24, gain: 0.1, decay: 6 }); }],
  attach: [0.32, (b) => { voice(b, { dur: 0.03, f0: 2000, gain: 0.2, decay: 60, partials: PURE }); voice(b, { at: 0.06, dur: 0.03, f0: 2400, gain: 0.2, decay: 60, partials: PURE }); voice(b, { at: 0.1, dur: 0.22, f0: hz("A6"), gain: 0.22, partials: BELL }); }],
  love: [0.65, (b) => ["F5", "A5", "C6"].forEach((n, i) => voice(b, { at: i * 0.05, dur: 0.6, f0: hz(n), gain: 0.18, attack: 0.08, decay: 3, vib: 0.006, partials: SOFT }))],
  proud: [0.55, (b) => ["C6", "G6", "C7"].forEach((n, i) => voice(b, { at: i * 0.11, dur: 0.32, f0: hz(n), gain: 0.25, partials: BELL }))],
  wink: [0.16, (b) => voice(b, { dur: 0.14, f0: hz("E6"), f1: hz("B6"), gain: 0.28, decay: 14, partials: BELL })],
  yawn: [0.85, (b) => voice(b, { dur: 0.82, f0: hz("G5"), f1: hz("C5"), gain: 0.24, attack: 0.1, decay: 1.8, vib: 0.01, vibRate: 4, partials: SOFT })],
  sleep: [0.8, (b) => { voice(b, { dur: 0.36, f0: hz("E5"), gain: 0.2, attack: 0.06, decay: 3, partials: SOFT }); voice(b, { at: 0.34, dur: 0.44, f0: hz("C5"), gain: 0.2, attack: 0.06, decay: 3, partials: SOFT }); }],
};

function wav(samples) {
  const data = Buffer.alloc(samples.length * 2);
  // Normalise every sound to the same peak, then fade the last 5 ms out.
  const peak = Math.max(1e-6, ...samples.map(Math.abs));
  const fade = Math.floor(0.005 * RATE);
  samples.forEach((s, i) => {
    const tail = Math.min(1, (samples.length - i) / fade);
    data.writeInt16LE(Math.round((s / peak) * 0.89 * tail * 32767), i * 2);
  });
  const h = Buffer.alloc(44);
  h.write("RIFF", 0);
  h.writeUInt32LE(36 + data.length, 4);
  h.write("WAVE", 8);
  h.write("fmt ", 12);
  h.writeUInt32LE(16, 16);
  h.writeUInt16LE(1, 20);
  h.writeUInt16LE(1, 22);
  h.writeUInt32LE(RATE, 24);
  h.writeUInt32LE(RATE * 2, 28);
  h.writeUInt16LE(2, 32);
  h.writeUInt16LE(16, 34);
  h.write("data", 36);
  h.writeUInt32LE(data.length, 40);
  return Buffer.concat([h, data]);
}

mkdirSync(OUT, { recursive: true });
for (const [name, [seconds, play]] of Object.entries(SOUNDS)) {
  const buf = new Float64Array(Math.ceil(seconds * RATE));
  play(buf);
  writeFileSync(join(OUT, `${name}.wav`), wav(Array.from(buf)));
}
console.log(`${Object.keys(SOUNDS).length} sounds in ${OUT}`);
