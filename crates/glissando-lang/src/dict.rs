//! Compiled, memory-mappable dictionary: a character trie with word
//! frequencies plus a sorted bigram table.
//!
//! File layout (little endian):
//!
//! ```text
//! 0   magic  "GLDICT01"
//! 8   u32    node count
//! 12  u32    bigram count
//! 16  u32    word count
//! 20  u32    reserved
//! 24  nodes  [ch u32, first_child u32, parent u32, child_count u16, freq u8, max_freq u8]
//! ..  bigrams [prev u32, next u32, freq u8, pad u8 x3]  sorted by (prev, freq desc)
//! ```
//!
//! Node 0 is the root. Children of a node are contiguous and sorted by
//! `ch`. `freq` is 0 for non-terminal nodes and 1..=255 (log scale, as in
//! AOSP word lists) for words; `max_freq` is the best frequency in the
//! subtree, which drives best-first completion.

use std::collections::BTreeMap;
use std::path::Path;

pub const MAGIC: &[u8; 8] = b"GLDICT01";
const HEADER: usize = 24;
const NODE: usize = 16;
const BIGRAM: usize = 12;

enum Storage {
    Mapped(memmap2::Mmap),
    Owned(Vec<u8>),
}

impl Storage {
    fn bytes(&self) -> &[u8] {
        match self {
            Storage::Mapped(m) => m,
            Storage::Owned(v) => v,
        }
    }
}

pub struct Dict {
    data: Storage,
    nodes: usize,
    bigrams: usize,
    words: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Node {
    pub ch: char,
    pub first_child: u32,
    pub parent: u32,
    pub child_count: u16,
    pub freq: u8,
    pub max_freq: u8,
}

#[derive(Debug)]
pub struct DictError(pub String);

impl std::fmt::Display for DictError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DictError {}

impl Dict {
    pub fn open(path: &Path) -> Result<Dict, DictError> {
        let f = std::fs::File::open(path).map_err(|e| DictError(format!("{}: {e}", path.display())))?;
        // SAFETY: dictionary files are installed read-only and never modified in place.
        let m = unsafe { memmap2::Mmap::map(&f) }.map_err(|e| DictError(e.to_string()))?;
        Dict::from_storage(Storage::Mapped(m))
    }

    pub fn from_bytes(v: Vec<u8>) -> Result<Dict, DictError> {
        Dict::from_storage(Storage::Owned(v))
    }

    fn from_storage(data: Storage) -> Result<Dict, DictError> {
        let b = data.bytes();
        if b.len() < HEADER || &b[..8] != MAGIC {
            return Err(DictError("not a glissando dictionary".into()));
        }
        let nodes = u32::from_le_bytes(b[8..12].try_into().unwrap()) as usize;
        let bigrams = u32::from_le_bytes(b[12..16].try_into().unwrap()) as usize;
        let words = u32::from_le_bytes(b[16..20].try_into().unwrap()) as usize;
        if b.len() != HEADER + nodes * NODE + bigrams * BIGRAM || nodes == 0 {
            return Err(DictError("truncated dictionary".into()));
        }
        Ok(Dict { data, nodes, bigrams, words })
    }

    pub fn node_count(&self) -> usize {
        self.nodes
    }

    pub fn word_count(&self) -> usize {
        self.words
    }

    #[inline]
    pub fn node(&self, i: u32) -> Node {
        let b = self.data.bytes();
        let o = HEADER + i as usize * NODE;
        let r = &b[o..o + NODE];
        Node {
            ch: char::from_u32(u32::from_le_bytes(r[0..4].try_into().unwrap())).unwrap_or('\u{fffd}'),
            first_child: u32::from_le_bytes(r[4..8].try_into().unwrap()),
            parent: u32::from_le_bytes(r[8..12].try_into().unwrap()),
            child_count: u16::from_le_bytes(r[12..14].try_into().unwrap()),
            freq: r[14],
            max_freq: r[15],
        }
    }

    pub fn children(&self, n: &Node) -> impl Iterator<Item = (u32, Node)> + '_ {
        let first = n.first_child;
        (0..n.child_count as u32).map(move |k| (first + k, self.node(first + k)))
    }

    pub fn child(&self, n: &Node, c: char) -> Option<(u32, Node)> {
        // Children are sorted by code point: binary search.
        let (mut lo, mut hi) = (0u32, n.child_count as u32);
        while lo < hi {
            let mid = (lo + hi) / 2;
            let id = n.first_child + mid;
            let m = self.node(id);
            match m.ch.cmp(&c) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some((id, m)),
            }
        }
        None
    }

    /// Exact lookup (case sensitive). Returns the terminal node id.
    pub fn lookup(&self, word: &str) -> Option<(u32, u8)> {
        let mut n = self.node(0);
        let mut id = 0;
        for c in word.chars() {
            let (cid, cn) = self.child(&n, c)?;
            id = cid;
            n = cn;
        }
        (n.freq > 0).then_some((id, n.freq))
    }

    /// Frequency of a word ignoring the case of its first letter
    /// (sentence-initial capitals). Returns the stored form too.
    pub fn lookup_any_case(&self, word: &str) -> Option<(u32, u8)> {
        let mut best = self.lookup(word);
        let lower = word.to_lowercase();
        if lower != word
            && let Some(l) = self.lookup(&lower)
            && best.map(|b| l.1 > b.1).unwrap_or(true)
        {
            best = Some(l);
        }
        let cap = glissando_core::text::capitalize(&lower);
        if cap != word
            && let Some(c) = self.lookup(&cap)
            && best.map(|b| c.1 > b.1).unwrap_or(true)
        {
            best = Some(c);
        }
        best
    }

    /// Reconstruct the word ending at node `id`.
    pub fn word(&self, mut id: u32) -> String {
        let mut chars = Vec::new();
        while id != 0 {
            let n = self.node(id);
            chars.push(n.ch);
            id = n.parent;
        }
        chars.iter().rev().collect()
    }

    fn bigram_at(&self, i: usize) -> (u32, u32, u8) {
        let b = self.data.bytes();
        let o = HEADER + self.nodes * NODE + i * BIGRAM;
        let r = &b[o..o + BIGRAM];
        (u32::from_le_bytes(r[0..4].try_into().unwrap()), u32::from_le_bytes(r[4..8].try_into().unwrap()), r[8])
    }

    /// Successors of `prev` (node id) as (node id, freq), best first.
    pub fn successors(&self, prev: u32) -> Vec<(u32, u8)> {
        let (mut lo, mut hi) = (0usize, self.bigrams);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.bigram_at(mid).0 < prev {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let mut out = Vec::new();
        let mut i = lo;
        while i < self.bigrams {
            let (p, n, f) = self.bigram_at(i);
            if p != prev {
                break;
            }
            out.push((n, f));
            i += 1;
        }
        out
    }

    pub fn bigram(&self, prev: u32, next: u32) -> Option<u8> {
        self.successors(prev).into_iter().find(|(n, _)| *n == next).map(|(_, f)| f)
    }

    /// Iterate all words (for tests and tools).
    pub fn for_each_word(&self, mut f: impl FnMut(u32, &str, u8)) {
        let mut stack = vec![(0u32, String::new())];
        while let Some((id, prefix)) = stack.pop() {
            let n = self.node(id);
            if n.freq > 0 {
                f(id, &prefix, n.freq);
            }
            for (cid, c) in self.children(&n) {
                let mut p = prefix.clone();
                p.push(c.ch);
                stack.push((cid, p));
            }
        }
    }
}

/// Builds dictionary files from word lists.
#[derive(Default)]
pub struct DictBuilder {
    words: BTreeMap<String, u8>,
    bigrams: Vec<(String, String, u8)>,
}

impl DictBuilder {
    pub fn add_word(&mut self, w: &str, freq: u8) {
        if w.is_empty() || freq == 0 {
            return;
        }
        let e = self.words.entry(w.to_string()).or_insert(0);
        *e = (*e).max(freq);
    }

    pub fn add_bigram(&mut self, a: &str, b: &str, freq: u8) {
        if freq > 0 {
            self.bigrams.push((a.to_string(), b.to_string(), freq));
        }
    }

    /// Parse the AOSP `.combined` format (`word=foo,f=123,...` lines and
    /// indented `bigram=bar,f=..` lines following their word).
    pub fn add_combined(&mut self, src: &str, min_freq: u8) {
        let mut last_word: Option<String> = None;
        for line in src.lines() {
            let t = line.trim_start();
            let fields: BTreeMap<&str, &str> = t.split(',').filter_map(|kv| kv.split_once('=')).collect();
            if let Some(w) = fields.get("word") {
                let f: u8 = fields.get("f").and_then(|f| f.parse().ok()).unwrap_or(0);
                let not_word = fields.get("not_a_word").map(|v| *v == "true").unwrap_or(false)
                    || fields.get("possibly_offensive").map(|v| *v == "true").unwrap_or(false);
                if f >= min_freq && !not_word {
                    self.add_word(w, f);
                    last_word = Some(w.to_string());
                } else {
                    last_word = None;
                }
            } else if let (Some(b), Some(prev)) = (fields.get("bigram"), last_word.as_ref()) {
                let f: u8 = fields.get("f").and_then(|f| f.parse().ok()).unwrap_or(0);
                self.add_bigram(prev, b, f);
            }
        }
    }

    pub fn build(&self) -> Vec<u8> {
        // Build a pointer trie first, then lay it out breadth-first so that
        // siblings are contiguous.
        struct T {
            ch: char,
            freq: u8,
            children: BTreeMap<char, usize>,
        }
        let mut t = vec![T { ch: '\0', freq: 0, children: BTreeMap::new() }];
        for (w, &f) in &self.words {
            let mut cur = 0;
            for c in w.chars() {
                cur = match t[cur].children.get(&c) {
                    Some(&n) => n,
                    None => {
                        t.push(T { ch: c, freq: 0, children: BTreeMap::new() });
                        let n = t.len() - 1;
                        t[cur].children.insert(c, n);
                        n
                    }
                };
            }
            t[cur].freq = f;
        }
        // max_freq via post-order.
        let mut maxf = vec![0u8; t.len()];
        let mut order = Vec::with_capacity(t.len());
        let mut stack = vec![0usize];
        while let Some(n) = stack.pop() {
            order.push(n);
            stack.extend(t[n].children.values().copied());
        }
        for &n in order.iter().rev() {
            let mut m = t[n].freq;
            for &c in t[n].children.values() {
                m = m.max(maxf[c]);
            }
            maxf[n] = m;
        }
        // BFS layout.
        let mut new_id = vec![0u32; t.len()];
        let mut bfs = vec![0usize];
        let mut head = 0;
        while head < bfs.len() {
            let n = bfs[head];
            head += 1;
            for &c in t[n].children.values() {
                new_id[c] = bfs.len() as u32;
                bfs.push(c);
            }
        }
        let mut parent = vec![0u32; t.len()];
        for (i, node) in t.iter().enumerate() {
            for &c in node.children.values() {
                parent[c] = new_id[i];
            }
        }
        let mut out = Vec::with_capacity(HEADER + bfs.len() * NODE);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(bfs.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // patched below
        out.extend_from_slice(&(self.words.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        for &n in &bfs {
            let node = &t[n];
            let first = node.children.values().next().map(|&c| new_id[c]).unwrap_or(0);
            out.extend_from_slice(&(node.ch as u32).to_le_bytes());
            out.extend_from_slice(&first.to_le_bytes());
            out.extend_from_slice(&parent[n].to_le_bytes());
            out.extend_from_slice(&(node.children.len() as u16).to_le_bytes());
            out.push(node.freq);
            out.push(maxf[n]);
        }
        // Resolve bigrams to node ids.
        let find = |w: &str| -> Option<u32> {
            let mut cur = 0usize;
            for c in w.chars() {
                cur = *t[cur].children.get(&c)?;
            }
            (t[cur].freq > 0).then(|| new_id[cur])
        };
        let mut bg: Vec<(u32, u32, u8)> =
            self.bigrams.iter().filter_map(|(a, b, f)| Some((find(a)?, find(b)?, *f))).collect();
        bg.sort_by(|x, y| x.0.cmp(&y.0).then(y.2.cmp(&x.2)).then(x.1.cmp(&y.1)));
        bg.dedup_by(|x, y| x.0 == y.0 && x.1 == y.1);
        for (p, n, f) in &bg {
            out.extend_from_slice(&p.to_le_bytes());
            out.extend_from_slice(&n.to_le_bytes());
            out.push(*f);
            out.extend_from_slice(&[0, 0, 0]);
        }
        out[12..16].copy_from_slice(&(bg.len() as u32).to_le_bytes());
        out
    }
}

#[cfg(test)]
pub(crate) fn test_dict(words: &[(&str, u8)], bigrams: &[(&str, &str, u8)]) -> Dict {
    let mut b = DictBuilder::default();
    for (w, f) in words {
        b.add_word(w, *f);
    }
    for (x, y, f) in bigrams {
        b.add_bigram(x, y, *f);
    }
    Dict::from_bytes(b.build()).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let d = test_dict(
            &[("the", 220), ("then", 150), ("there", 160), ("Berlin", 140), ("über", 120)],
            &[("the", "there", 90), ("the", "then", 40)],
        );
        assert_eq!(d.word_count(), 5);
        let (id, f) = d.lookup("there").unwrap();
        assert_eq!(f, 160);
        assert_eq!(d.word(id), "there");
        assert!(d.lookup("ther").is_none());
        assert_eq!(d.lookup_any_case("berlin").map(|x| x.1), Some(140));
        assert_eq!(d.lookup_any_case("Über").map(|x| x.1), Some(120));
        let the = d.lookup("the").unwrap().0;
        let succ = d.successors(the);
        assert_eq!(succ.len(), 2);
        assert_eq!(d.word(succ[0].0), "there");
        assert_eq!(d.node(0).max_freq, 220);
        let mut n = 0;
        d.for_each_word(|_, _, _| n += 1);
        assert_eq!(n, 5);
    }

    #[test]
    fn combined_format() {
        let src = "dictionary=main:en\n word=hello,f=200,flags=,originalFreq=200\n  bigram=world,f=100\n word=world,f=180\n word=xx,f=10,not_a_word=true\n";
        let mut b = DictBuilder::default();
        b.add_combined(src, 1);
        let d = Dict::from_bytes(b.build()).unwrap();
        assert_eq!(d.word_count(), 2);
        let h = d.lookup("hello").unwrap().0;
        let w = d.lookup("world").unwrap().0;
        assert_eq!(d.bigram(h, w), Some(100));
    }
}
