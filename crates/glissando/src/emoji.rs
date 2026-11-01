//! Emoji catalogue (from Unicode/CLDR data) with keyword search and a
//! locally stored list of recently used emoji.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Emoji {
    pub text: String,
    pub group: usize,
    /// Skin tone variants (for a future tone picker).
    #[allow(dead_code)]
    pub tones: Vec<String>,
    /// Lowercased keywords of all languages.
    pub keywords: Vec<String>,
    /// Lowercased CLDR short names (one per language).
    pub names: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct EmojiDb {
    pub groups: Vec<String>,
    pub emoji: Vec<Emoji>,
}

/// Representative emoji per group, used as tab labels.
pub fn group_icon(name: &str) -> &'static str {
    match name {
        "Smileys & Emotion" => "😀",
        "People & Body" => "👋",
        "Animals & Nature" => "🐻",
        "Food & Drink" => "🍔",
        "Travel & Places" => "🚗",
        "Activities" => "⚽",
        "Objects" => "💡",
        "Symbols" => "❤️",
        "Flags" => "🏳️",
        _ => "•",
    }
}

impl EmojiDb {
    pub fn parse(src: &str) -> EmojiDb {
        let mut db = EmojiDb::default();
        for line in src.lines() {
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            let cols: Vec<&str> = line.split('\t').collect();
            if cols.len() < 3 {
                continue;
            }
            let group = match db.groups.iter().position(|g| g == cols[1]) {
                Some(i) => i,
                None => {
                    db.groups.push(cols[1].to_string());
                    db.groups.len() - 1
                }
            };
            let tones = cols[2].split_whitespace().map(|s| s.to_string()).collect();
            let mut keywords = Vec::new();
            let mut names = Vec::new();
            for c in &cols[3..] {
                for (i, k) in c.split('|').filter(|k| !k.is_empty()).enumerate() {
                    if i == 0 {
                        names.push(k.to_lowercase());
                    }
                    keywords.push(k.to_lowercase());
                }
            }
            db.emoji.push(Emoji { text: cols[0].to_string(), group, tones, keywords, names });
        }
        db
    }

    pub fn load(path: &Path) -> EmojiDb {
        std::fs::read_to_string(path).map(|s| EmojiDb::parse(&s)).unwrap_or_default()
    }

    pub fn in_group(&self, g: usize) -> impl Iterator<Item = &Emoji> {
        self.emoji.iter().filter(move |e| e.group == g)
    }

    /// Keyword search: prefix matches on any keyword word, best first.
    pub fn search(&self, query: &str, max: usize) -> Vec<&Emoji> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let mut scored: Vec<(f32, usize, &Emoji)> = Vec::new();
        for (i, e) in self.emoji.iter().enumerate() {
            let mut best = 0f32;
            for k in e.keywords.iter() {
                let s = if *k == q {
                    80.0
                } else if k.starts_with(&q) {
                    60.0
                } else if k.split(|c: char| !c.is_alphanumeric()).any(|w| w.starts_with(&q)) {
                    40.0
                } else {
                    0.0
                };
                best = best.max(s);
            }
            if best > 0.0 {
                // Prefer emoji whose short name is the query or contains it as a word, shortest first.
                for n in &e.names {
                    if *n == q {
                        best += 30.0;
                    } else if n.split(|c: char| !c.is_alphanumeric()).any(|w| w == q) {
                        best += 10.0 - (n.chars().count().min(40) as f32) / 4.0;
                    }
                }
                scored.push((best, i, e));
            }
        }
        scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        scored.into_iter().take(max).map(|s| s.2).collect()
    }

    /// Emoji for a word, used for emoji suggestions in the strip.
    pub fn for_word(&self, word: &str, max: usize) -> Vec<&Emoji> {
        let w = word.to_lowercase();
        if w.chars().count() < 3 {
            return Vec::new();
        }
        // Ranked like search, but only emoji with the word as a keyword.
        let mut v: Vec<&Emoji> = self.search(&w, 40).into_iter().filter(|e| e.keywords.contains(&w)).collect();
        v.truncate(max);
        v
    }
}

/// Recently used emoji, persisted (emoji only, no context).
#[derive(Debug, Default)]
pub struct Recents {
    items: VecDeque<String>,
    path: Option<PathBuf>,
}

impl Recents {
    pub fn load(path: &Path) -> Recents {
        let items = std::fs::read_to_string(path)
            .map(|s| s.lines().filter(|l| !l.is_empty()).map(|l| l.to_string()).collect())
            .unwrap_or_default();
        Recents { items, path: Some(path.to_path_buf()) }
    }

    pub fn used(&mut self, e: &str) {
        self.items.retain(|x| x != e);
        self.items.push_front(e.to_string());
        self.items.truncate(40);
        if let Some(p) = &self.path {
            let s: Vec<&str> = self.items.iter().map(|s| s.as_str()).collect();
            if let Err(e) = glissando_config::write_private(p, s.join("\n").as_bytes()) {
                eprintln!("glissando: saving {} failed: {e}", p.display());
            }
        }
    }

    pub fn items(&self) -> impl Iterator<Item = &String> {
        self.items.iter()
    }

    pub fn clear(&mut self) {
        self.items.clear();
        if let Some(p) = &self.path {
            let _ = std::fs::remove_file(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// recent-emoji as promised for 1.x: one emoji per line, newest first.
    #[test]
    fn reads_the_recent_emoji_format() {
        let dir = std::env::temp_dir().join(format!("glissando-recent-format-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("recent-emoji");
        std::fs::write(&path, "🍕\n🐱").unwrap();

        let r = Recents::load(&path);

        assert_eq!(r.items().map(String::as_str).collect::<Vec<_>>(), ["🍕", "🐱"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn parses_shipped_data_and_searches() {
        let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/emoji/emoji.tsv");
        let db = EmojiDb::load(Path::new(p));
        assert!(db.emoji.len() > 1500);
        assert_eq!(db.groups[0], "Smileys & Emotion");
        assert_eq!(db.search("pizza", 3)[0].text, "🍕");
        assert_eq!(db.for_word("Pizza", 1)[0].text, "🍕");
        assert!(db.search("кот", 20).iter().any(|e| e.text == "🐱" || e.text == "🐈"));
        assert!(db.search("herz", 10).iter().any(|e| e.text.starts_with('❤')));
        let wave = db.emoji.iter().find(|e| e.text == "👋").unwrap();
        assert_eq!(wave.tones.len(), 5);
    }

    #[cfg(unix)]
    #[test]
    fn recents_are_replaced_atomically_and_privately() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("glissando-recents-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("recent-emoji");
        std::fs::write(&path, "🍕\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let before = std::fs::metadata(&path).unwrap();

        let mut r = Recents::load(&path);
        r.used("🐱");

        let after = std::fs::metadata(&path).unwrap();
        assert_eq!(after.permissions().mode() & 0o777, 0o600);
        assert_ne!(
            std::os::unix::fs::MetadataExt::ino(&before),
            std::os::unix::fs::MetadataExt::ino(&after),
            "written in place"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "🐱\n🍕");
        let _ = std::fs::remove_dir_all(dir);
    }
}
