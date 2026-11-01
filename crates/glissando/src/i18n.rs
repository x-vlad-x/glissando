//! Minimal UI string translations (English, Russian, German), chosen from
//! the session locale.

use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lang {
    En,
    Ru,
    De,
}

fn lang() -> Lang {
    static L: OnceLock<Lang> = OnceLock::new();
    *L.get_or_init(|| {
        let v = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|k| std::env::var(k).ok())
            .find(|v| !v.is_empty())
            .unwrap_or_default();
        if v.starts_with("ru") {
            Lang::Ru
        } else if v.starts_with("de") {
            Lang::De
        } else {
            Lang::En
        }
    })
}

pub fn tr(key: &str) -> &'static str {
    let l = lang();
    let t: (&str, &str, &str) = match key {
        "settings" => ("Settings", "Настройки", "Einstellungen"),
        "one-handed" => ("One-handed", "Одной рукой", "Einhandmodus"),
        "number-row" => ("Number row", "Ряд цифр", "Zahlenreihe"),
        "glide" => ("Glide typing", "Ввод свайпом", "Gleitende Eingabe"),
        "theme-system" => ("Theme: auto", "Тема: авто", "Design: auto"),
        "theme-light" => ("Theme: light", "Тема: светлая", "Design: hell"),
        "theme-dark" => ("Theme: dark", "Тема: тёмная", "Design: dunkel"),
        "clipboard-history" => ("Clipboard history", "История буфера", "Zwischenablage-Verlauf"),
        "taller" => ("Taller", "Выше", "Höher"),
        "shorter" => ("Shorter", "Ниже", "Niedriger"),
        "clear-learned" => ("Delete learned words", "Удалить выученные слова", "Gelernte Wörter löschen"),
        "clear" => ("Clear", "Очистить", "Leeren"),
        "clipboard-empty" => (
            "Copied text appears here. Long-press an item to pin it.",
            "Здесь появится скопированный текст. Долгое нажатие закрепляет элемент.",
            "Kopierter Text erscheint hier. Lange drücken zum Anheften.",
        ),
        "clipboard-off" => {
            ("Clipboard history is off.", "История буфера обмена выключена.", "Der Zwischenablage-Verlauf ist aus.")
        }
        "voice-listening" => ("Speak now…", "Говорите…", "Jetzt sprechen…"),
        "voice-processing" => ("Recognising…", "Распознаю…", "Wird erkannt…"),
        _ => return "?",
    };
    match l {
        Lang::En => t.0,
        Lang::Ru => t.1,
        Lang::De => t.2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_in_use_has_a_translation() {
        for source in [include_str!("panels.rs"), include_str!("keyboard.rs")] {
            for (at, _) in source.match_indices("tr(\"") {
                if source[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                let key = &source[at + 4..];
                let key = &key[..key.find('"').unwrap()];
                assert_ne!(tr(key), "?", "{key}");
            }
        }
    }
}
