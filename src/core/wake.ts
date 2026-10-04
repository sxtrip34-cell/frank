// The wake word: with it on, the island keeps an ear open and starts a spoken
// conversation when the user says it ("Frank, …"), wherever they are on the
// desktop. Every utterance is transcribed on this machine by whisper.cpp; only
// the ones that start with the wake word go any further.

import { Bridge } from "./bridge";
import { State } from "./state";
import { VoiceCapture, type UtteranceLevel } from "./voice";

/**
 * Only an utterance this loud — and this far above the room's background —
 * is checked for the wake word: someone at the desk, not a TV across the room.
 * Measured without automatic gain, so distance still shows in the level.
 */
const WAKE_MIN_PEAK = 0.035;
const WAKE_OVER_FLOOR = 5;
/** "Frank, …" is a sentence or two; a long monologue is somebody else talking. */
const WAKE_MAX_MS = 20_000;

/**
 * The wake word opens the sentence ("Frank, …"), or follows a greeting ("Hey
 * Frank, …"). Anywhere else it is just a name: "Bugün Frank Sinatra dinledim".
 * Folded forms, see fold().
 */
const GREETINGS = new Set([
  "hey", "hi", "hello", "ok", "okay", "alo", "ey", "selam", "merhaba", "privet",
]);

export class WakeListener {
  private capture: VoiceCapture | null = null;
  private busy = false;
  /** start() is waiting on the microphone; a second start() must not open another. */
  private starting = false;
  private stopRequested = false;
  private paused = false;

  /** `onWake` gets what was said after the wake word (maybe nothing). */
  constructor(private readonly onWake: (command: string) => void) {}

  get running(): boolean {
    return this.capture != null || this.starting;
  }

  async start() {
    if (this.running) return;
    this.starting = true;
    this.stopRequested = false;
    void Bridge.voiceWarm(); // have the model loaded before the first "Frank"
    const capture = new VoiceCapture((wav, level) => void this.heard(capture, wav, level), false);
    try {
      await capture.start();
    } finally {
      this.starting = false;
    }
    if (this.stopRequested) {
      capture.stop();
      return;
    }
    this.capture = capture;
    if (this.paused) capture.pause();
  }

  stop() {
    this.stopRequested = this.starting;
    this.capture?.stop();
    this.capture = null;
  }

  /** While a conversation runs, the conversation has the microphone. */
  pause() {
    this.paused = true;
    this.capture?.pause();
  }

  resume() {
    this.paused = false;
    this.capture?.resume();
  }

  private async heard(from: VoiceCapture, wav: Uint8Array, level: UtteranceLevel) {
    if (this.busy || from !== this.capture) return;
    // Numbers only — never what was said — so the thresholds can be tuned.
    const loud = level.peak >= Math.max(WAKE_MIN_PEAK, level.floor * WAKE_OVER_FLOOR);
    const verdict = !loud ? "too quiet" : level.ms > WAKE_MAX_MS ? "too long" : "checked";
    void Bridge.log(
      `wake: ${verdict} (${Math.round(level.ms)} ms, peak ${level.peak.toFixed(3)}, floor ${level.floor.toFixed(4)})`,
    );
    if (verdict !== "checked") return;
    this.busy = true;
    from.pause();
    let command: string | null = null;
    try {
      command = afterWakeWord(await Bridge.voiceTranscribe(wav), State.settings.wakeWord);
    } catch (err) {
      void Bridge.log(`wake: transcribe failed: ${String(err)}`);
    } finally {
      this.busy = false;
    }
    if (from !== this.capture) return;
    // Heard: stay paused; whoever runs the conversation resumes us after it.
    if (command !== null) this.onWake(command);
    else if (!this.paused) from.resume();
  }
}

/**
 * What follows the wake word in `text`, or null when it is not there. Matching
 * is forgiving about spelling — Whisper writes "Frank" as "Frenk" in Turkish
 * and "Франк" in Russian — but not about length: "fren" (Turkish for brake)
 * is not "Frank".
 */
export function afterWakeWord(text: string, word: string): string | null {
  const target = fold(word);
  if (target.length === 0) return null;
  const words = text.split(/\s+/).filter(Boolean);
  const at = sounds(fold(words[0] ?? ""), target)
    ? 0
    : GREETINGS.has(fold(words[0] ?? "")) && sounds(fold(words[1] ?? ""), target)
      ? 1
      : -1;
  if (at < 0) return null;
  return words.slice(at + 1).join(" ").replace(/^[\s,.!?;:—–-]+/, "").trim();
}

const CYRILLIC: Record<string, string> = {
  а: "a", б: "b", в: "v", г: "g", д: "d", е: "e", ё: "e", ж: "zh", з: "z", и: "i",
  й: "y", к: "k", л: "l", м: "m", н: "n", о: "o", п: "p", р: "r", с: "s", т: "t",
  у: "u", ф: "f", х: "h", ц: "c", ч: "ch", ш: "sh", щ: "sh", ы: "y", э: "e", ю: "yu",
  я: "ya",
};

const TURKISH: Record<string, string> = { ı: "i", ş: "s", ç: "c", ğ: "g", ö: "o", ü: "u" };

/** Lowercase Latin letters only: Cyrillic transliterated, Turkish letters plain. */
function fold(word: string): string {
  let out = "";
  for (const c of word.toLocaleLowerCase("tr")) {
    const mapped = CYRILLIC[c] ?? TURKISH[c] ?? c;
    if (/^[a-z]+$/.test(mapped)) out += mapped;
  }
  return out;
}

/** Same word, give or take one letter — but never a shorter one. */
function sounds(heard: string, target: string): boolean {
  if (heard === target) return true;
  if (target.length < 4 || heard.length < target.length) return false;
  return editDistance(heard, target) <= 1;
}

function editDistance(a: string, b: string): number {
  const row = Array.from({ length: b.length + 1 }, (_, j) => j);
  for (let i = 1; i <= a.length; i++) {
    let diagonal = row[0];
    row[0] = i;
    for (let j = 1; j <= b.length; j++) {
      const above = row[j];
      row[j] = Math.min(row[j] + 1, row[j - 1] + 1, diagonal + (a[i - 1] === b[j - 1] ? 0 : 1));
      diagonal = above;
    }
  }
  return row[b.length];
}
