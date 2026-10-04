// Entry point: boot the bridge, wire the island, start the greeting.

import "./style.css";
import { Bridge, IS_TAURI, onEvent } from "./core/bridge";
import { lang } from "./core/i18n";
import { Sound } from "./core/sound";
import {
  State, type ChatMessage, type Dock, type IntegrationInfo, type IslandAnchor, type LiveSession,
  type Settings,
} from "./core/state";
import { Island } from "./island/island";
import { registerHookHandlers } from "./island/hooks";
import { registerIntegrationHandlers, refreshConfigured } from "./island/integrations";
import { WakeListener } from "./core/wake";

/**
 * What the island takes across the reload a language change needs. Everything
 * else comes back by itself: settings from Rust, pills from the next hook
 * event, the wake word from the settings.
 */
interface Carry {
  paused: boolean;
  sessions: LiveSession[];
  integrations: Record<string, IntegrationInfo>;
  chat: ChatMessage[];
  file: { name: string; path: string } | null;
}

const CARRY_KEY = "frank.carry";

let reloading = false;

/**
 * Every view is built in one language, so a new one means loading the page
 * again. It is fine that the launch greeting plays once more.
 */
async function reloadForLanguage() {
  if (reloading) return;
  reloading = true;
  // The approval card will not come back: hand its request to the terminal now
  // rather than leave Claude Code waiting on a click that can never come.
  const pending = State.pendingApproval;
  if (pending?.requestId) await Bridge.approvalDecline(pending.requestId);
  const carry: Carry = {
    paused: State.paused,
    sessions: [...State.liveSessions.values()],
    integrations: State.integrations,
    chat: State.chatHistory,
    file: State.droppedFile,
  };
  try {
    sessionStorage.setItem(CARRY_KEY, JSON.stringify(carry));
  } catch {
    // No storage: the island starts afresh, which is all that is lost.
  }
  window.location.reload();
}

/** What reloadForLanguage left behind, when this page is its reload. */
function takeCarry(): Carry | null {
  try {
    const raw = sessionStorage.getItem(CARRY_KEY);
    sessionStorage.removeItem(CARRY_KEY);
    return raw ? (JSON.parse(raw) as Carry) : null;
  } catch {
    return null;
  }
}

async function main() {
  const root = document.getElementById("root");
  if (!root) return;

  void Sound.preload();

  // Settings before the island: its views are built in the chosen language.
  const boot = await Bridge.boot();
  if (boot) {
    State.settings = { ...State.settings, ...boot.settings };
  }
  const builtIn = lang();
  document.documentElement.lang = builtIn;

  // Back from a language change: pick up where the last page left off.
  const carry = takeCarry();
  if (carry) {
    State.paused = carry.paused;
    for (const session of carry.sessions) State.liveSessions.set(session.id, session);
    State.integrations = carry.integrations;
    State.chatHistory = carry.chat;
    State.droppedFile = carry.file;
  }

  const island = new Island(root);
  island.applySettings();
  State.loadIntegrationTasks();
  if (boot && !boot.cursorPoll) island.followPageCursor();

  await onEvent<{ x: number; y: number }>("cursor", ({ x, y }) => island.onCursor(x, y));

  // Where Rust hangs the island inside its window: now (it placed the window
  // before this page loaded), after every move, and while it is dragged.
  const anchor = await Bridge.islandAnchor();
  if (anchor) island.onAnchor(anchor);
  await onEvent<IslandAnchor>("island-anchor", (a) => island.onAnchor(a));
  await onEvent<{ dock: Dock | null }>("island-drag-dock", ({ dock }) => island.onDragDock(dock));
  await onEvent<null>("island-drag-end", () => island.onDragEnd());

  // An action the chat wants to take with a connected service (an email, an
  // issue, a Notion page, an n8n workflow) waits in Rust until Allow or Deny on
  // the card. One card at a time: an ask that arrives while another approval
  // is up waits its turn.
  type ToolAsk = { requestId: string; title: string; detail: string };
  const toolAsks: ToolAsk[] = [];
  const showToolAsk = () => {
    if (State.pendingApproval || toolAsks.length === 0) return;
    const ask = toolAsks.shift()!;
    State.pendingApproval = { requestId: ask.requestId, sessionId: "", tool: ask.title, command: ask.detail, source: "frank" };
    State.isPinned = true;
    Sound.play("approval");
    island.alert("approval");
  };
  State.subscribe(showToolAsk);
  await onEvent<ToolAsk>("tool-approval", (ask) => {
    toolAsks.push(ask);
    showToolAsk();
  });
  // Unanswered in time: Rust has refused it already, so the card goes.
  await onEvent<string>("tool-approval-closed", (id) => {
    const queued = toolAsks.findIndex((a) => a.requestId === id);
    if (queued >= 0) toolAsks.splice(queued, 1);
    if (State.pendingApproval?.requestId !== id) return;
    State.pendingApproval = null;
    State.isPinned = false;
    island.dropPin();
    island.setView("prompt");
  });

  /** Pause has to reach Rust too, or the pollers keep calling out. */
  const setPaused = (on: boolean) => {
    if (State.paused === on) return;
    State.paused = on;
    void Bridge.setPaused(on);
  };

  await onEvent<string>("tray", (what) => {
    switch (what) {
      case "settings":
        setPaused(false);
        island.alert("settings");
        break;
      case "open":
        setPaused(false);
        island.alert(State.defaultView());
        break;
      case "pause":
        setPaused(!State.paused);
        if (State.paused) island.fsm.forceHidden();
        else island.reveal();
        break;
    }
  });

  await onEvent<null>("screen-changed", () => void Bridge.reposition());

  // The settings window writes preferences; apply them here without a restart.
  await onEvent<Settings>("settings-changed", (s) => {
    State.settings = { ...State.settings, ...s };
    if (lang() !== builtIn) {
      void reloadForLanguage();
      return;
    }
    island.applySettings();
    State.loadIntegrationTasks();
    void refreshConfigured();
  });

  registerHookHandlers(island);
  registerIntegrationHandlers(island);

  // The wake word: hearing it opens the chat and starts a spoken conversation.
  const wake = new WakeListener((command) => {
    Sound.play("pop");
    State.voiceWake = { command };
    island.alert("prompt");
    State.notify();
  });
  let held = false;
  const syncWake = () => {
    const want = State.settings.wakeEnabled && !State.paused;
    if (want && !wake.running) {
      wake.start().catch((err) => void Bridge.log(`wake: microphone unavailable: ${String(err)}`));
    } else if (!want && wake.running) {
      wake.stop();
    }
    // A conversation (or one about to start) has the microphone; the wake
    // listener steps aside, and comes back when it is over.
    const nowHeld = State.voiceActive || State.voiceWake != null;
    if (nowHeld !== held) {
      held = nowHeld;
      if (held) wake.pause();
      else wake.resume();
    }
  };
  State.subscribe(syncWake);
  syncWake();

  // The last page may have left the window shrunk to the wake strip.
  if (carry) island.resumeWindow(State.paused);
  // Paused stays paused across a reload: out of sight, no greeting.
  if (!State.paused) island.launch();

  // In a plain browser there is no wake strip behind the cursor: make the whole
  // page wake the island so the visuals can be checked with `npm run dev`.
  if (!IS_TAURI) {
    document.addEventListener("click", () => Sound.resume(), { once: true });
  }
}

void main();
