// Hands-free voice chat, the island's half: listens on the microphone, cuts
// each utterance at the first real pause, and hands it over as the 16 kHz mono
// WAV whisper.cpp wants. Frank's spoken answer is played from here too. While
// Frank thinks or speaks the microphone is ignored, so it never hears itself.

/** whisper.cpp's input rate. */
const TARGET_RATE = 16_000;
/** A pause this long ends what the user was saying. */
const SILENCE_MS = 1_100;
/** Shorter bursts (a cough, a click, a key) are not speech. */
const MIN_SPEECH_MS = 350;
/** Nobody talks for this long in one go; cut and send it anyway. */
const MAX_UTTERANCE_MS = 30_000;
/** Audio kept from just before the voice got loud, so first syllables survive. */
const PREROLL_MS = 300;

/** How loud an utterance was: lets the wake word tell a voice at the desk from a TV. */
export interface UtteranceLevel {
  /** Loudest block of the utterance (RMS). */
  peak: number;
  /** The room's background level just before it. */
  floor: number;
  /** How long it was, pauses excluded. */
  ms: number;
}

export class VoiceCapture {
  private stream: MediaStream | null = null;
  private ctx: AudioContext | null = null;
  private node: ScriptProcessorNode | null = null;
  private listening = false;
  private inSpeech = false;
  private chunks: Float32Array[] = [];
  private preroll: Float32Array[] = [];
  private speechMs = 0;
  private silenceMs = 0;
  private peak = 0;
  /** Rolling estimate of the room's background level. */
  private noiseFloor = 0.01;

  /**
   * `autoGain` off keeps levels honest — a voice at the desk stays louder than
   * one across the room — which is what the wake word's loudness check needs.
   */
  constructor(
    private readonly onUtterance: (wav: Uint8Array, level: UtteranceLevel) => void,
    private readonly autoGain = true,
  ) {}

  /** True while the user is in the middle of saying something. */
  get speaking(): boolean {
    return this.listening && this.inSpeech;
  }

  async start() {
    this.stream = await navigator.mediaDevices.getUserMedia({
      audio: {
        echoCancellation: true,
        noiseSuppression: true,
        autoGainControl: this.autoGain,
        channelCount: 1,
      },
    });
    this.ctx = new AudioContext();
    const source = this.ctx.createMediaStreamSource(this.stream);
    // ScriptProcessor rather than an AudioWorklet: a worklet needs a module URL,
    // which the page's CSP does not allow from a blob, and this is plenty fast.
    this.node = this.ctx.createScriptProcessor(4096, 1, 1);
    this.node.onaudioprocess = (e) => this.process(e.inputBuffer.getChannelData(0));
    source.connect(this.node);
    // It only runs while connected; it writes nothing, so this plays silence.
    this.node.connect(this.ctx.destination);
    this.listening = true;
  }

  /** Stop hearing (Frank is thinking or speaking). */
  pause() {
    this.listening = false;
    this.reset();
  }

  resume() {
    this.reset();
    this.listening = true;
  }

  stop() {
    this.listening = false;
    this.node?.disconnect();
    this.stream?.getTracks().forEach((t) => t.stop());
    void this.ctx?.close();
    this.node = null;
    this.stream = null;
    this.ctx = null;
  }

  private reset() {
    this.inSpeech = false;
    this.chunks = [];
    this.preroll = [];
    this.speechMs = 0;
    this.silenceMs = 0;
    this.peak = 0;
  }

  private process(input: Float32Array) {
    if (!this.listening || !this.ctx) return;
    const rate = this.ctx.sampleRate;
    const block = new Float32Array(input); // the engine reuses its buffer
    const ms = (block.length / rate) * 1000;
    let sum = 0;
    for (const s of block) sum += s * s;
    const rms = Math.sqrt(sum / block.length);

    // Loud enough to start, a little less to keep going: speech dips between words.
    const startLevel = Math.max(0.02, this.noiseFloor * 3);
    const keepLevel = Math.max(0.012, this.noiseFloor * 1.8);

    if (!this.inSpeech) {
      this.noiseFloor = this.noiseFloor * 0.95 + rms * 0.05;
      this.preroll.push(block);
      while (this.preroll.length > 1 && this.preroll.length * ms > PREROLL_MS) this.preroll.shift();
      if (rms > startLevel) {
        this.inSpeech = true;
        this.chunks = this.preroll;
        this.preroll = [];
        this.speechMs = 0;
        this.silenceMs = 0;
        this.peak = rms;
      }
      return;
    }

    this.chunks.push(block);
    this.speechMs += ms;
    this.peak = Math.max(this.peak, rms);
    this.silenceMs = rms < keepLevel ? this.silenceMs + ms : 0;
    if (this.silenceMs < SILENCE_MS && this.speechMs < MAX_UTTERANCE_MS) return;

    const chunks = this.chunks;
    const voiced = this.speechMs - this.silenceMs;
    const level = { peak: this.peak, floor: this.noiseFloor, ms: voiced };
    this.reset();
    if (voiced >= MIN_SPEECH_MS) this.onUtterance(encodeWav(chunks, rate), level);
  }
}

/** Plays Frank's answer; `stop` cuts it short. */
export class VoicePlayer {
  private ctx: AudioContext | null = null;
  private source: AudioBufferSourceNode | null = null;
  /** Settles the play() in progress. A closed context may never fire "ended". */
  private finish: (() => void) | null = null;

  async play(wav: ArrayBuffer): Promise<void> {
    this.stop();
    const ctx = new AudioContext();
    this.ctx = ctx;
    const buffer = await ctx.decodeAudioData(wav);
    if (this.ctx !== ctx) return; // stopped while decoding
    const source = ctx.createBufferSource();
    source.buffer = buffer;
    source.connect(ctx.destination);
    this.source = source;
    await new Promise<void>((resolve) => {
      this.finish = resolve;
      source.onended = () => resolve();
      source.start();
    });
    if (this.ctx === ctx) this.stop();
  }

  stop() {
    const finish = this.finish;
    this.finish = null;
    try {
      this.source?.stop();
    } catch {
      // never started, or already over
    }
    void this.ctx?.close();
    this.source = null;
    this.ctx = null;
    finish?.();
  }
}

/**
 * Reads `text` sentence by sentence, the next one synthesized while the
 * current one plays: the first words come out as soon as the first sentence
 * is ready instead of after the whole answer. Stops when `cancelled` says so.
 */
export async function speakAll(
  text: string,
  synthesize: (sentence: string) => Promise<ArrayBuffer>,
  player: VoicePlayer,
  cancelled: () => boolean,
): Promise<void> {
  const parts = sentences(text);
  const prepare = (i: number) => {
    if (i >= parts.length) return null;
    const audio = synthesize(parts[i]);
    audio.catch(() => {}); // a cancelled read never awaits it
    return audio;
  };
  let next = prepare(0);
  for (let i = 0; next; i++) {
    const audio = await next;
    next = prepare(i + 1);
    if (cancelled()) return;
    await player.play(audio);
    if (cancelled()) return;
  }
}

/** Sentences of a reply, short ones joined to the next so Piper never reads a fragment. */
export function sentences(text: string): string[] {
  const raw = text.match(/[^.!?…]+(?:[.!?…]+["'»”)\]]*|$)/g) ?? [];
  const out: string[] = [];
  let carry = "";
  for (const piece of raw.map((s) => s.trim()).filter(Boolean)) {
    carry = carry ? `${carry} ${piece}` : piece;
    if (carry.length >= 24) {
      out.push(carry);
      carry = "";
    }
  }
  if (carry) {
    if (out.length > 0 && carry.length < 24) out[out.length - 1] += ` ${carry}`;
    else out.push(carry);
  }
  return out;
}

/** What is worth reading aloud: no links, no sources list, no markup. */
export function speakable(text: string): string {
  let t = text;
  const sources = t.search(/^\s*(sources|kaynaklar)\s*:?\s*$/im);
  if (sources > 0) t = t.slice(0, sources);
  return t
    .replace(/\[([^\]]+)\]\([^)]+\)/g, "$1")
    .replace(/https?:\/\/\S+/g, "")
    .replace(/^\s*[-•]\s+/gm, "")
    .replace(/[*_#`>|~]/g, "")
    .replace(/\s+/g, " ")
    .trim();
}

/** Mono float chunks at `rate` → a 16-bit 16 kHz WAV file. */
function encodeWav(chunks: Float32Array[], rate: number): Uint8Array {
  const total = chunks.reduce((n, c) => n + c.length, 0);
  const all = new Float32Array(total);
  let at = 0;
  for (const c of chunks) {
    all.set(c, at);
    at += c.length;
  }

  // Downsample by averaging each output sample's window: enough of a low-pass
  // for speech, and whisper.cpp only takes 16 kHz.
  const ratio = rate / TARGET_RATE;
  const count = Math.floor(total / ratio);
  const pcm = new Int16Array(count);
  for (let i = 0; i < count; i++) {
    const from = Math.floor(i * ratio);
    const to = Math.min(total, Math.floor((i + 1) * ratio));
    let sum = 0;
    for (let j = from; j < to; j++) sum += all[j];
    const v = Math.max(-1, Math.min(1, sum / Math.max(1, to - from)));
    pcm[i] = v < 0 ? v * 0x8000 : v * 0x7fff;
  }

  const bytes = new Uint8Array(44 + pcm.length * 2);
  const view = new DataView(bytes.buffer);
  const text = (offset: number, s: string) => {
    for (let i = 0; i < s.length; i++) view.setUint8(offset + i, s.charCodeAt(i));
  };
  text(0, "RIFF");
  view.setUint32(4, 36 + pcm.length * 2, true);
  text(8, "WAVE");
  text(12, "fmt ");
  view.setUint32(16, 16, true); // fmt chunk size
  view.setUint16(20, 1, true); // PCM
  view.setUint16(22, 1, true); // mono
  view.setUint32(24, TARGET_RATE, true);
  view.setUint32(28, TARGET_RATE * 2, true); // byte rate
  view.setUint16(32, 2, true); // block align
  view.setUint16(34, 16, true); // bits per sample
  text(36, "data");
  view.setUint32(40, pcm.length * 2, true);
  for (let i = 0; i < pcm.length; i++) view.setInt16(44 + i * 2, pcm[i], true);
  return bytes;
}
