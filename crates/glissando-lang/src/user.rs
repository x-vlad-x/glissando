//! Per-language personal model: learned words, personal dictionary entries
//! and word pair counts. Stored locally as a private TSV file; nothing ever
//! leaves the device.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct UserWord {
    pub count: u32,
    /// Unix seconds of the last use.
    pub last: u64,
    /// Explicitly added to the personal dictionary.
    pub pinned: bool,
    /// Explicitly removed: never suggest.
    pub blocked: bool,
}

#[derive(Debug, Default)]
pub struct UserModel {
    pub words: HashMap<String, UserWord>,
    pub bigrams: HashMap<(String, String), u32>,
    path: Option<PathBuf>,
    dirty: bool,
}

/// Words typed this many times become suggestions of their own.
pub const LEARN_THRESHOLD: u32 = 2;
const MAX_WORDS: usize = 20_000;
const MAX_BIGRAMS: usize = 50_000;

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl UserModel {
    pub fn load(path: &Path) -> UserModel {
        let mut m = UserModel { path: Some(path.to_path_buf()), ..Default::default() };
        if let Ok(s) = std::fs::read_to_string(path) {
            for line in s.lines() {
                let f: Vec<&str> = line.split('\t').collect();
                match f.as_slice() {
                    ["w", w, c, l, flags] => {
                        m.words.insert(
                            w.to_string(),
                            UserWord {
                                count: c.parse().unwrap_or(0),
                                last: l.parse().unwrap_or(0),
                                pinned: flags.contains('p'),
                                blocked: flags.contains('b'),
                            },
                        );
                    }
                    ["b", a, b, c] => {
                        m.bigrams.insert((a.to_string(), b.to_string()), c.parse().unwrap_or(0));
                    }
                    _ => {}
                }
            }
        }
        m
    }

    pub fn in_memory() -> UserModel {
        UserModel::default()
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn save(&mut self) -> std::io::Result<()> {
        let Some(path) = self.path.clone() else { return Ok(()) };
        if !self.dirty {
            return Ok(());
        }
        self.prune();
        let mut out = String::new();
        for (w, u) in &self.words {
            let flags = format!("{}{}", if u.pinned { "p" } else { "" }, if u.blocked { "b" } else { "" });
            out.push_str(&format!("w\t{w}\t{}\t{}\t{flags}\n", u.count, u.last));
        }
        for ((a, b), c) in &self.bigrams {
            out.push_str(&format!("b\t{a}\t{b}\t{c}\n"));
        }
        glissando_config::write_private(&path, out.as_bytes())?;
        self.dirty = false;
        Ok(())
    }

    fn prune(&mut self) {
        if self.words.len() > MAX_WORDS {
            let mut v: Vec<(String, UserWord)> = self.words.drain().collect();
            v.sort_by_key(|(_, w)| std::cmp::Reverse((w.pinned, w.last)));
            v.truncate(MAX_WORDS * 9 / 10);
            self.words = v.into_iter().collect();
        }
        if self.bigrams.len() > MAX_BIGRAMS {
            let mut v: Vec<((String, String), u32)> = self.bigrams.drain().collect();
            v.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
            v.truncate(MAX_BIGRAMS * 9 / 10);
            self.bigrams = v.into_iter().collect();
        }
    }

    fn valid(w: &str) -> bool {
        let n = w.chars().count();
        // Never learn things that look like secrets or numbers.
        (1..=40).contains(&n)
            && w.chars().any(|c| c.is_alphabetic())
            && !w.chars().any(|c| c.is_ascii_digit())
            && !w.contains('@')
    }

    pub fn learn(&mut self, word: &str, prev: Option<&str>) {
        if !Self::valid(word) {
            return;
        }
        let e = self.words.entry(word.to_string()).or_default();
        if e.blocked {
            return;
        }
        e.count = e.count.saturating_add(1);
        e.last = now();
        if let Some(p) = prev
            && Self::valid(p)
        {
            *self.bigrams.entry((p.to_lowercase(), word.to_string())).or_insert(0) += 1;
        }
        self.dirty = true;
    }

    /// Undo of an autocorrection: the user insists on this word.
    pub fn insist(&mut self, word: &str) {
        if !Self::valid(word) {
            return;
        }
        let e = self.words.entry(word.to_string()).or_default();
        e.blocked = false;
        e.count = e.count.max(LEARN_THRESHOLD);
        e.last = now();
        self.dirty = true;
    }

    pub fn pin(&mut self, word: &str) {
        let e = self.words.entry(word.to_string()).or_default();
        e.pinned = true;
        e.blocked = false;
        e.last = now();
        self.dirty = true;
    }

    pub fn block(&mut self, word: &str) {
        let e = self.words.entry(word.to_string()).or_default();
        e.blocked = true;
        e.pinned = false;
        self.bigrams.retain(|(_, b), _| b != word);
        self.dirty = true;
    }

    pub fn is_blocked(&self, word: &str) -> bool {
        self.words.get(word).map(|u| u.blocked).unwrap_or(false)
    }

    /// Words that count as known vocabulary.
    pub fn known(&self, word: &str) -> Option<&UserWord> {
        self.words.get(word).filter(|u| !u.blocked && (u.pinned || u.count >= LEARN_THRESHOLD))
    }

    /// Log-score bonus for frequently used words (0 for unknown).
    pub fn boost(&self, word: &str) -> f32 {
        match self.words.get(word) {
            Some(u) if !u.blocked => (1.0 + u.count as f32).ln() * 0.6 + if u.pinned { 1.0 } else { 0.0 },
            _ => 0.0,
        }
    }

    pub fn successors(&self, prev: &str) -> Vec<(String, u32)> {
        let p = prev.to_lowercase();
        let mut v: Vec<(String, u32)> = self
            .bigrams
            .iter()
            .filter(|((a, b), _)| *a == p && !self.is_blocked(b))
            .map(|((_, b), c)| (b.clone(), *c))
            .collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v
    }

    pub fn bigram(&self, prev: &str, word: &str) -> u32 {
        self.bigrams.get(&(prev.to_lowercase(), word.to_string())).copied().unwrap_or(0)
    }

    pub fn clear(&mut self) {
        self.words.clear();
        self.bigrams.clear();
        self.dirty = true;
        if let Some(p) = &self.path {
            let _ = std::fs::remove_file(p);
        }
        self.dirty = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learning_and_persistence() {
        let dir = std::env::temp_dir().join(format!("glissando-user-{}", std::process::id()));
        let path = dir.join("en.tsv");
        let mut m = UserModel::load(&path);
        m.learn("glissando", Some("the"));
        assert!(m.known("glissando").is_none());
        m.learn("glissando", Some("the"));
        assert!(m.known("glissando").is_some());
        m.learn("hunter2", None);
        assert!(!m.words.contains_key("hunter2"), "words with digits are never learned");
        m.save().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let mut m2 = UserModel::load(&path);
        assert_eq!(m2.words["glissando"].count, 2);
        assert_eq!(m2.successors("The")[0].0, "glissando");
        m2.block("glissando");
        assert!(m2.known("glissando").is_none());
        m2.clear();
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
