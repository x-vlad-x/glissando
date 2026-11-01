//! Clipboard history. Entries live in memory only and expire after the
//! configured retention; pinned entries are kept in a private file. Text
//! copied from password managers (sensitive MIME hint) is never recorded.

use std::cell::RefCell;
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub text: String,
    pub pinned: bool,
    added: Option<Instant>,
}

#[derive(Debug, Default)]
pub struct ClipboardHistory {
    clips: Vec<Clip>,
    pub enabled: bool,
    pub retention: Duration,
    pinned_path: Option<PathBuf>,
}

const MAX_CLIPS: usize = 30;
const MAX_LEN: usize = 20_000;

thread_local! {
    pub static HISTORY: RefCell<ClipboardHistory> = RefCell::new(ClipboardHistory::default());
}

impl ClipboardHistory {
    pub fn configure(&mut self, enabled: bool, hours: u32, pinned_path: Option<PathBuf>) {
        self.enabled = enabled;
        self.retention = Duration::from_secs(hours as u64 * 3600);
        if self.pinned_path.is_none() {
            if let Some(p) = &pinned_path
                && let Ok(s) = std::fs::read_to_string(p)
            {
                for t in s.split('\u{1e}').filter(|t| !t.is_empty()) {
                    self.clips.push(Clip { text: t.to_string(), pinned: true, added: None });
                }
            }
            self.pinned_path = pinned_path;
        }
        if !enabled {
            self.clips.retain(|c| c.pinned);
        }
    }

    /// Record a new clipboard text. `sensitive` comes from the offer's MIME
    /// types (e.g. `x-kde-passwordManagerHint`).
    pub fn add(&mut self, text: &str, sensitive: bool) {
        if !self.enabled || sensitive || text.trim().is_empty() || text.len() > MAX_LEN {
            return;
        }
        let pinned = self.clips.iter().any(|c| c.text == text && c.pinned);
        self.clips.retain(|c| c.text != text);
        self.clips.insert(0, Clip { text: text.to_string(), pinned, added: Some(Instant::now()) });
        self.expire();
    }

    pub fn expire(&mut self) {
        let r = self.retention;
        self.clips.retain(|c| c.pinned || c.added.map(|a| a.elapsed() <= r).unwrap_or(false));
        let mut unpinned = 0;
        self.clips.retain(|c| {
            if c.pinned {
                true
            } else {
                unpinned += 1;
                unpinned <= MAX_CLIPS
            }
        });
    }

    pub fn items(&mut self) -> Vec<Clip> {
        self.expire();
        let mut v = self.clips.clone();
        v.sort_by_key(|c| !c.pinned);
        v
    }

    pub fn toggle_pin(&mut self, text: &str) {
        if let Some(c) = self.clips.iter_mut().find(|c| c.text == text) {
            c.pinned = !c.pinned;
            if !c.pinned {
                c.added = Some(Instant::now());
            }
        }
        self.save_pinned();
    }

    pub fn clear(&mut self) {
        self.clips.retain(|c| c.pinned);
    }

    pub fn clear_all(&mut self) {
        self.clips.clear();
        self.save_pinned();
    }

    fn save_pinned(&self) {
        let Some(p) = &self.pinned_path else { return };
        let pinned: Vec<&str> = self.clips.iter().filter(|c| c.pinned).map(|c| c.text.as_str()).collect();
        if pinned.is_empty() {
            let _ = std::fs::remove_file(p);
            return;
        }
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        if let Ok(mut f) = opts.open(p) {
            use std::io::Write;
            let _ = f.write_all(pinned.join("\u{1e}").as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_rules() {
        let mut h = ClipboardHistory::default();
        h.configure(true, 1, None);
        h.add("one", false);
        h.add("secret", true);
        h.add("two", false);
        h.add("one", false);
        let items: Vec<String> = h.items().into_iter().map(|c| c.text).collect();
        assert_eq!(items, vec!["one", "two"]);
        h.toggle_pin("two");
        assert_eq!(h.items()[0].text, "two");
        h.clear();
        assert_eq!(h.items().len(), 1);
        h.configure(false, 1, None);
        h.add("three", false);
        assert_eq!(h.items().len(), 1, "disabled history records nothing");
    }
}
