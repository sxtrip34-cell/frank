// Chat view — DOM port of PromptView / ChatBubble / TypingDotsView from
// IslandViewContent.swift.

import { h, svg, clear } from "./dom";
import { ICONS } from "./icons";
import { Bridge, type ChatContext, type SessionNote } from "../core/bridge";
import { Sound } from "../core/sound";
import { State, type ChatMessage } from "../core/state";
import { VoiceCapture, VoicePlayer, speakAll, speakable } from "../core/voice";
import { afterWakeWord } from "../core/wake";
import { t, type MessageKey } from "../core/i18n";
import type { ViewActions, ViewHost } from "./views";

type VoicePhase = "listening" | "transcribing" | "thinking" | "speaking";

/** A conversation the wake word started ends after this much quiet. */
const WAKE_IDLE_MS = 15_000;

/** The text field tells the user what voice chat is doing right now. */
const VOICE_PLACEHOLDERS: Record<VoicePhase, MessageKey> = {
  listening: "chat.voice.listening",
  transcribing: "chat.voice.transcribing",
  thinking: "chat.voice.thinking",
  speaking: "chat.voice.speaking",
};

/** voice_status names what is missing in English; these are its words for it. */
const MISSING_VOICE_PARTS: Record<string, MessageKey> = {
  "a Whisper model": "chat.voice.whisperModel",
  "a Piper voice": "chat.voice.piperVoice",
};

let nextId = 1;

/** The attachment that already went out, so each one is sent exactly once. */
let sentFilePath: string | null = null;

/** Sessions quiet for longer than this are left out of what the chat hears. */
const SESSION_MAX_AGE_MS = 3 * 60 * 60 * 1000;

/** The live coding sessions, most recently active first, for the chat. */
function sessionNotes(): SessionNote[] {
  const now = Date.now();
  return [...State.liveSessions.values()]
    .filter((s) => now - s.updatedAt < SESSION_MAX_AGE_MS)
    .sort((a, b) => b.updatedAt - a.updatedAt)
    .map((s) => ({
      project: s.project,
      cwd: s.cwd,
      agent: s.agent,
      state: s.state,
      steps: s.steps,
      lastMessage: s.lastMessage,
      minutesAgo: Math.floor((now - s.updatedAt) / 60_000),
    }));
}

/** Claude scales images down to about this anyway; anything bigger only costs more. */
const MAX_IMAGE_EDGE = 1568;

/** A pasted image, shrunk to MAX_IMAGE_EDGE and re-encoded as PNG. */
async function toPng(blob: Blob): Promise<Uint8Array> {
  const bitmap = await createImageBitmap(blob);
  const scale = Math.min(1, MAX_IMAGE_EDGE / Math.max(bitmap.width, bitmap.height));
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(bitmap.width * scale));
  canvas.height = Math.max(1, Math.round(bitmap.height * scale));
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error(t("chat.imageUnreadable"));
  ctx.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
  bitmap.close();
  const png = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, "image/png"));
  if (!png) throw new Error(t("chat.imageUnreadable"));
  return new Uint8Array(await png.arrayBuffer());
}

function bubble(message: ChatMessage): HTMLElement {
  if (message.role === "user") {
    return h(
      "div",
      { class: "chat-row user" },
      h("div", { class: "bubble", text: message.content }),
    );
  }
  return h("div", { class: "chat-row" }, h("div", { class: "reply", text: message.content }));
}

function typingDots(): HTMLElement {
  return h(
    "div",
    { class: "chat-row" },
    h("div", { class: "typing" }, h("i"), h("i"), h("i")),
  );
}

/** The coloured chip showing what the question is about (an attached file), with × to take it off. */
function contextChip(label: string, onRemove: () => void): HTMLElement {
  const remove = h("button", { class: "chip-remove", title: t("chat.attach.remove") }, svg(ICONS.xmark, 9));
  remove.addEventListener("click", onRemove);
  const chip = h("div", { class: "chip" }, h("i", { class: "chip-dot" }), h("span", { text: label }), remove);
  requestAnimationFrame(() => chip.classList.add("settled"));
  return chip;
}

export function buildPrompt(actions: ViewActions, onHeightChange: () => void): ViewHost {
  const chipRow = h("div", { class: "chip-row" });
  const log = h("div", { class: "chat-log" });
  const input = h("input", {
    type: "text",
    class: "chat-input",
    placeholder: t("chat.placeholder.first"),
    spellcheck: "false",
  }) as HTMLInputElement;
  const attach = h("button", { class: "attach-btn", title: t("chat.attach") }, svg(ICONS.paperclip, 13, { stroke: 2 }));
  const mic = h("button", { class: "mic-btn", title: t("chat.voice") }, svg(ICONS.mic, 13));
  const send = h("button", { class: "send-btn", title: t("chat.send") }, svg(ICONS.arrowUp, 11));
  const bar = h("div", { class: "chat-bar" }, attach, input, mic, send);

  const card = h("div", { class: "card wash chat-card" }, h("div", { class: "chat-body" }, chipRow, log, bar));
  const el = h("div", { class: "view" }, card);
  card.style.setProperty("--wash", "rgba(99,102,241,0.5)");

  let sending = false;
  let renderedCount = -1;

  async function submit() {
    const query = input.value.trim();
    if (!query || sending || capture) return;
    input.value = "";
    await ask(query, false);
    input.focus();
  }

  /** One turn, typed or spoken. Returns the answer, or null after an error. */
  async function ask(query: string, voice: boolean): Promise<string | null> {
    sending = true;
    Sound.play("send");

    State.chatHistory.push({ id: nextId++, role: "user", content: query });
    State.stateOverride = "thinking";
    State.notify();
    onHeightChange();

    // A dropped or pasted file rides along with the first message after it
    // arrived — mid-conversation too, for a screenshot pasted later on.
    const file = State.droppedFile;
    // No path yet: the dropped bytes are still on their way into the inbox.
    const context: ChatContext | null =
      file && file.path && file.path !== sentFilePath ? { kind: "file", name: file.name, path: file.path } : null;

    try {
      const reply = await Bridge.chatSend(query, context, voice, sessionNotes());
      if (context) sentFilePath = context.path;
      State.chatHistory.push({ id: nextId++, role: "assistant", content: reply.text });
      State.stateOverride = null;
      // A spoken answer is its own signal; the chime would talk over it.
      if (!voice) Sound.play("finish");
      return reply.text;
    } catch (err) {
      State.stateOverride = null;
      State.noteMessage = String(err).replace(/^Error:\s*/, "");
      State.view = "note";
      Sound.play("error");
      return null;
    } finally {
      sending = false;
      State.notify();
      onHeightChange();
    }
  }

  // ── Voice chat ──────────────────────────────────────────────────────────────
  // Talk, Frank answers out loud, and it listens again — until the mic is
  // tapped, or the chat closes. Whisper and Piper both run on this machine.

  let capture: VoiceCapture | null = null;
  let phase: VoicePhase | null = null;
  const player = new VoicePlayer();

  function setPhase(next: VoicePhase | null) {
    phase = next;
    lastVoiceActivity = Date.now();
    State.notify();
  }

  function showNote(message: string) {
    State.noteMessage = message;
    State.view = "note";
    Sound.play("error");
    State.notify();
  }

  /** Started by the wake word: ends by itself after a quiet spell. */
  let autoEnd = false;
  let lastVoiceActivity = 0;
  let idleTimer: number | null = null;

  /**
   * `command`: what the user already said after the wake word, answered
   * straight away. `autoEnd`: a wake-word conversation, which closes itself.
   */
  async function startVoice(opts: { command?: string; autoEnd?: boolean } = {}) {
    // Claim the microphone now, so the wake listener stays out of the way.
    State.voiceActive = true;
    void Bridge.voiceWarm();
    const failed = (message: string) => {
      State.voiceActive = false;
      showNote(message);
    };
    const status = await Bridge.voiceStatus();
    if (!status?.ready) {
      const missing = status?.missing
        .map((part) => (MISSING_VOICE_PARTS[part] ? t(MISSING_VOICE_PARTS[part]) : part))
        .join(", ");
      failed(t("chat.voice.missing", { missing: missing || t("chat.voice.missingTools") }));
      return;
    }
    const next = new VoiceCapture((wav) => void heard(next, wav));
    try {
      await next.start();
    } catch (err) {
      failed(t("chat.voice.noMic", { error: String(err).replace(/^\w*Error:\s*/, "") }));
      return;
    }
    capture = next;
    autoEnd = opts.autoEnd ?? false;
    actions.setVoiceActive(true);
    if (autoEnd && idleTimer == null) idleTimer = window.setInterval(checkIdle, 1000);
    const command = opts.command?.trim();
    if (command) {
      void respond(next, command);
      return;
    }
    Sound.play("blip");
    setPhase("listening");
  }

  function stopVoice() {
    if (!capture) return;
    capture.stop();
    capture = null;
    player.stop();
    phase = null;
    if (idleTimer != null) {
      window.clearInterval(idleTimer);
      idleTimer = null;
    }
    State.voiceActive = false;
    actions.setVoiceActive(false);
    State.notify();
  }

  /** A wake-word conversation ends after a quiet spell, and the island folds away. */
  function checkIdle() {
    if (!capture || !autoEnd || phase !== "listening") return;
    if (capture.speaking) {
      lastVoiceActivity = Date.now();
      return;
    }
    if (Date.now() - lastVoiceActivity < WAKE_IDLE_MS) return;
    stopVoice();
    actions.collapse();
  }

  /** Each answer gets a number; one that was interrupted stops at the next check. */
  let turnId = 0;
  let barging = false;

  /** One utterance from `from`: transcribe, then answer it. */
  async function heard(from: VoiceCapture, wav: Uint8Array) {
    if (capture !== from) return;
    if (phase === "speaking") {
      void bargeIn(from, wav);
      return;
    }
    if (sending) return;
    from.pause();
    setPhase("transcribing");
    let text = "";
    try {
      text = (await Bridge.voiceTranscribe(wav)).trim();
    } catch (err) {
      void Bridge.log(`voice: transcribe failed: ${String(err)}`);
    }
    if (capture !== from) return;
    if (!text) {
      setPhase("listening");
      from.resume();
      return;
    }
    await respond(from, text);
  }

  /** Ask, read the answer aloud, and listen again. */
  async function respond(from: VoiceCapture, text: string) {
    const turn = ++turnId;
    const over = () => capture !== from || turn !== turnId;
    from.pause();
    setPhase("thinking");
    const reply = await ask(text, true);
    // Stopped meanwhile — or the error note took the view, which stops it.
    if (over()) return;
    if (reply) {
      setPhase("speaking");
      // Keep listening while speaking: the wake word cuts the answer short.
      from.resume();
      try {
        const words = speakable(reply);
        if (words) await speakAll(words, (s) => Bridge.voiceSpeak(s), player, over);
      } catch (err) {
        void Bridge.log(`voice: speak failed: ${String(err)}`);
      }
    }
    if (over()) return;
    setPhase("listening");
    from.resume();
  }

  /**
   * Something was said while Frank was speaking: its own voice through the
   * speakers, someone in the room, or the user cutting in. Only the wake word
   * interrupts; then whatever followed it is the next request.
   */
  async function bargeIn(from: VoiceCapture, wav: Uint8Array) {
    if (barging) return;
    barging = true;
    let command: string | null = null;
    try {
      const text = await Bridge.voiceTranscribe(wav).catch(() => "");
      if (capture === from && phase === "speaking") {
        command = afterWakeWord(text, State.settings.wakeWord);
      }
    } finally {
      // Only the check is exclusive; the answer that follows may be cut short too.
      barging = false;
    }
    if (command === null) return;
    turnId++; // the answer being read is dropped
    player.stop();
    Sound.play("pop");
    if (command) {
      await respond(from, command);
    } else {
      setPhase("listening");
      from.resume();
    }
  }

  mic.addEventListener("click", () => {
    if (capture) stopVoice();
    else void startVoice();
  });

  State.subscribe(() => {
    // The wake word was heard: the island has opened on the chat for it.
    if (State.voiceWake && !capture && State.view === "prompt" && State.mode === "expanded") {
      const { command } = State.voiceWake;
      State.voiceActive = true; // before the request goes, so the mic is never left unclaimed
      State.voiceWake = null;
      void startVoice({ command, autoEnd: true });
      return;
    }
    // Leaving the chat, or the island closing, ends voice chat.
    if (capture && (State.view !== "prompt" || State.mode !== "expanded")) stopVoice();
  });

  /** Ctrl+V with an image on the clipboard (Win+Shift+S) attaches it. */
  async function attachImage(blob: Blob) {
    try {
      const file = await Bridge.ingestImage(await toPng(blob));
      State.droppedFile = { name: file.name, path: file.path };
      Sound.play("attach");
    } catch (err) {
      State.noteMessage = String(err).replace(/^Error:\s*/, "");
      State.view = "note";
      Sound.play("error");
    } finally {
      State.notify();
      onHeightChange();
    }
  }

  /**
   * A file for the next question, from the Open dialog, the clipboard or a drop
   * on the chat: it rides along with the next message, like a pasted screenshot.
   * `pending` resolves to null when there turned out to be nothing to attach.
   */
  async function attachFile(pending: Promise<{ name: string; path: string } | null>): Promise<boolean> {
    try {
      const file = await pending;
      if (!file) return false;
      State.droppedFile = { name: file.name, path: file.path };
      Sound.play("attach");
      return true;
    } catch (err) {
      State.noteMessage = String(err).replace(/^Error:\s*/, "");
      State.view = "note";
      Sound.play("error");
      return true;
    } finally {
      State.notify();
      onHeightChange();
      input.focus();
    }
  }

  // The Open dialog takes the user away from the island: it must still be open
  // when they come back with a file.
  attach.addEventListener("click", async () => {
    actions.keepOpen(true);
    try {
      await attachFile(Bridge.pickFile(t("chat.attach.dialog")));
    } finally {
      actions.keepOpen(false);
    }
  });

  send.addEventListener("click", () => void submit());
  input.addEventListener("paste", (e) => {
    const files = Array.from(e.clipboardData?.items ?? []).filter((i) => i.kind === "file");
    if (files.length === 0) return; // text pastes as usual
    e.preventDefault();
    // Taken now: the clipboard data is gone once the event is over.
    const image = files.find((i) => i.type.startsWith("image/"))?.getAsFile() ?? null;
    void (async () => {
      // A file copied in Explorer goes as itself, read by its path; with no
      // such file on the clipboard it is a screenshot (Win+Shift+S): pixels.
      if (await attachFile(Bridge.pasteFile())) return;
      if (image) await attachImage(image);
    })();
  });
  input.addEventListener("keydown", (e) => {
    if ((e as KeyboardEvent).key === "Enter") {
      e.preventDefault();
      void submit();
    }
    e.stopPropagation(); // Escape closes the island, not the chat
  });

  return {
    el,
    attach(file) {
      void attachFile(Bridge.ingestDropped(file));
    },
    sync() {
      const file = State.droppedFile;
      const wantChip = file?.name ?? "";
      if (chipRow.dataset.label !== wantChip) {
        chipRow.dataset.label = wantChip;
        clear(chipRow);
        if (wantChip) {
          chipRow.append(
            contextChip(wantChip, () => {
              State.droppedFile = null;
              State.notify();
              onHeightChange();
            }),
          );
        }
      }
      // A file held over the open chat goes into it when let go.
      card.classList.toggle("drop-ready", State.fileDragOver);

      const thinking = State.stateOverride === "thinking";
      const count = State.chatHistory.length + (thinking ? 0.5 : 0);
      if (count !== renderedCount) {
        renderedCount = count;
        clear(log);
        for (const m of State.chatHistory) log.append(bubble(m));
        if (thinking) log.append(typingDots());
        log.scrollTop = log.scrollHeight;
      }

      input.placeholder = phase
        ? t(VOICE_PLACEHOLDERS[phase])
        : t(State.chatHistory.length === 0 ? "chat.placeholder.first" : "chat.placeholder.more");
      input.disabled = sending || capture != null;
      mic.classList.toggle("on", capture != null);
      mic.dataset.phase = phase ?? "";
    },
    focus() {
      input.focus();
      input.select();
    },
  };
}
