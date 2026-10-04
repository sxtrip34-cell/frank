// Settings window — the place where anything that writes to disk is confirmed.
// Stage 2 covers the Claude Code hooks and the general preferences; API keys and
// integrations land here too in a later stage.

import "./settings.css";
import { Bridge, onEvent, type HookStatus } from "../core/bridge";
import { lang, t, type MessageKey } from "../core/i18n";
import { DEFAULT_SETTINGS, State, type Settings } from "../core/state";
import { h, clear } from "../views/dom";

let settings: Settings = { ...DEFAULT_SETTINGS };
let version = "";

const root = document.getElementById("settings-root")!;

async function save() {
  await Bridge.saveSettings(settings);
}

// ── Reusable bits ─────────────────────────────────────────────────────────────

function toggle(on: boolean, onChange: (v: boolean) => void): HTMLElement {
  const el = h("button", { class: on ? "switch on" : "switch", "aria-pressed": on });
  el.addEventListener("click", () => {
    const next = !el.classList.contains("on");
    el.classList.toggle("on", next);
    onChange(next);
  });
  return el;
}

function statusDot(ok: boolean): HTMLElement {
  return h("i", { class: "dot", style: `background:${ok ? "#22c55e" : "#f4505e"}` });
}

function renderDiff(text: string): HTMLElement {
  const box = h("div", { class: "diff" });
  for (const line of text.split("\n")) {
    const cls = line.startsWith("+") ? "add" : line.startsWith("-") ? "del" : "ctx";
    box.append(h("div", { class: cls, text: line }));
  }
  return box;
}

// ── Claude Code section ───────────────────────────────────────────────────────

function claudeSection(status: HookStatus): HTMLElement {
  const body = h("div", { style: "display:flex;flex-direction:column;gap:12px" });
  const section = h(
    "section",
    {},
    h("h2", {}, statusDot(status.installed), h("span", { text: "Claude Code" })),
    body,
  );

  const rebuild = async () => {
    const fresh = await Bridge.hooksStatus();
    if (fresh) Object.assign(status, fresh);
    clear(body);
    draw();
    const head = section.querySelector("h2")!;
    clear(head);
    head.append(statusDot(status.installed), h("span", { text: "Claude Code" }));
  };

  function draw() {
    body.append(
      h("div", {
        class: "hint",
        text: t(status.installed ? "settings.claude.installed" : "settings.claude.notInstalled"),
      }),
      h("div", { class: "row" },
        h("label", { text: "settings.json" }),
        h("span", { class: "path", text: status.settingsPath }),
      ),
      h("div", { class: "row" },
        h("label", { text: t("settings.claude.relay") }),
        h("span", { class: "path", text: status.hookPath }),
        statusDot(status.hookReady),
      ),
    );

    if (!status.hookReady) {
      body.append(h("div", {
        class: "notice warn",
        text: t("settings.claude.relayMissing"),
      }));
    }

    const actions = h("div", { class: "row" });
    const install = h("button", {
      class: "primary",
      text: t(status.installed ? "settings.claude.reinstall" : "settings.claude.install"),
      onclick: () => showPreview(true),
    });
    // Writing hook commands that point at a relay which isn't there would give
    // every Claude Code session a broken hook and nothing to show for it.
    if (!status.hookReady) {
      install.disabled = true;
      install.title = t("settings.claude.relayNotReady");
    }
    actions.append(install);
    if (status.installed) {
      actions.append(h("button", {
        class: "danger",
        text: t("settings.claude.uninstall"),
        onclick: () => showPreview(false),
      }));
    }
    body.append(actions);
  }

  async function showPreview(install: boolean) {
    let preview;
    try {
      preview = await Bridge.hooksPreview(install);
    } catch (err) {
      // An unreadable or invalid settings.json stops here rather than being
      // treated as empty and written over.
      clear(body);
      body.append(
        h("div", { class: "notice err", text: String(err).replace(/^Error:\s*/, "") }),
        h("div", { class: "row" }, h("button", {
          text: t("settings.back"),
          onclick: () => { clear(body); draw(); },
        })),
      );
      return;
    }
    if (!preview) return;
    clear(body);
    body.append(
      h("div", {
        class: "hint",
        text: t(install ? "settings.claude.previewInstall" : "settings.claude.previewRemove"),
      }),
      renderDiff(preview.diff),
      h("div", { class: "row" },
        h("span", { class: "path", text: t("settings.claude.backup", { path: preview.backup }) }),
      ),
    );
    const confirm = h("button", {
      class: install ? "primary" : "danger",
      text: t(install ? "settings.claude.confirmInstall" : "settings.claude.confirmRemove"),
    });
    confirm.addEventListener("click", async () => {
      confirm.disabled = true;
      try {
        const backup = await Bridge.hooksApply(install, preview.fingerprint);
        clear(body);
        body.append(h("div", {
          class: "notice ok",
          text: t("settings.claude.done", { backup }),
        }));
        window.setTimeout(() => void rebuild(), 2600);
      } catch (err) {
        confirm.disabled = false;
        body.append(h("div", { class: "notice err", text: t("settings.claude.writeFailed", { error: String(err) }) }));
      }
    });
    body.append(h("div", { class: "row" }, confirm, h("button", {
      text: t("common.cancel"),
      onclick: () => { clear(body); draw(); },
    })));
  }

  draw();
  return section;
}

// ── Claude API section ────────────────────────────────────────────────────────

const MODELS: [string, string][] = [
  ["claude-opus-5", "Claude Opus 5"],
  ["claude-sonnet-5", "Claude Sonnet 5"],
  ["claude-haiku-4-5", "Claude Haiku 4.5"],
];

const PROVIDERS: [Settings["chatProvider"], MessageKey][] = [
  ["claude-code", "settings.api.providerCli"],
  ["anthropic-api", "settings.api.providerKey"],
];

/** Aliases the Claude Code CLI understands; "default" leaves the choice to it. */
function cliModels(): [string, string][] {
  return [
    ["sonnet", "Sonnet"],
    ["opus", "Opus"],
    ["haiku", "Haiku"],
    ["default", t("settings.api.cliDefault")],
  ];
}

function apiSection(hasKey: boolean): HTMLElement {
  let keyPresent = hasKey;
  const dot = statusDot(hasKey);
  const state = h("span", { class: "hint" });
  const stored = `••••••••••••  ${t("settings.stored")}`;

  const field = h("input", {
    type: "password",
    placeholder: hasKey ? stored : "sk-ant-...",
    style: "flex:1 1 auto;min-width:0",
    autocomplete: "off",
    spellcheck: "false",
  }) as HTMLInputElement;

  const saveBtn = h("button", { class: "primary", text: t("settings.api.saveKey") });
  const clearBtn = h("button", { class: "danger", text: t("settings.remove") });
  const feedback = h("div", {});

  async function refresh() {
    const present = (await Bridge.secretPresent("anthropic-api-key")) ?? false;
    keyPresent = present;
    field.placeholder = present ? stored : "sk-ant-...";
    clearBtn.style.display = present ? "" : "none";
    paint();
  }

  saveBtn.addEventListener("click", async () => {
    const value = field.value.trim();
    if (!value) return;
    clear(feedback);
    try {
      await Bridge.secretSet("anthropic-api-key", value);
      field.value = "";
      feedback.append(h("div", { class: "notice ok", text: t("settings.api.saved") }));
      await refresh();
    } catch (err) {
      feedback.append(h("div", { class: "notice err", text: t("settings.api.saveFailed", { error: String(err) }) }));
    }
  });

  clearBtn.addEventListener("click", async () => {
    clear(feedback);
    try {
      await Bridge.secretClear("anthropic-api-key");
      feedback.append(h("div", { class: "notice ok", text: t("settings.api.removed") }));
      await refresh();
    } catch (err) {
      feedback.append(h("div", { class: "notice err", text: t("settings.api.removeFailed", { error: String(err) }) }));
    }
  });

  const model = h("select", {}) as HTMLSelectElement;
  for (const [id, label] of MODELS) model.append(h("option", { value: id, text: label }));
  if (!MODELS.some(([id]) => id === settings.model)) {
    model.append(h("option", { value: settings.model, text: settings.model }));
  }
  model.value = settings.model;
  model.addEventListener("change", () => {
    settings.model = model.value;
    void save();
  });

  clearBtn.style.display = hasKey ? "" : "none";

  const provider = h("select", {}) as HTMLSelectElement;
  for (const [id, label] of PROVIDERS) provider.append(h("option", { value: id, text: t(label) }));
  provider.value = settings.chatProvider;
  provider.addEventListener("change", () => {
    settings.chatProvider = provider.value as Settings["chatProvider"];
    paint();
    void save();
  });

  const cliChoices = cliModels();
  const cliModel = h("select", {}) as HTMLSelectElement;
  for (const [id, label] of cliChoices) cliModel.append(h("option", { value: id, text: label }));
  if (!cliChoices.some(([id]) => id === settings.cliModel)) {
    cliModel.append(h("option", { value: settings.cliModel, text: settings.cliModel }));
  }
  cliModel.value = settings.cliModel;
  cliModel.addEventListener("change", () => {
    settings.cliModel = cliModel.value;
    void save();
  });

  const voiceModel = h("select", {}) as HTMLSelectElement;
  for (const [id, label] of cliChoices) voiceModel.append(h("option", { value: id, text: label }));
  if (!cliChoices.some(([id]) => id === settings.cliVoiceModel)) {
    voiceModel.append(h("option", { value: settings.cliVoiceModel, text: settings.cliVoiceModel }));
  }
  voiceModel.value = settings.cliVoiceModel;
  voiceModel.addEventListener("change", () => {
    settings.cliVoiceModel = voiceModel.value;
    void save();
  });

  const cliModelRow = h("div", { style: "display:flex;flex-direction:column;gap:12px" },
    h("div", { class: "row" }, h("label", { text: t("settings.api.model") }), cliModel),
    h("div", { class: "row" },
      h("label", { text: t("settings.api.voiceModel") }),
      voiceModel,
      h("span", { class: "hint", text: t("settings.api.voiceModelHint") }),
    ),
  );
  const keyRow = h("div", { class: "row" }, h("label", { text: t("settings.api.key") }), field, saveBtn, clearBtn);
  const modelRow = h("div", { class: "row" }, h("label", { text: t("settings.api.model") }), model);

  /** Shows the rows that matter for the chosen provider. */
  function paint() {
    const viaCli = settings.chatProvider === "claude-code";
    dot.style.background = viaCli || keyPresent ? "#22c55e" : "#f4505e";
    state.textContent = t(viaCli
      ? "settings.api.viaCli"
      : keyPresent
        ? "settings.api.keySaved"
        : "settings.api.noKey");
    cliModelRow.style.display = viaCli ? "" : "none";
    keyRow.style.display = viaCli ? "none" : "";
    modelRow.style.display = viaCli ? "none" : "";
  }
  paint();

  return h(
    "section",
    {},
    h("h2", {}, dot, h("span", { text: "Claude" })),
    state,
    h("div", { class: "row" }, h("label", { text: t("settings.api.chatVia") }), provider),
    cliModelRow,
    keyRow,
    modelRow,
    feedback,
  );
}

// ── Integrations section ──────────────────────────────────────────────────────

interface IntegrationDef {
  id: string;
  name: string;
  color: string;
  /** Credential Manager keys, in the order they are shown. */
  fields: { key: string; label: MessageKey; placeholder: string; secret: boolean }[];
}

const INTEGRATIONS: IntegrationDef[] = [
  { id: "integration_stripe", name: "Stripe", color: "#0570DE",
    fields: [{ key: "stripe-api-key", label: "settings.integrations.secretKey", placeholder: "sk_live_…", secret: true }] },
  { id: "integration_github", name: "GitHub", color: "#F4505E",
    fields: [{ key: "github-token", label: "settings.integrations.token", placeholder: "ghp_…", secret: true }] },
  { id: "integration_vercel", name: "Vercel", color: "#7C5CFF",
    fields: [{ key: "vercel-token", label: "settings.integrations.token", placeholder: "…", secret: true }] },
  { id: "integration_n8n", name: "n8n", color: "#F29B38",
    fields: [
      { key: "n8n-url", label: "settings.integrations.instanceUrl", placeholder: "https://n8n.example.com", secret: false },
      { key: "n8n-api-key", label: "settings.api.key", placeholder: "…", secret: true },
    ] },
  { id: "integration_resend", name: "Resend", color: "#22C55E",
    fields: [{ key: "resend-api-key", label: "settings.api.key", placeholder: "re_…", secret: true }] },
  { id: "integration_notion", name: "Notion", color: "#8C8C8C",
    fields: [{ key: "notion-api-key", label: "settings.integrations.integrationToken", placeholder: "ntn_…", secret: true }] },
  { id: "integration_calcom", name: "Cal.com", color: "#C9956A",
    fields: [{ key: "calcom-api-key", label: "settings.api.key", placeholder: "cal_…", secret: true }] },
];

const MAX_ACTIVE = 4;

function integrationsSection(present: Record<string, boolean>): HTMLElement {
  const note = h("div", { class: "hint" });
  const list = h("div", { style: "display:flex;flex-direction:column;gap:14px" });
  const stored = `••••••••  ${t("settings.stored")}`;

  function updateNote() {
    const used = settings.activeIntegrations.length;
    note.textContent = t("settings.integrations.note", { max: MAX_ACTIVE, used });
  }

  for (const def of INTEGRATIONS) {
    const active = settings.activeIntegrations.includes(def.id);
    const sw = h("button", { class: active ? "switch on" : "switch" });
    sw.addEventListener("click", () => {
      const on = settings.activeIntegrations.includes(def.id);
      if (on) {
        settings.activeIntegrations = settings.activeIntegrations.filter((x) => x !== def.id);
      } else {
        if (settings.activeIntegrations.length >= MAX_ACTIVE) return;
        settings.activeIntegrations = [...settings.activeIntegrations, def.id];
      }
      sw.classList.toggle("on", !on);
      updateNote();
      void save();
    });

    const rows = h("div", { style: "display:flex;flex-direction:column;gap:6px;flex:1 1 auto;min-width:0" });
    for (const field of def.fields) {
      const input = h("input", {
        type: field.secret ? "password" : "text",
        placeholder: present[field.key] ? stored : field.placeholder,
        autocomplete: "off",
        spellcheck: "false",
        style: "flex:1 1 auto;min-width:0",
      }) as HTMLInputElement;
      const saveBtn = h("button", { text: t("settings.save") });
      const dotEl = statusDot(present[field.key] ?? false);
      saveBtn.addEventListener("click", async () => {
        const value = input.value.trim();
        try {
          await Bridge.secretSet(field.key, value);
          present[field.key] = value.length > 0;
          input.value = "";
          input.placeholder = value ? stored : field.placeholder;
          dotEl.style.background = value ? "#22c55e" : "#f4505e";
        } catch {
          dotEl.style.background = "#f5a524";
        }
      });
      rows.append(
        h("div", { class: "row" },
          h("label", { style: "min-width:104px", text: t(field.label) }),
          input, saveBtn, dotEl,
        ),
      );
    }

    list.append(
      h("div", { style: "display:flex;gap:12px;align-items:flex-start" },
        h("div", { style: "display:flex;align-items:center;gap:8px;min-width:132px;padding-top:4px" },
          sw,
          h("i", { class: "dot", style: `background:${def.color}` }),
          h("span", { style: "font-size:12.5px", text: def.name }),
        ),
        rows,
      ),
    );
  }

  updateNote();
  return h("section", {}, h("h2", {}, h("span", { text: t("settings.integrations.title") })), note, list);
}

// ── General section ───────────────────────────────────────────────────────────

function generalSection(): HTMLElement {
  const volume = h("input", {
    type: "range", min: "0", max: "0.2", step: "0.005",
    value: String(settings.soundVolume),
  }) as HTMLInputElement;
  volume.addEventListener("input", () => {
    settings.soundVolume = Number(volume.value);
    void save();
  });

  const autoClose = h("input", {
    type: "number", min: "5", max: "120", step: "1",
    value: String(Math.round(settings.autoCloseInterval)),
    style: "width:72px",
  }) as HTMLInputElement;
  autoClose.addEventListener("change", () => {
    settings.autoCloseInterval = Math.max(5, Math.min(120, Number(autoClose.value) || 15));
    autoClose.value = String(settings.autoCloseInterval);
    void save();
  });

  const screen = h("select", {}) as HTMLSelectElement;
  screen.append(
    h("option", { value: "primary", text: t("settings.general.screenPrimary") }),
    h("option", { value: "cursor", text: t("settings.general.screenCursor") }),
  );
  screen.value = settings.screen;
  screen.addEventListener("change", () => {
    settings.screen = screen.value as Settings["screen"];
    void save();
  });

  // Each language in its own words, so it can be found by whoever reads it.
  // Saving reloads both windows in the new language (see main below).
  const language = h("select", {}) as HTMLSelectElement;
  language.append(
    h("option", { value: "auto", text: t("settings.general.languageAuto") }),
    h("option", { value: "en", text: "English" }),
    h("option", { value: "tr", text: "Türkçe" }),
    h("option", { value: "ru", text: "Русский" }),
  );
  language.value = settings.language;
  language.addEventListener("change", () => {
    settings.language = language.value as Settings["language"];
    void save();
  });

  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: t("settings.general.title") })),
    h("div", { class: "row" },
      h("label", { text: t("settings.general.language") }),
      language,
    ),
    h("div", { class: "row" },
      h("label", { text: t("common.sound") }),
      toggle(settings.soundEnabled, (v) => { settings.soundEnabled = v; void save(); }),
      volume,
    ),
    h("div", { class: "row" },
      h("label", { text: t("settings.general.autoClose") }),
      autoClose,
      h("span", { class: "hint", text: t("settings.general.autoCloseHint") }),
    ),
    h("div", { class: "row" },
      h("label", { text: t("settings.general.screen") }),
      screen,
    ),
    h("div", { class: "row" },
      h("label", { text: t("settings.general.autostart") }),
      toggle(settings.autostart, (v) => { settings.autostart = v; void save(); }),
    ),
    wakeRow(),
    h("div", {
      class: "hint",
      text: t("settings.general.wakeHint"),
    }),
    positionRow(),
    h("div", {
      class: "hint",
      text: t("settings.general.positionHint"),
    }),
  );
}

/** Redraws the island position row when the island is dragged elsewhere. */
let refreshPosition: (() => void) | null = null;

function positionRow(): HTMLElement {
  const where = h("span", { class: "hint" });
  const reset = h("button", { text: t("settings.general.positionReset") });
  reset.addEventListener("click", () => {
    settings.islandPos = null;
    paint();
    void save();
  });
  function paint() {
    const pos = settings.islandPos;
    const dock = pos == null ? "top" : (pos.dock ?? "free");
    where.textContent = t(
      dock === "left" ? "settings.general.positionLeft"
      : dock === "right" ? "settings.general.positionRight"
      : dock === "free" ? "settings.general.positionFree"
      : "settings.general.positionTop",
    );
    // Its own place is the top centre: anywhere else can be sent back there.
    reset.style.display = pos != null ? "" : "none";
  }
  paint();
  refreshPosition = paint;
  return h("div", { class: "row" },
    h("label", { text: t("settings.general.position") }),
    where,
    reset,
  );
}

function wakeRow(): HTMLElement {
  const word = h("input", {
    type: "text",
    value: settings.wakeWord,
    placeholder: "Frank",
    autocomplete: "off",
    spellcheck: "false",
    style: "width:120px",
  }) as HTMLInputElement;
  word.addEventListener("change", () => {
    settings.wakeWord = word.value.trim() || "Frank";
    word.value = settings.wakeWord;
    void save();
  });
  return h("div", { class: "row" },
    h("label", { text: t("settings.general.wakeWord") }),
    toggle(settings.wakeEnabled, (v) => { settings.wakeEnabled = v; void save(); }),
    word,
  );
}

// ── Boot ──────────────────────────────────────────────────────────────────────

/** How far down the page was before a language change reloaded it. */
const SCROLL_KEY = "frank.settings.scroll";

async function main() {
  // Only ever seen right after a language change; the system language is the
  // best guess until the settings arrive.
  root.querySelector("p")?.replaceChildren(t("settings.loading"));

  const boot = await Bridge.boot();
  if (boot) {
    settings = { ...settings, ...boot.settings };
    version = boot.version;
  }
  // t() reads the language from State, here as in the island.
  State.settings.language = settings.language;
  const builtIn = lang();
  document.documentElement.lang = builtIn;
  document.title = t("settings.title");
  const status = (await Bridge.hooksStatus()) ?? {
    installed: false, settingsPath: "", hookPath: "", hookReady: false,
  };

  const hasKey = (await Bridge.secretPresent("anthropic-api-key")) ?? false;

  const keys = [
    "stripe-api-key", "github-token", "vercel-token",
    "n8n-url", "n8n-api-key", "resend-api-key", "notion-api-key", "calcom-api-key",
  ];
  const present: Record<string, boolean> = {};
  for (const k of keys) present[k] = (await Bridge.secretPresent(k)) ?? false;

  clear(root);
  root.append(
    h("h1", {}, h("span", { text: "Frank" }), h("span", { class: "version", text: version })),
    claudeSection(status),
    apiSection(hasKey),
    integrationsSection(present),
    generalSection(),
    h("div", {
      class: "hint",
      text: t("settings.privacy"),
    }),
  );

  try {
    const y = Number(sessionStorage.getItem(SCROLL_KEY));
    sessionStorage.removeItem(SCROLL_KEY);
    if (y > 0) window.scrollTo(0, y);
  } catch {
    // Nothing kept: the page starts at the top.
  }

  void onEvent<Settings>("settings-changed", (s) => {
    settings = { ...settings, ...s };
    State.settings.language = settings.language;
    refreshPosition?.();
    // Everything here is in the old language: build the page again, and come
    // back to the same place on it.
    if (lang() !== builtIn) {
      try {
        sessionStorage.setItem(SCROLL_KEY, String(window.scrollY));
      } catch {
        // Back at the top, then.
      }
      window.location.reload();
    }
  });
}

void main();
