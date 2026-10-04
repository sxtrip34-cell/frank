// Notification-area icon: Open, Settings, Pause, Quit — in the interface language.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, Wry};

use crate::i18n::{self, Lang};
use crate::island::WINDOW_LABEL;

/// The menu items, kept so their labels can follow the language setting.
pub struct TrayItems {
    open: MenuItem<Wry>,
    settings: MenuItem<Wry>,
    pause: MenuItem<Wry>,
    quit: MenuItem<Wry>,
}

pub fn build(app: &AppHandle, lang: Lang) -> tauri::Result<()> {
    let t = i18n::texts(lang);
    let open = MenuItem::with_id(app, "open", t.open, true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", t.settings, true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause", t.pause, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", t.quit, true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;

    let menu = Menu::with_items(app, &[&open, &sep1, &settings, &pause, &sep2, &quit])?;

    let mut builder = TrayIconBuilder::with_id("frank")
        .tooltip("Frank")
        .menu(&menu)
        .on_menu_event(|app: &AppHandle, event| match event.id.as_ref() {
            "quit" => app.exit(0),
            "settings" => crate::show_settings_window(app),
            id => {
                let _ = app.emit_to(WINDOW_LABEL, "tray", id.to_string());
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }

    builder.build(app)?;
    app.manage(TrayItems { open, settings, pause, quit });
    Ok(())
}

/// Relabels the menu after the language setting changed.
pub fn relabel(app: &AppHandle, lang: Lang) {
    let Some(items) = app.try_state::<TrayItems>() else { return };
    let t = i18n::texts(lang);
    let _ = items.open.set_text(t.open);
    let _ = items.settings.set_text(t.settings);
    let _ = items.pause.set_text(t.pause);
    let _ = items.quit.set_text(t.quit);
}
