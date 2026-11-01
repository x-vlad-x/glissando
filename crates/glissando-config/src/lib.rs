//! Shared configuration of the keyboard and the settings app.
//!
//! User configuration (`$XDG_CONFIG_HOME/glissando/config.toml`).
//!
//! Every field has a default so partial files work; unknown keys are
//! ignored. The settings app writes this file and the keyboard reloads it
//! when it changes.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Enabled layouts in switching order (ids of files in the layout dir).
    pub layouts: Vec<String>,
    /// Languages typed with Latin layouts in addition to the layout's own.
    pub multilingual: bool,
    pub theme: ThemeMode,
    pub key_borders: bool,
    pub font: String,
    pub font_scale: f32,
    /// Keyboard height factor (0.8 short ... 1.3 tall).
    pub height: f32,
    pub height_landscape: f32,
    pub number_row: bool,
    /// Esc, Tab, Ctrl, Alt and arrow keys above the letters in terminals.
    pub terminal_keys: bool,
    pub hints: bool,
    pub key_popup: bool,
    pub long_press_ms: u32,
    pub haptics: bool,
    /// 0.0 .. 1.0
    pub haptic_strength: f32,
    pub haptic_ms: u32,
    pub glide: bool,
    pub glide_trail: bool,
    pub glide_delete: bool,
    pub glide_cursor: bool,
    pub trackpad_long_press_space: bool,
    pub one_handed: OneHanded,
    pub suggestions: bool,
    pub autocorrect: bool,
    pub autocap: bool,
    pub autospace: bool,
    pub double_space_period: bool,
    pub next_word: bool,
    pub personalization: bool,
    pub composing_underline: bool,
    pub clipboard_history: bool,
    /// Hours to keep clipboard history.
    pub clipboard_hours: u32,
    pub emoji_recent: bool,
    /// Offer matching emoji in the suggestion strip while typing.
    pub emoji_suggestions: bool,
    /// Show the microphone button when a local speech model is installed.
    pub voice: bool,
    /// App ids (Wayland app_id / desktop file name) in which the keyboard
    /// does not open by itself when a text field gets focus.
    pub no_auto_show: Vec<String>,
    /// Bumped by the settings app after deleting personal data; the running
    /// keyboard drops its in-memory copies when it changes.
    pub user_data_epoch: u64,
    /// Bumped after editing the personal dictionary; the keyboard reloads it.
    pub user_dict_epoch: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OneHanded {
    Off,
    Left,
    Right,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            layouts: vec!["en".into()],
            multilingual: true,
            theme: ThemeMode::System,
            key_borders: false,
            font: "Inter, Cantarell, Sans".into(),
            font_scale: 1.0,
            height: 1.0,
            height_landscape: 1.0,
            number_row: false,
            terminal_keys: true,
            hints: true,
            key_popup: true,
            long_press_ms: 300,
            haptics: true,
            haptic_strength: 0.5,
            haptic_ms: 12,
            glide: true,
            glide_trail: true,
            glide_delete: true,
            glide_cursor: true,
            trackpad_long_press_space: false,
            one_handed: OneHanded::Off,
            suggestions: true,
            autocorrect: true,
            autocap: true,
            autospace: true,
            double_space_period: true,
            next_word: true,
            personalization: true,
            composing_underline: true,
            clipboard_history: true,
            clipboard_hours: 1,
            emoji_recent: true,
            emoji_suggestions: true,
            voice: true,
            no_auto_show: vec!["org.gnome.Calculator".into()],
            user_data_epoch: 0,
            user_dict_epoch: 0,
        }
    }
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(fallback))
}

pub fn config_dir() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join("glissando")
}

pub fn data_dir() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share").join("glissando")
}

pub fn state_dir() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state").join("glissando")
}

/// Directories searched for layouts and dictionaries, most specific first.
pub fn resource_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(d) = std::env::var_os("GLISSANDO_DATA_DIR") {
        v.push(PathBuf::from(d));
    }
    v.push(data_dir());
    let sys = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    for d in sys.split(':').filter(|d| !d.is_empty()) {
        v.push(PathBuf::from(d).join("glissando"));
    }
    v
}

pub fn find_resource(rel: &str) -> Option<PathBuf> {
    resource_dirs().into_iter().map(|d| d.join(rel)).find(|p| p.exists())
}

impl Config {
    /// Should the keyboard stay closed on focus in this app?
    pub fn auto_show_blocked(&self, app_id: &str) -> bool {
        let id = app_id.strip_suffix(".desktop").unwrap_or(app_id);
        !id.is_empty()
            && self.no_auto_show.iter().any(|a| a.strip_suffix(".desktop").unwrap_or(a).eq_ignore_ascii_case(id))
    }

    pub fn path() -> PathBuf {
        std::env::var_os("GLISSANDO_CONFIG").map(PathBuf::from).unwrap_or_else(|| config_dir().join("config.toml"))
    }

    pub fn load() -> Config {
        let p = Config::path();
        match std::fs::read_to_string(&p) {
            Ok(s) => match toml::from_str::<Config>(&s) {
                Ok(c) => c.sanitized(),
                Err(e) => {
                    eprintln!("glissando: ignoring invalid {}: {e}", p.display());
                    Config::default()
                }
            },
            Err(_) => Config::default(),
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let p = Config::path();
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        let s = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp = p.with_extension("tmp");
        std::fs::write(&tmp, s)?;
        std::fs::rename(tmp, p)
    }

    pub fn sanitized(mut self) -> Config {
        self.height = self.height.clamp(0.7, 1.4);
        self.height_landscape = self.height_landscape.clamp(0.7, 1.4);
        self.font_scale = self.font_scale.clamp(0.8, 1.5);
        self.haptic_strength = self.haptic_strength.clamp(0.0, 1.0);
        self.haptic_ms = self.haptic_ms.clamp(3, 60);
        self.long_press_ms = self.long_press_ms.clamp(150, 1000);
        self.clipboard_hours = self.clipboard_hours.clamp(1, 72);
        if self.layouts.is_empty() {
            self.layouts = vec!["en".into()];
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_config_uses_defaults() {
        let c: Config = toml::from_str("layouts = [\"ru\", \"en\"]\nheight = 3.0\n").unwrap();
        let c = c.sanitized();
        assert_eq!(c.layouts, vec!["ru", "en"]);
        assert_eq!(c.height, 1.4);
        assert!(c.glide);
        let s = toml::to_string_pretty(&c).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn auto_show_block_list() {
        let mut c = Config::default();
        assert!(c.auto_show_blocked("org.gnome.Calculator"));
        c.no_auto_show.push("Pix.desktop".into());
        assert!(c.auto_show_blocked("pix"));
        assert!(!c.auto_show_blocked("org.gnome.Console"));
        assert!(!c.auto_show_blocked(""));
    }

    /// config.toml as promised for 1.x. A failure here means existing files
    /// would change meaning: add a migration instead of editing the fixture.
    #[test]
    fn reads_the_config_format() {
        let c: Config = toml::from_str(include_str!("../tests/fixtures/config.toml")).unwrap();
        let expected = Config {
            layouts: vec!["ru".into(), "en".into()],
            multilingual: false,
            theme: ThemeMode::Dark,
            key_borders: true,
            font: "Noto Sans".into(),
            font_scale: 1.1,
            height: 0.4,
            height_landscape: 0.5,
            number_row: true,
            terminal_keys: false,
            hints: false,
            key_popup: false,
            long_press_ms: 400,
            haptics: false,
            haptic_strength: 50.0,
            haptic_ms: 30,
            glide: false,
            glide_trail: false,
            glide_delete: false,
            glide_cursor: false,
            trackpad_long_press_space: true,
            one_handed: OneHanded::Left,
            suggestions: false,
            autocorrect: false,
            autocap: false,
            autospace: false,
            double_space_period: false,
            next_word: false,
            personalization: false,
            composing_underline: false,
            clipboard_history: false,
            clipboard_hours: 3,
            emoji_recent: false,
            emoji_suggestions: false,
            voice: true,
            no_auto_show: vec!["org.gnome.Calculator".into()],
            user_data_epoch: 2,
            user_dict_epoch: 5,
        };
        assert_eq!(c, expected);
    }
}
