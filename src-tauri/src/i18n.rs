// The few words Rust shows itself — the tray menu and the settings window title
// — in English, Turkish or Russian. Everything else is translated by the front
// end (src/core/i18n.ts), from the same `language` setting.

/// An interface language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Tr,
    Ru,
}

impl Lang {
    /// The `language` setting: "en", "tr" or "ru" as given; "auto" (or anything
    /// else) follows the system's own language.
    pub fn from_setting(value: &str) -> Lang {
        match value {
            "en" => Lang::En,
            "tr" => Lang::Tr,
            "ru" => Lang::Ru,
            _ => crate::platform::ui_language(),
        }
    }
}

/// Everything Rust labels, in one language.
pub struct Texts {
    pub open: &'static str,
    pub settings: &'static str,
    pub pause: &'static str,
    pub quit: &'static str,
    /// The settings window's title bar.
    pub settings_title: &'static str,
}

pub fn texts(lang: Lang) -> &'static Texts {
    match lang {
        Lang::En => &EN,
        Lang::Tr => &TR,
        Lang::Ru => &RU,
    }
}

static EN: Texts = Texts {
    open: "Open Frank",
    settings: "Settings…",
    pause: "Pause",
    quit: "Quit",
    settings_title: "Settings — Frank",
};

static TR: Texts = Texts {
    open: "Frank'i aç",
    settings: "Ayarlar…",
    pause: "Duraklat",
    quit: "Çıkış",
    settings_title: "Ayarlar — Frank",
};

static RU: Texts = Texts {
    open: "Открыть Frank",
    settings: "Настройки…",
    pause: "Пауза",
    quit: "Выход",
    settings_title: "Настройки — Frank",
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chosen_language_wins_over_the_system() {
        assert_eq!(Lang::from_setting("en"), Lang::En);
        assert_eq!(Lang::from_setting("tr"), Lang::Tr);
        assert_eq!(Lang::from_setting("ru"), Lang::Ru);
        assert_eq!(texts(Lang::Tr).settings_title, "Ayarlar — Frank");
    }
}
