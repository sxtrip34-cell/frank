// Frank's look: a small ball of light with one big eye, a soft halo and three
// motes of light circling it. Drawn in code, like everything else — the
// island's Frank, the launch greeting and the file-drop scene all draw him
// through drawOrb(), so he looks the same everywhere.
//
// Everything is drawn around (0, 0): callers translate to the orb's centre.

export type RGB = readonly [number, number, number]; // components 0…1

export type OrbEye =
  | "pill" | "wide" | "dot" | "line" | "flat" | "happy" | "closed"
  | "spiral" | "heart" | "star" | "tired" | "wink" | "cup";

export interface OrbLook {
  /** Radius of the ball, in canvas pixels. */
  r: number;
  /** Main colour: the mood (state) colour, or a pill's own colour. */
  color: RGB;
  /** Halo strength, 0…1. */
  glow: number;
  eye: OrbEye;
  /** Eyelid, 0 (shut) … 1 (open): blinks. */
  open: number;
  /** Where the eye looks, −1…1 each way (+y is down). */
  lookX: number;
  lookY: number;
  /** Foreshortening of the eye as it turns away, 0…1 each way. */
  eyeSquashX?: number;
  eyeSquashY?: number;
  /** 0 hides the eye (it has rolled round the back). */
  eyeVisible?: number;
  eyeScale?: number;
  /** Seconds, for the shimmer, the motes and spinning eyes. */
  time: number;
  /** Orbiting motes: 0 none … 1 all three. */
  motes: number;
  /** Mote speed multiplier: they whirl when Frank waves hello. */
  moteSpeed?: number;
  /** Squash and stretch, and tilt, of the ball itself. */
  sx?: number;
  sy?: number;
  tilt?: number;
  /** Small sizes (pills): no shimmer, a plainer eye. */
  flat?: boolean;
}

const TAU = Math.PI * 2;
const WHITE: RGB = [1, 1, 1];
const NIGHT: RGB = [0.04, 0.06, 0.14];
/** The eye: a deep night blue rather than black, so it sits in the light. */
const INK = "rgb(10,14,30)";

/** Frank's own colour at rest: electric cyan. */
export const ORB_BASE: RGB = [0.37, 0.86, 0.98];

export function hexToRGB(hex: string): RGB {
  const v = parseInt(hex.replace("#", ""), 16);
  return [((v >> 16) & 255) / 255, ((v >> 8) & 255) / 255, (v & 255) / 255];
}

export const rgba = (c: RGB, a = 1) =>
  `rgba(${Math.round(c[0] * 255)},${Math.round(c[1] * 255)},${Math.round(c[2] * 255)},${a})`;

export const mixRGB = (a: RGB, b: RGB, t: number): RGB => [
  a[0] + (b[0] - a[0]) * t,
  a[1] + (b[1] - a[1]) * t,
  a[2] + (b[2] - a[2]) * t,
];

const clamp = (v: number, a: number, b: number) => Math.max(a, Math.min(b, v));

export function drawOrb(x: CanvasRenderingContext2D, o: OrbLook) {
  const t = o.time;
  const breathe = 1 + Math.sin(t * 2.1) * 0.025;
  const R = o.r * breathe;

  // Halo: soft light spilling around the ball, breathing with it.
  if (o.glow > 0.01) {
    const g = x.createRadialGradient(0, 0, R * 0.5, 0, 0, R * 2);
    g.addColorStop(0, rgba(o.color, 0.42 * o.glow));
    g.addColorStop(0.45, rgba(o.color, 0.15 * o.glow));
    g.addColorStop(1, rgba(o.color, 0));
    x.fillStyle = g;
    x.beginPath();
    x.arc(0, 0, R * 2, 0, TAU);
    x.fill();
  }

  if (o.motes > 0.01) drawMotes(x, o, R, "back");

  x.save();
  if (o.tilt) x.rotate(o.tilt);
  x.scale(o.sx ?? 1, o.sy ?? 1);

  // The ball: white-hot near the top left, the mood colour through the
  // middle, deepening at the rim.
  const body = x.createRadialGradient(-R * 0.3, -R * 0.34, R * 0.04, 0, 0, R);
  body.addColorStop(0, "rgba(255,255,255,0.98)");
  body.addColorStop(0.3, rgba(mixRGB(o.color, WHITE, 0.7)));
  body.addColorStop(0.76, rgba(o.color));
  body.addColorStop(1, rgba(mixRGB(o.color, NIGHT, 0.42)));
  x.fillStyle = body;
  x.beginPath();
  x.arc(0, 0, R, 0, TAU);
  x.fill();

  // Shimmer: two soft patches of light drifting round inside.
  if (!o.flat) {
    x.save();
    x.beginPath();
    x.arc(0, 0, R, 0, TAU);
    x.clip();
    for (const [offset, speed, alpha] of [[0, 0.6, 0.24], [Math.PI, -0.45, 0.14]] as const) {
      const a = t * speed + offset;
      const px = Math.cos(a) * R * 0.46;
      const py = Math.sin(a) * R * 0.3;
      const s = x.createRadialGradient(px, py, 0, px, py, R * 0.6);
      s.addColorStop(0, `rgba(255,255,255,${alpha})`);
      s.addColorStop(1, "rgba(255,255,255,0)");
      x.fillStyle = s;
      x.fillRect(-R, -R, R * 2, R * 2);
    }
    x.restore();
  }

  // Rim light.
  x.strokeStyle = rgba(mixRGB(o.color, WHITE, 0.55), 0.6);
  x.lineWidth = Math.max(0.8, R * 0.05);
  x.beginPath();
  x.arc(0, 0, R * 0.97, 0, TAU);
  x.stroke();

  const visible = o.eyeVisible ?? 1;
  if (visible > 0.01) {
    x.save();
    x.globalAlpha *= visible;
    drawEye(x, R, o);
    x.restore();
  }
  x.restore();

  if (o.motes > 0.01) drawMotes(x, o, R, "front");
}

/** Three motes on tilted orbits: those behind the ball are drawn before it. */
function drawMotes(x: CanvasRenderingContext2D, o: OrbLook, R: number, layer: "back" | "front") {
  const speed = o.moteSpeed ?? 1;
  for (let i = 0; i < 3; i++) {
    const a = o.time * (1.05 + i * 0.32) * speed + i * 2.1;
    const depth = Math.sin(a);
    if ((layer === "back") !== (depth < 0)) continue;
    const orbit = R * (1.42 + 0.1 * i);
    const px = Math.cos(a) * orbit;
    const py = depth * orbit * 0.34 + R * (0.1 - i * 0.12);
    const s = R * (0.075 + 0.025 * depth) * o.motes;
    const g = x.createRadialGradient(px, py, 0, px, py, s * 3.2);
    g.addColorStop(0, "rgba(255,255,255,0.95)");
    g.addColorStop(0.3, rgba(mixRGB(o.color, WHITE, 0.4), 0.7));
    g.addColorStop(1, rgba(o.color, 0));
    x.fillStyle = g;
    x.beginPath();
    x.arc(px, py, s * 3.2, 0, TAU);
    x.fill();
  }
}

function drawEye(x: CanvasRenderingContext2D, R: number, o: OrbLook) {
  const er = R * 0.34 * (o.eyeScale ?? 1);
  x.translate(clamp(o.lookX, -1, 1) * R * 0.38, clamp(o.lookY, -1, 1) * R * 0.32);
  x.scale(Math.max(0.2, o.eyeSquashX ?? 1), Math.max(0.2, o.eyeSquashY ?? 1));
  x.fillStyle = INK;
  x.strokeStyle = INK;
  x.lineCap = "round";

  switch (o.eye) {
    case "wide":
      lens(x, er * 1.16, o.open, o);
      break;
    case "pill":
      lens(x, er, o.open, o);
      break;
    case "dot":
      lens(x, er * 0.48, 1, o);
      break;
    case "line":
      x.rotate(-0.22);
      pill(x, er * 1.9, er * 0.36);
      break;
    case "flat":
      pill(x, er * 1.8, er * 0.34);
      break;
    case "happy":
    case "wink":
      // A single eye has no wink: it smiles instead, with a little tilt.
      if (o.eye === "wink") x.rotate(0.18);
      x.lineWidth = er * 0.42;
      x.beginPath();
      x.arc(0, er * 0.38, er * 0.86, Math.PI * 1.15, Math.PI * 1.85);
      x.stroke();
      break;
    case "closed":
      x.lineWidth = er * 0.32;
      x.beginPath();
      x.arc(0, -er * 0.3, er * 0.86, Math.PI * 0.15, Math.PI * 0.85);
      x.stroke();
      break;
    case "tired":
      // Half-shut: the lower half of the lens under a heavy lid.
      x.save();
      x.beginPath();
      x.rect(-er * 1.2, -er * 0.05, er * 2.4, er * 1.3);
      x.clip();
      lens(x, er, 1, o);
      x.restore();
      pill(x, er * 2.1, er * 0.22);
      break;
    case "spiral": {
      x.lineWidth = er * 0.16;
      x.beginPath();
      for (let a = 0; a < 4.6 * Math.PI; a += 0.18) {
        const r = er * 0.05 + a * er * 0.058;
        const aa = a + o.time * 9;
        if (a === 0) x.moveTo(Math.cos(aa) * r, Math.sin(aa) * r);
        else x.lineTo(Math.cos(aa) * r, Math.sin(aa) * r);
      }
      x.stroke();
      break;
    }
    case "heart":
      x.fillStyle = "#FF4D6D";
      heartPath(x, er * 1.25);
      x.fill();
      break;
    case "star":
      x.fillStyle = "#F7B32B";
      x.rotate(o.time * 1.5);
      starPath(x, er * 1.1, er * 0.48);
      x.fill();
      break;
    case "cup": {
      // A wide-open "o", looking up at what is about to be swallowed.
      const w = er * 1.15;
      const h = er * 1.2;
      x.beginPath();
      x.moveTo(-w / 2, -h / 2);
      x.lineTo(w / 2, -h / 2);
      x.lineTo(w / 2, h / 2 - w / 2);
      x.arc(0, h / 2 - w / 2, w / 2, 0, Math.PI, false);
      x.closePath();
      x.fill();
      break;
    }
  }
}

/** The open eye: a dark lens with a ring of the mood's light and a glint. */
function lens(x: CanvasRenderingContext2D, r: number, open: number, o: OrbLook) {
  const ry = Math.max(r * 0.12, r * clamp(open, 0, 1));
  x.beginPath();
  x.ellipse(0, 0, r, ry, 0, 0, TAU);
  x.fill();
  if (open < 0.45) return;
  if (!o.flat) {
    x.strokeStyle = rgba(mixRGB(o.color, WHITE, 0.35), 0.85);
    x.lineWidth = r * 0.11;
    x.beginPath();
    x.ellipse(0, 0, r * 0.6, ry * 0.6, 0, 0, TAU);
    x.stroke();
    x.strokeStyle = INK;
  }
  x.fillStyle = "rgba(255,255,255,0.92)";
  x.beginPath();
  x.arc(-r * 0.32, -ry * 0.34, r * 0.17, 0, TAU);
  x.fill();
  x.fillStyle = INK;
}

function pill(x: CanvasRenderingContext2D, w: number, h: number) {
  x.beginPath();
  x.roundRect(-w / 2, -h / 2, w, h, h / 2);
  x.fill();
}

export function heartPath(x: CanvasRenderingContext2D, s: number) {
  x.beginPath();
  x.moveTo(0, s * 0.38);
  x.bezierCurveTo(-s * 1.05, -s * 0.15, -s * 0.5, -s * 0.95, 0, -s * 0.38);
  x.bezierCurveTo(s * 0.5, -s * 0.95, s * 1.05, -s * 0.15, 0, s * 0.38);
  x.closePath();
}

export function starPath(x: CanvasRenderingContext2D, ro: number, ri: number) {
  x.beginPath();
  for (let i = 0; i < 10; i++) {
    const r = i % 2 ? ri : ro;
    const a = -Math.PI / 2 + (i * Math.PI) / 5;
    x.lineTo(Math.cos(a) * r, Math.sin(a) * r);
  }
  x.closePath();
}

/**
 * The opening Frank swallows a dropped file through: a dark gap with a lit
 * rim, centred on (cx, cy), `w`×`h`.
 */
export function drawMouth(x: CanvasRenderingContext2D, cx: number, cy: number, w: number, h: number, color: RGB) {
  if (h <= 0.3) return;
  const g = x.createRadialGradient(cx, cy, 0, cx, cy, w / 2);
  g.addColorStop(0, "#020309");
  g.addColorStop(0.75, rgba(mixRGB(color, NIGHT, 0.8)));
  g.addColorStop(1, rgba(mixRGB(color, NIGHT, 0.5)));
  x.fillStyle = g;
  x.beginPath();
  x.ellipse(cx, cy, w / 2, h / 2, 0, 0, TAU);
  x.fill();
  x.strokeStyle = rgba(mixRGB(color, WHITE, 0.6), 0.85);
  x.lineWidth = Math.max(1, h * 0.12);
  x.stroke();
}
