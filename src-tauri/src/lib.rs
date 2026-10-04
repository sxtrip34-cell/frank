// Frank for Windows — app wiring and the commands the island calls.

mod claude;
mod claude_cli;
mod files;
mod hooks;
mod i18n;
mod integrations;
mod island;
mod log;
mod office;
mod pipe;
mod placement;
mod platform;
mod secrets;
mod sessions;
mod settings;
mod tools;
mod tray;
mod voice;

use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_autostart::{ManagerExt, MacosLauncher};

use claude::{Chat, ChatContext, ChatReply};
use claude_cli::CliChat;
use files::DroppedFile;
use hooks::{HookPreview, HookStatus};
use island::{PollGate, ScreenInfo};
use pipe::Pending;
use settings::Settings;

pub struct Shared {
    pub settings: Mutex<Settings>,
    pub gate: Arc<PollGate>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootInfo {
    settings: Settings,
    screen: ScreenInfo,
    version: String,
    hook_path: String,
    /// False where the OS has no global cursor (Wayland): the page then reports
    /// the cursor from its own mouse events.
    cursor_poll: bool,
}

#[tauri::command]
fn boot(app: AppHandle, shared: State<Shared>) -> BootInfo {
    let mut settings = shared.settings.lock().unwrap().clone();
    // The real state of ~/.claude/settings.json wins over whatever we stored.
    settings.hooks_installed = hooks::status().installed;
    let screen = island::screen_info(&app, &settings.screen);
    BootInfo {
        settings,
        screen,
        version: env!("CARGO_PKG_VERSION").to_string(),
        hook_path: settings::hook_exe_path().to_string_lossy().to_string(),
        cursor_poll: platform::CURSOR_POLL,
    }
}

#[tauri::command]
fn save_settings(app: AppHandle, shared: State<Shared>, settings: Settings) {
    let (screen_changed, autostart_changed, language_changed) = {
        let mut current = shared.settings.lock().unwrap();
        // A new display, or the island sent back to the top: place it again.
        let screen_changed =
            current.screen != settings.screen || current.island_pos != settings.island_pos;
        let autostart_changed = current.autostart != settings.autostart;
        let language_changed = current.language != settings.language;
        *current = settings.clone();
        (screen_changed, autostart_changed, language_changed)
    };
    if let Err(err) = settings::save(&settings) {
        eprintln!("[frank] could not save settings: {err}");
    }
    if autostart_changed {
        let manager = app.autolaunch();
        let result = if settings.autostart { manager.enable() } else { manager.disable() };
        if let Err(err) = result {
            eprintln!("[frank] autostart: {err}");
        }
    }
    if screen_changed {
        let collapsed = shared.gate.collapsed.load(Ordering::Relaxed);
        island::apply_geometry(&app, &settings.screen, collapsed, settings.island_pos);
    }
    // The pages reload themselves in the new language; Rust's own words follow here.
    if language_changed {
        let lang = i18n::Lang::from_setting(&settings.language);
        tray::relabel(&app, lang);
        if let Some(win) = app.get_webview_window("settings") {
            let _ = win.set_title(i18n::texts(lang).settings_title);
        }
    }
    // Keep the other window in step (island ⇄ settings window).
    let _ = app.emit("settings-changed", settings);
}

/// Hidden island → shrink the window to the invisible wake strip and park the
/// cursor poll; anything else → full panel and 60 Hz polling.
#[tauri::command]
fn set_collapsed(app: AppHandle, shared: State<Shared>, collapsed: bool) {
    let (pref, pos) = {
        let s = shared.settings.lock().unwrap();
        (s.screen.clone(), s.island_pos)
    };
    shared.gate.collapsed.store(collapsed, Ordering::Relaxed);
    island::apply_geometry(&app, &pref, collapsed, pos);
    // The wake strip must always take the mouse, and a resize invalidates the flag.
    island::refresh_click_through(&app, &shared.gate);
    shared.gate.set_active(!collapsed);
}

/// The front end pushes the island shape; Rust decides click-through from it.
#[tauri::command]
fn set_island_rect(app: AppHandle, shared: State<Shared>, x: f64, y: f64, width: f64, height: f64) {
    shared.gate.set_rect(island::IslandRect { x, y, w: width, h: height });
    // Without the cursor poll the input region is the click-through: it follows the island.
    if !platform::CURSOR_POLL {
        island::refresh_click_through(&app, &shared.gate);
    }
}

#[tauri::command]
fn focus_window(app: AppHandle, focused: bool) {
    let Some(win) = island::window(&app) else { return };
    platform::set_activating(&win, focused);
    if focused {
        let _ = win.set_focus();
    }
}

#[tauri::command]
fn reposition(app: AppHandle, shared: State<Shared>) {
    let (pref, pos) = {
        let s = shared.settings.lock().unwrap();
        (s.screen.clone(), s.island_pos)
    };
    let collapsed = shared.gate.collapsed.load(Ordering::Relaxed);
    island::apply_geometry(&app, &pref, collapsed, pos);
}

/// The user pressed on the island and moved: it follows the mouse until the
/// button is let go (island::spawn_drag). `grab_x`/`grab_y` is where it was
/// taken hold of, in logical pixels from the island's top-left corner.
#[tauri::command]
fn island_drag_begin(app: AppHandle, grab_x: f64, grab_y: f64) {
    island::spawn_drag(app, (grab_x, grab_y));
}

/// Where the island hangs inside the window, for a page that loaded after
/// Rust last placed it.
#[tauri::command]
fn island_anchor(shared: State<Shared>) -> island::Anchor {
    *shared.gate.anchor.lock().unwrap()
}

#[tauri::command]
fn open_url(url: String) {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return;
    }
    platform::open_url(&url);
}

/// "Open terminal" opens the working folder in VS Code when `code` is on PATH,
/// and falls back to the file manager otherwise.
#[tauri::command]
fn open_in_vscode(path: Option<String>) -> bool {
    // No shell anywhere near this. The path is a project folder chosen by
    // whoever is using Claude Code, and a shell would happily read `&`, `^`, `%`
    // or `$` in a folder name as syntax. Finding the launcher ourselves and
    // handing the path over as a separate argument keeps it a path.
    let path = path.filter(|p| !p.is_empty());
    // It arrives in a hook payload: only an existing folder, given by its full
    // path, goes any further. `code` would read `--something` as an option, and
    // xdg-open would launch a file with whatever handles its type.
    if let Some(p) = path.as_deref() {
        let p = std::path::Path::new(p);
        if !(p.is_absolute() && p.is_dir()) {
            return false;
        }
    }
    if let Some(code) = platform::find_on_path("code") {
        let mut cmd = Command::new(code);
        if let Some(p) = path.as_deref() {
            cmd.arg(p);
        }
        if platform::no_console(&mut cmd).spawn().is_ok() {
            return true;
        }
    }
    if let Some(p) = path.as_deref() {
        platform::reveal_folder(p);
    }
    false
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

/// Tray → Pause. Paused means paused: the pollers stop talking to the network,
/// not just the island stopping showing things.
#[tauri::command]
fn set_paused(paused: bool) {
    integrations::set_paused(paused);
}

// ── Claude Code hooks ─────────────────────────────────────────────────────────

#[tauri::command]
fn hooks_status() -> HookStatus {
    hooks::status()
}

/// Returns the diff the user has to look at before anything is written.
#[tauri::command]
fn hooks_preview(install: bool) -> Result<HookPreview, String> {
    hooks::preview(install)
}

/// Only ever called from an explicit click in the settings window.
#[tauri::command]
fn hooks_apply(
    app: AppHandle,
    shared: State<Shared>,
    install: bool,
    fingerprint: String,
) -> Result<String, String> {
    // The fingerprint comes from the preview the user actually looked at, so a
    // settings.json that changed in between is refused rather than overwritten.
    let backup = hooks::write(install, &fingerprint)?;
    let updated = {
        let mut current = shared.settings.lock().unwrap();
        current.hooks_installed = install;
        let _ = settings::save(&current);
        current.clone()
    };
    let _ = app.emit("settings-changed", updated);
    Ok(backup)
}

#[tauri::command]
fn approval_decision(app: AppHandle, request_id: String, decision: String) {
    pipe::answer(&app, &request_id, &decision);
}

/// The island has the card on screen, so the long wait for a human may begin.
/// Until this arrives the relay only waits a few hundred milliseconds, which is
/// what stops a paused or unresponsive island from freezing Claude Code.
#[tauri::command]
fn approval_ack(app: AppHandle, request_id: String) {
    pipe::acknowledge(&app, &request_id);
}

/// Nobody can act on this request — the island is paused, or another card is
/// already up. Claude Code falls back to asking in the terminal immediately.
#[tauri::command]
fn approval_decline(app: AppHandle, request_id: String) {
    pipe::decline(&app, &request_id);
}

// ── Chat, files and secrets ───────────────────────────────────────────────────

/// Asked of every spoken turn: the answer is going to be read aloud.
const VOICE_HINT: &str = "(Spoken conversation: your answer will be read aloud. \
Reply in one to three short, natural sentences. No lists, links, emojis or symbols.)";

/// One chat turn. The API key and any file bytes stay on the Rust side.
/// Answered by the Claude Code CLI or the Anthropic API, as chosen in settings.
/// `sessions` are the coding sessions the island has heard from: the chat is
/// told about them, and through the CLI may read (never write) their folders.
#[tauri::command]
async fn chat_send(
    shared: State<'_, Shared>,
    chat: State<'_, Chat>,
    cli_chat: State<'_, CliChat>,
    query: String,
    context: Option<ChatContext>,
    voice: Option<bool>,
    sessions: Option<Vec<sessions::SessionNote>>,
) -> Result<ChatReply, String> {
    let voice = voice.unwrap_or(false);
    let (provider, model, cli_model) = {
        let s = shared.settings.lock().unwrap();
        let cli = if voice { s.cli_voice_model.clone() } else { s.cli_model.clone() };
        (s.chat_provider.clone(), s.model.clone(), cli)
    };
    let briefing = sessions::brief(sessions.unwrap_or_default(), sessions::claude_projects(), sessions::desktop_folders()).await;

    let mut prompt = String::new();
    if voice {
        prompt.push_str(VOICE_HINT);
        prompt.push_str("\n\n");
    }
    if let Some(b) = &briefing {
        prompt.push_str(&b.text);
        prompt.push('\n');
    }
    prompt.push_str(&query);

    if provider == "anthropic-api" {
        claude::send(&chat, &model, prompt, context).await
    } else {
        let dirs = briefing.map(|b| b.dirs).unwrap_or_default();
        claude_cli::send(&cli_chat, &cli_model, prompt, context, &dirs).await
    }
}

#[tauri::command]
fn chat_reset(chat: State<Chat>, cli_chat: State<CliChat>) {
    chat.reset();
    cli_chat.reset();
}

/// Copies a dropped file into the inbox and reports its name back.
#[tauri::command]
fn ingest_file(path: String) -> Result<DroppedFile, String> {
    files::ingest(&path)
}

/// The chat's paperclip: Windows' Open dialog, then the chosen file copied into
/// the inbox. None when the dialog is cancelled.
#[tauri::command]
async fn pick_file(title: String) -> Result<Option<DroppedFile>, String> {
    tauri::async_runtime::spawn_blocking(move || match platform::pick_file(&title) {
        Some(path) => files::ingest(&path.to_string_lossy()).map(Some),
        None => Ok(None),
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Ctrl+V in the chat after copying a file in Explorer: the first copied file,
/// into the inbox. None when the clipboard holds no files.
#[tauri::command]
async fn paste_file() -> Result<Option<DroppedFile>, String> {
    tauri::async_runtime::spawn_blocking(|| match platform::clipboard_files().into_iter().next() {
        Some(path) => files::ingest(&path.to_string_lossy()).map(Some),
        None => Ok(None),
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Whether whisper.cpp, Piper and their models are in place.
#[tauri::command]
fn voice_status() -> voice::VoiceStatus {
    voice::status()
}

/// Starts the voice helpers ahead of the first utterance. Returns at once.
#[tauri::command]
fn voice_warm() {
    tauri::async_runtime::spawn(voice::warm_up());
}

/// Speech → text, on this machine. The WAV arrives as the raw request body.
#[tauri::command]
async fn voice_transcribe(request: tauri::ipc::Request<'_>) -> Result<String, String> {
    let wav = match request.body() {
        tauri::ipc::InvokeBody::Raw(bytes) => bytes.clone(),
        _ => return Err("Expected a WAV recording.".into()),
    };
    voice::transcribe(wav).await
}

/// Text → speech, on this machine. The WAV goes back as raw bytes.
#[tauri::command]
async fn voice_speak(text: String) -> Result<tauri::ipc::Response, String> {
    voice::speak(&text).await.map(tauri::ipc::Response::new)
}

/// Saves an image pasted into the chat. The PNG arrives as the raw request
/// body, so a screenshot never goes through JSON as a list of numbers.
#[tauri::command]
fn ingest_image(request: tauri::ipc::Request<'_>) -> Result<DroppedFile, String> {
    match request.body() {
        tauri::ipc::InvokeBody::Raw(bytes) => files::ingest_image(bytes),
        _ => Err("Expected image bytes.".into()),
    }
}

/// Allow or Deny on the island for an action the chat wants to take with one
/// of the connected services (tools.rs waits for it).
#[tauri::command]
fn tool_decision(request_id: String, allow: bool) {
    tools::decide(&request_id, allow);
}

/// Saves a file dropped on the island: its bytes as the raw request body, its
/// name (URI-encoded) in the `x-file-name` header.
#[tauri::command]
fn ingest_dropped(request: tauri::ipc::Request<'_>) -> Result<DroppedFile, String> {
    let name = request
        .headers()
        .get("x-file-name")
        .and_then(|v| v.to_str().ok())
        .map(percent_decode)
        .unwrap_or_default();
    match request.body() {
        tauri::ipc::InvokeBody::Raw(bytes) => files::ingest_bytes(&name, bytes),
        _ => Err("Expected the file's bytes.".into()),
    }
}

/// Undoes encodeURIComponent: a header carries only ASCII.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = || std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex()) {
            (b'%', Some(b)) => {
                out.push(b);
                i += 3;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The island may only ask whether a key exists — never read it.
#[tauri::command]
fn secret_present(key: String) -> bool {
    secrets::present(&key)
}

#[tauri::command]
fn secret_set(key: String, value: String) -> Result<(), String> {
    secrets::set(&key, &value)
}

#[tauri::command]
fn secret_clear(key: String) -> Result<(), String> {
    secrets::clear(&key)
}

/// Opens the configured n8n instance — the URL lives in the Credential Manager.
#[tauri::command]
fn open_n8n() {
    if let Some(url) = secrets::get("n8n-url") {
        open_url(url);
    }
}

/// Refresh buttons in the integration cards.
#[tauri::command]
async fn refresh_integration(app: AppHandle, id: String) {
    integrations::poll_once(app, &id).await;
}

/// Lets the island write to the same log as the Rust side.
#[tauri::command]
fn log_line(message: String) {
    log::line(format!("ui  {message}"));
}

// ── Settings window ───────────────────────────────────────────────────────────

/// WebView2 allows exactly one browser environment per app, and its options are
/// fixed by whichever webview is created first. Every window must therefore ask
/// for the *same* arguments as the island (see `additionalBrowserArgs` in
/// tauri.conf.json) — a mismatch makes the second window come up blank, with no
/// error anywhere.
const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required";

/// In a dev build the pages are served by Vite, so the second window needs the
/// absolute dev URL; a bundled build resolves it inside the app bundle.
fn settings_page_url(app: &AppHandle) -> WebviewUrl {
    #[cfg(dev)]
    if let Some(mut base) = app.config().build.dev_url.clone() {
        base.set_path("/settings.html");
        return WebviewUrl::External(base);
    }
    let _ = app;
    WebviewUrl::App("settings.html".into())
}

/// The settings window is created hidden at launch and only ever shown and
/// hidden afterwards. A WebView2 window created later — on the main thread or
/// not — silently comes up blank in this app, so the window that works is the
/// one that exists before the island's webview does.
fn create_settings_window(app: &AppHandle, lang: i18n::Lang) {
    let url = settings_page_url(app);
    match WebviewWindowBuilder::new(app, "settings", url)
        .additional_browser_args(BROWSER_ARGS)
        .title(i18n::texts(lang).settings_title)
        .inner_size(560.0, 680.0)
        .min_inner_size(460.0, 480.0)
        .resizable(true)
        .visible(false)
        .center()
        .build()
    {
        Ok(win) => {
            // Closing it must only hide it, or it could never be reopened.
            let hidden = win.clone();
            win.on_window_event(move |event| {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = hidden.hide();
                }
            });
        }
        Err(err) => log::line(format!("settings window failed: {err}")),
    }
}

pub fn show_settings_window(app: &AppHandle) {
    let Some(win) = app.get_webview_window("settings") else {
        log::line("settings window missing");
        return;
    };
    let _ = win.unminimize();
    let _ = win.show();
    let _ = win.set_focus();
}

#[tauri::command]
fn open_settings_window(app: AppHandle) {
    show_settings_window(&app);
}

pub fn run() {
    platform::prepare_environment();
    let loaded = settings::load();
    let gate = Arc::new(PollGate::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            let _ = app.emit_to(island::WINDOW_LABEL, "tray", "open".to_string());
        }))
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        // The island's voice chat listens on the microphone. Granted here, for
        // the island only: WebView2's own prompt would pop up over a window
        // that never takes focus. Everything else keeps the default.
        .on_permission_request(|webview, kind| match kind {
            tauri::webview::PermissionKind::Microphone if webview.label() == island::WINDOW_LABEL => {
                tauri::webview::PermissionResponse::Allow
            }
            _ => tauri::webview::PermissionResponse::Default,
        })
        .manage(Shared {
            settings: Mutex::new(loaded.clone()),
            gate: gate.clone(),
        })
        .manage(Pending::default())
        .manage(Chat::default())
        .manage(CliChat::default())
        .invoke_handler(tauri::generate_handler![
            boot,
            save_settings,
            set_collapsed,
            set_island_rect,
            focus_window,
            reposition,
            island_drag_begin,
            island_anchor,
            open_url,
            open_in_vscode,
            quit_app,
            hooks_status,
            hooks_preview,
            hooks_apply,
            approval_decision,
            approval_ack,
            approval_decline,
            log_line,
            chat_send,
            chat_reset,
            ingest_file,
            ingest_dropped,
            tool_decision,
            pick_file,
            paste_file,
            ingest_image,
            voice_status,
            voice_warm,
            voice_transcribe,
            voice_speak,
            secret_present,
            secret_set,
            secret_clear,
            refresh_integration,
            open_n8n,
            open_settings_window,
            set_paused,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let lang = i18n::Lang::from_setting(&loaded.language);
            tray::build(&handle, lang)?;
            // Before the island: see create_settings_window.
            create_settings_window(&handle, lang);

            if let Some(win) = island::window(&handle) {
                platform::make_non_activating(&win);
                island::apply_geometry(&handle, &loaded.screen, false, loaded.island_pos);
                let _ = win.show();
            }
            gate.collapsed.store(false, Ordering::Relaxed);
            // Nothing drawn yet, so nothing takes the mouse until the page
            // reports the island's shape.
            if !platform::CURSOR_POLL {
                island::refresh_click_through(&handle, &gate);
            }
            gate.set_active(true);
            island::spawn_cursor_poll(handle.clone(), gate.clone());

            log::line(format!("--- Frank {} started ---", env!("CARGO_PKG_VERSION")));
            // Frank's own folder, with his memory: there from the start, so the
            // user can find it before the first chat.
            let memory = platform::frank_home().join("memory");
            if let Err(err) = std::fs::create_dir_all(&memory) {
                log::line(format!("could not create {}: {err}", memory.display()));
            }
            hooks::ensure_hook_exe(&handle);
            pipe::start(handle.clone());
            integrations::start(handle.clone());
            tools::start(handle.clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Frank");
}
