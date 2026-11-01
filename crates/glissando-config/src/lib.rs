//! Shared configuration of the keyboard and the settings app.
//!
//! User configuration (`$XDG_CONFIG_HOME/glissando/config.toml`).
//!
//! Every field has a default so partial files work; unknown keys are
//! ignored. The settings app writes this file and the keyboard reloads it
//! when it changes.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

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

/// Replace the file `path` with `contents`. The bytes go to a temporary file
/// next to it, readable only by the user, which is then renamed over `path`: a
/// crash leaves either the old or the new file, never a partial one, and
/// readers never see an empty file. The name of the temporary file is unique,
/// since the keyboard and the settings app can write at the same time.
pub fn write_private(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".{}.{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let tmp = PathBuf::from(tmp);
    // A crashed process with the same id can have left this name behind.
    match std::fs::remove_file(&tmp) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let written = opts.open(&tmp).and_then(|mut f| {
        f.write_all(contents)?;
        f.sync_all()
    });
    let result = written.and_then(|()| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
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
        Config::read().unwrap_or_else(|e| {
            eprintln!("glissando: ignoring invalid {}: {e}", Config::path().display());
            Config::default()
        })
    }

    /// The settings in the file, the defaults if there is no file, or an error
    /// if it cannot be used. Unlike `load` this tells a broken file from
    /// default settings, for callers that act on the difference.
    pub fn read() -> std::io::Result<Config> {
        Config::read_from(&Config::path())
    }

    fn read_from(p: &Path) -> std::io::Result<Config> {
        match std::fs::read_to_string(p) {
            Ok(s) => toml::from_str::<Config>(&s)
                .map(Config::sanitized)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e),
        }
    }

    fn save_to(&self, p: &Path) -> std::io::Result<()> {
        let s = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        write_private(p, s.as_bytes())
    }

    /// Run `f` while no other process writes the file at `path`.
    fn locked<T>(path: &Path, f: impl FnOnce() -> std::io::Result<T>) -> std::io::Result<T> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let lock =
            std::fs::OpenOptions::new().write(true).create(true).truncate(false).open(path.with_extension("lock"))?;
        lock.lock()?;
        f()
    }

    /// Change the settings as they are in the file now and return the result.
    /// A copy loaded earlier may be out of date, since the keyboard and the
    /// settings app both write the file. A broken file stays as it is.
    pub fn update(change: impl FnOnce(&mut Config)) -> std::io::Result<Config> {
        Config::update_file(&Config::path(), change)
    }

    fn update_file(path: &Path, change: impl FnOnce(&mut Config)) -> std::io::Result<Config> {
        Config::locked(path, || {
            let before = Config::read_from(path)?;
            let mut after = before.clone();
            change(&mut after);
            if after != before {
                after.save_to(path)?;
            }
            Ok(after)
        })
    }

    /// Write the settings that differ between `base` and `mine` into the file,
    /// keeping everything else as it is there now.
    pub fn save_changes(base: &Config, mine: &Config) -> std::io::Result<()> {
        Config::save_changes_to(&Config::path(), base, mine)
    }

    fn save_changes_to(path: &Path, base: &Config, mine: &Config) -> std::io::Result<()> {
        let table = |c: &Config| toml::Table::try_from(c).map_err(std::io::Error::other);
        let (base, mine) = (table(base)?, table(mine)?);
        Config::locked(path, || {
            let mut merged = table(&Config::read_from(path)?)?;
            for (key, value) in mine {
                if base.get(&key) != Some(&value) {
                    merged.insert(key, value);
                }
            }
            let merged: Config = merged.try_into().map_err(std::io::Error::other)?;
            merged.save_to(path)
        })
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

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("glissando-config-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(unix)]
    #[test]
    fn write_private_leaves_only_a_private_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch_dir("private");
        let path = dir.join("data");
        std::fs::write(&path, "old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        write_private(&path, b"new").unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn write_private_creates_missing_directories() {
        let dir = scratch_dir("mkdir");
        let path = dir.join("a").join("b").join("data");
        write_private(&path, b"x").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"x");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn failed_write_leaves_no_temporary_file() {
        let dir = scratch_dir("fail");
        // A directory in the way makes the final rename fail.
        let path = dir.join("data");
        std::fs::create_dir(&path).unwrap();

        assert!(write_private(&path, b"new").is_err());

        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn concurrent_updates_are_all_kept() {
        let dir = scratch_dir("concurrent");
        let path = dir.join("config.toml");
        std::thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(|| {
                    for _ in 0..25 {
                        Config::update_file(&path, |c| c.user_dict_epoch += 1).unwrap();
                    }
                });
            }
        });
        assert_eq!(Config::read_from(&path).unwrap().user_dict_epoch, 100);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn update_leaves_a_broken_file_alone() {
        let dir = scratch_dir("broken");
        let path = dir.join("config.toml");
        std::fs::write(&path, "theme = [unfinished").unwrap();

        assert!(Config::update_file(&path, |c| c.haptics = false).is_err());
        assert!(Config::save_changes_to(&path, &Config::default(), &Config::default()).is_err());

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "theme = [unfinished");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn saved_changes_keep_what_the_other_process_changed() {
        let dir = scratch_dir("merge");
        let path = dir.join("config.toml");
        let base = Config::default();
        // The settings app raises the epoch while the keyboard's copy is old.
        Config { user_data_epoch: 3, ..base.clone() }.save_to(&path).unwrap();
        let mine = Config { number_row: !base.number_row, ..base.clone() };

        Config::save_changes_to(&path, &base, &mine).unwrap();

        let on_disk = Config::read_from(&path).unwrap();
        assert_eq!(on_disk.user_data_epoch, 3);
        assert_eq!(on_disk.number_row, mine.number_row);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn update_keeps_changes_made_by_the_other_process() {
        let dir = scratch_dir("update");
        let path = dir.join("config.toml");
        // The settings app has the file open with these values...
        Config::default().save_to(&path).unwrap();
        // ...when the keyboard's quick menu changes two others.
        let keyboard = Config { theme: ThemeMode::Dark, number_row: true, ..Config::read_from(&path).unwrap() };
        keyboard.save_to(&path).unwrap();

        let c = Config::update_file(&path, |c| c.haptics = false).unwrap();

        let on_disk = Config::read_from(&path).unwrap();
        assert_eq!(on_disk, c);
        assert_eq!(on_disk.theme, ThemeMode::Dark);
        assert!(on_disk.number_row);
        assert!(!on_disk.haptics);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn update_counts_from_the_value_on_disk() {
        let dir = scratch_dir("epoch");
        let path = dir.join("config.toml");
        Config { user_dict_epoch: 7, ..Config::default() }.save_to(&path).unwrap();

        let c = Config::update_file(&path, |c| c.user_dict_epoch += 1).unwrap();

        assert_eq!(c.user_dict_epoch, 8);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn update_without_a_change_does_not_write() {
        let dir = scratch_dir("noop");
        let path = dir.join("config.toml");

        Config::update_file(&path, |_| {}).unwrap();

        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
