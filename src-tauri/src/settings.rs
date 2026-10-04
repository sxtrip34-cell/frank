// Preferences, stored as plain JSON in settings.json under platform::config_dir().
// No secret ever lands here — API keys live in the OS keychain (see secrets.rs).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub sound_enabled: bool,
    pub sound_volume: f64,
    pub auto_close_interval: f64,
    pub absence_interval: f64,
    pub active_integrations: Vec<String>,
    /// "primary" = the main display, "cursor" = whichever display the mouse is on.
    pub screen: String,
    pub autostart: bool,
    pub hooks_installed: bool,
    /// Claude model used by the chat. Changeable in the settings window.
    /// Defaulted explicitly so a settings.json written by an older build still loads.
    #[serde(default = "default_model")]
    pub model: String,
    /// What answers the chat: "claude-code" (the signed-in Claude Code CLI, no
    /// API key) or "anthropic-api" (the key in the Credential Manager).
    #[serde(default = "default_chat_provider")]
    pub chat_provider: String,
    /// Model alias passed to the Claude Code CLI; "default" leaves it to the CLI.
    #[serde(default = "default_cli_model")]
    pub cli_model: String,
    /// The model for spoken turns: a quick answer matters more there.
    #[serde(default = "default_cli_voice_model")]
    pub cli_voice_model: String,
    /// Hands-free voice chat: the island listens for `wake_word` and starts a
    /// spoken conversation when it hears it. Off unless the user turns it on.
    #[serde(default)]
    pub wake_enabled: bool,
    #[serde(default = "default_wake_word")]
    pub wake_word: String,
    /// Interface language: "en", "tr", "ru", or "auto" for the system's.
    #[serde(default = "default_language")]
    pub language: String,
    /// Where the user dragged the island, or None for its place at the top
    /// centre of the screen.
    #[serde(default)]
    pub island_pos: Option<IslandPos>,
}

/// Where the island sits, in logical pixels from the top-left of the display's
/// work area (the screen minus the taskbar): `x` is the island's centre, `y` its
/// top. Docked to the top edge only `x` counts, docked to a side only `y`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct IslandPos {
    pub x: f64,
    pub y: f64,
    /// Positions saved before docking existed are free ones.
    #[serde(default)]
    pub dock: Dock,
}

/// The edge the island is attached to, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dock {
    Top,
    Left,
    Right,
    #[default]
    Free,
}

fn default_wake_word() -> String {
    "Frank".into()
}

fn default_language() -> String {
    "auto".into()
}

fn default_cli_voice_model() -> String {
    "haiku".into()
}

fn default_model() -> String {
    crate::claude::DEFAULT_MODEL.to_string()
}

fn default_chat_provider() -> String {
    "claude-code".into()
}

fn default_cli_model() -> String {
    crate::claude_cli::DEFAULT_MODEL.to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            sound_enabled: true,
            sound_volume: 0.12,
            auto_close_interval: 15.0,
            absence_interval: 180.0,
            // None until the user turns one on and gives it a key.
            active_integrations: Vec::new(),
            screen: "primary".into(),
            autostart: false,
            hooks_installed: false,
            model: default_model(),
            chat_provider: default_chat_provider(),
            cli_model: default_cli_model(),
            cli_voice_model: default_cli_voice_model(),
            wake_enabled: false,
            wake_word: default_wake_word(),
            language: default_language(),
            island_pos: None,
        }
    }
}

pub use crate::platform::{config_dir, local_dir};

pub fn hook_exe_path() -> PathBuf {
    local_dir().join("bin").join(crate::platform::HOOK_EXE)
}

fn settings_path() -> PathBuf {
    config_dir().join("settings.json")
}

pub fn load() -> Settings {
    match std::fs::read(settings_path()) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => Settings::default(),
    }
}

pub fn save(settings: &Settings) -> std::io::Result<()> {
    let dir = config_dir();
    crate::platform::ensure_private_dir(&dir)?;
    let json = serde_json::to_vec_pretty(settings)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(settings_path(), json)
}
