// Dev harness: the island docked to an edge (or free), in a plain browser
// sized like the island window (720×320), so each shape can be looked at
// without dragging the real one. Not part of the app bundle.
//
//   dock-preview.html?dock=left|right|top|free&mode=tab|drawer|hidden
//                    &ax=…&ay=…        anchor inside the window, as Rust sends it
//                    &preview=top|left|right   the edge lit while dragging

import "../src/style.css";
import { State, type AgentTask, type Dock } from "../src/core/state";
import type { BotStateName } from "../src/core/layout";
import { Island } from "../src/island/island";

const task = (id: string, name: string, color: string, state: BotStateName): AgentTask => ({
  id,
  name,
  color,
  state,
  stepIndex: 0,
  steps: [],
  source: "claudeCode",
  isIntegration: false,
});

State.tasks = [
  task("a", "web", "#60A5FA", "working"),
  task("b", "api", "#22C55E", "finished"),
  task("c", "docs", "#E879F9", "thinking"),
];
State.focusId = "a";

const q = new URLSearchParams(location.search);
const dock = (q.get("dock") ?? "right") as Dock;
const defaults: Record<Dock, [number, number]> = { top: [360, 0], left: [0, 0], right: [720, 0], free: [360, 0] };
const ax = Number(q.get("ax") ?? defaults[dock][0]);
const ay = Number(q.get("ay") ?? defaults[dock][1]);

const island = new Island(document.getElementById("root")!);
island.applySettings();
island.onAnchor({ x: ax, y: ay, dock });
const preview = q.get("preview") as Dock | null;
if (preview) island.onDragDock(preview);

const mode = q.get("mode") ?? "tab";
if (mode === "tab") island.fsm.mouseEntered();
else if (mode === "drawer") island.fsm.forceHome();
// "hidden" stays as it starts: hidden, with the handle at a side edge.

// A headless browser (used for screenshots) barely runs requestAnimationFrame,
// which would catch the island halfway through opening. Step its frame loop
// from a timer for a couple of seconds so it settles either way.
const frame = (island as unknown as { frame: (now: number) => void }).frame;
const pump = window.setInterval(() => frame(performance.now()), 16);
window.setTimeout(() => window.clearInterval(pump), 2000);
