//! Glide typing decoder.
//!
//! The built-in decoder is layout and language agnostic. It walks the
//! dictionary trie and keeps only prefixes whose letters the finger passed
//! in order (a cheap geometric feasibility test), then scores the surviving
//! words with
//!
//! * a monotonic alignment of each letter's key to the gesture path,
//! * the location distance between the resampled gesture and the ideal
//!   polyline through the word's keys,
//! * the scale/translation invariant shape distance of both,
//!
//! combined with word frequency, personal history and context. All
//! coordinates are in key-width units.

use glissando_lang::dict::Dict;
use glissando_lang::model::{Language, logf};
use std::collections::HashMap;

pub const RESAMPLE: usize = 48;
/// Candidates that get the full (expensive) score.
const STAGE2: usize = 120;
/// Candidates that get the alignment score.
const STAGE1: usize = 700;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pt {
    pub x: f32,
    pub y: f32,
}

impl Pt {
    fn d(self, o: Pt) -> f32 {
        ((self.x - o.x).powi(2) + (self.y - o.y).powi(2)).sqrt()
    }
}

/// Key centres of the letters on the current layout, in key-width units.
#[derive(Debug, Clone, Default)]
pub struct KeyMap {
    keys: HashMap<char, Pt>,
}

impl KeyMap {
    pub fn new(keys: impl IntoIterator<Item = (char, f32, f32)>) -> KeyMap {
        let mut m = HashMap::new();
        for (c, x, y) in keys {
            for l in c.to_lowercase() {
                m.insert(l, Pt { x, y });
            }
        }
        KeyMap { keys: m }
    }

    /// Where a word character has to be swiped (folding letters that live
    /// in long-press popups onto their base key).
    pub fn pos(&self, c: char) -> Option<Pt> {
        let l = c.to_lowercase().next().unwrap_or(c);
        if let Some(p) = self.keys.get(&l) {
            return Some(*p);
        }
        let base = match l {
            'ё' => 'е',
            'ъ' => 'ь',
            'ß' => 's',
            _ => glissando_core::text::fold_char(l),
        };
        self.keys.get(&base).copied()
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Distinct key positions (letters sharing a key appear once).
    pub fn distinct_positions(&self) -> Vec<Pt> {
        let mut v: Vec<Pt> = Vec::new();
        for p in self.keys.values() {
            if !v.iter().any(|q| q.d(*p) < 1e-3) {
                v.push(*p);
            }
        }
        v.sort_by(|a, b| (a.y, a.x).partial_cmp(&(b.y, b.x)).unwrap());
        v
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GestureCandidate {
    pub word: String,
    pub score: f32,
    pub lang: usize,
}

#[derive(Debug, Clone)]
pub struct DecodeParams {
    /// Radius around a key centre that counts as passing the key.
    pub pass_radius: f32,
    /// Allowed distance of the first/last letter from the gesture ends.
    pub end_radius: f32,
    pub max_results: usize,
    pub max_expansions: usize,
    pub w_align: f32,
    pub w_loc: f32,
    pub w_shape: f32,
    pub w_len: f32,
    /// Weight of the geometric cost against the language score.
    pub w_geom: f32,
    /// Penalty for words with apostrophes/hyphens (rarely glided).
    pub punct_penalty: f32,
    /// Weight of the squared distance between the first/last letter and the gesture ends.
    pub w_ends: f32,
    /// Penalty for all-caps words (acronyms are rarely glided).
    pub caps_penalty: f32,
    /// Fingers land slightly below key centres; the trace is shifted up by this (kw).
    pub y_offset: f32,
    /// Weight of finger speed at inner letters (needs timestamps).
    pub w_slow: f32,
}

impl Default for DecodeParams {
    /// Tuned on the FUTO swipe dev split (real gestures); see docs/ARCHITECTURE.md.
    fn default() -> Self {
        DecodeParams {
            pass_radius: 1.3,
            end_radius: 1.7,
            max_results: 8,
            max_expansions: 400_000,
            w_align: 2.4,
            w_loc: 1.3,
            w_shape: 1.6,
            w_len: 2.0,
            w_geom: 3.5,
            punct_penalty: 3.0,
            w_ends: 0.5,
            caps_penalty: 3.0,
            y_offset: 0.1,
            w_slow: 0.0,
        }
    }
}

impl DecodeParams {
    /// Override parameters from a `name=value,...` string (tuning tools).
    pub fn with_overrides(mut self, spec: &str) -> DecodeParams {
        for kv in spec.split(',').filter(|s| !s.is_empty()) {
            let Some((k, v)) = kv.split_once('=') else { continue };
            let Ok(v) = v.parse::<f32>() else { continue };
            match k {
                "pass" => self.pass_radius = v,
                "end" => self.end_radius = v,
                "align" => self.w_align = v,
                "loc" => self.w_loc = v,
                "shape" => self.w_shape = v,
                "len" => self.w_len = v,
                "geom" => self.w_geom = v,
                "punct" => self.punct_penalty = v,
                "ends" => self.w_ends = v,
                "caps" => self.caps_penalty = v,
                "yoff" => self.y_offset = v,
                "slow" => self.w_slow = v,
                _ => {}
            }
        }
        self
    }
}

/// Resample a polyline to `n` equidistant points.
pub fn resample(points: &[Pt], n: usize) -> Vec<Pt> {
    if points.is_empty() {
        return Vec::new();
    }
    let total: f32 = points.windows(2).map(|w| w[0].d(w[1])).sum();
    if total <= 1e-4 || points.len() == 1 {
        return vec![points[0]; n];
    }
    let step = total / (n - 1) as f32;
    let mut out = Vec::with_capacity(n);
    out.push(points[0]);
    let mut acc = 0.0;
    let mut prev = points[0];
    let mut i = 1;
    while i < points.len() && out.len() < n - 1 {
        let cur = points[i];
        let d = prev.d(cur);
        if acc + d >= step && d > 0.0 {
            let t = (step - acc) / d;
            let q = Pt { x: prev.x + t * (cur.x - prev.x), y: prev.y + t * (cur.y - prev.y) };
            out.push(q);
            prev = q;
            acc = 0.0;
        } else {
            acc += d;
            prev = cur;
            i += 1;
        }
    }
    while out.len() < n {
        out.push(*points.last().unwrap());
    }
    out
}

fn normalize(points: &[Pt]) -> Vec<Pt> {
    let (mut minx, mut miny, mut maxx, mut maxy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    let (mut cx, mut cy) = (0.0, 0.0);
    for p in points {
        minx = minx.min(p.x);
        miny = miny.min(p.y);
        maxx = maxx.max(p.x);
        maxy = maxy.max(p.y);
        cx += p.x;
        cy += p.y;
    }
    let n = points.len() as f32;
    cx /= n;
    cy /= n;
    let s = (maxx - minx).max(maxy - miny).max(0.5);
    points.iter().map(|p| Pt { x: (p.x - cx) / s, y: (p.y - cy) / s }).collect()
}

fn mean_dist(a: &[Pt], b: &[Pt]) -> f32 {
    a.iter().zip(b).map(|(p, q)| p.d(*q)).sum::<f32>() / a.len().max(1) as f32
}

/// Remove jitter and very close duplicate points of a raw trace.
pub fn clean_trace(raw: &[(f32, f32)], min_step: f32) -> Vec<Pt> {
    let mut out: Vec<Pt> = Vec::with_capacity(raw.len());
    for &(x, y) in raw {
        let p = Pt { x, y };
        if out.last().map(|l| l.d(p) >= min_step).unwrap_or(true) {
            out.push(p);
        }
    }
    if let (Some(&(x, y)), Some(last)) = (raw.last(), out.last())
        && (last.x != x || last.y != y)
    {
        out.push(Pt { x, y });
    }
    out
}

/// Like [`clean_trace`] but keeps the timestamps of the retained points.
pub fn clean_trace_timed(raw: &[(f32, f32, f32)], min_step: f32) -> (Vec<Pt>, Vec<f32>) {
    let mut pts: Vec<Pt> = Vec::with_capacity(raw.len());
    let mut ts: Vec<f32> = Vec::with_capacity(raw.len());
    for (i, &(x, y, t)) in raw.iter().enumerate() {
        let p = Pt { x, y };
        let last = i + 1 == raw.len();
        if pts.last().map(|l| l.d(p) >= min_step).unwrap_or(true) || (last && pts.last() != Some(&p)) {
            pts.push(p);
            ts.push(t);
        } else if let Some(lt) = ts.last_mut() {
            // Dwelling: keep the latest time for the retained point.
            *lt = t;
        }
    }
    (pts, ts)
}

struct Prepared {
    path: Vec<Pt>,
    /// Relative finger speed at each resampled point (1 = median), 1 if unknown.
    speed: Vec<f32>,
    norm: Vec<Pt>,
    start: Pt,
    end: Pt,
    length: f32,
}

/// Letter positions of a word; consecutive duplicates collapse. `None` if
/// the word contains letters that cannot be glided on this layout.
fn word_keys(word: &str, keys: &KeyMap) -> Option<Vec<Pt>> {
    let mut v: Vec<Pt> = Vec::new();
    for c in word.chars() {
        if c == '\'' || c == '’' || c == '-' {
            continue;
        }
        let p = keys.pos(c)?;
        if v.last().map(|l| l.d(p) > 1e-3).unwrap_or(true) {
            v.push(p);
        }
    }
    Some(v)
}

/// Time at each of `n` equidistant arc-length positions.
fn resample_times(points: &[Pt], times: &[f32], n: usize) -> Vec<f32> {
    let mut cum = Vec::with_capacity(points.len());
    let mut acc = 0.0;
    cum.push(0.0);
    for w in points.windows(2) {
        acc += w[0].d(w[1]);
        cum.push(acc);
    }
    let total = acc.max(1e-6);
    let mut out = Vec::with_capacity(n);
    let mut j = 0;
    for k in 0..n {
        let s = total * k as f32 / (n - 1) as f32;
        while j + 1 < cum.len() - 1 && cum[j + 1] < s {
            j += 1;
        }
        let (s0, s1) = (cum[j], cum[(j + 1).min(cum.len() - 1)]);
        let (t0, t1) = (times[j], times[(j + 1).min(times.len() - 1)]);
        let f = if s1 > s0 { ((s - s0) / (s1 - s0)).clamp(0.0, 1.0) } else { 0.0 };
        out.push(t0 + (t1 - t0) * f);
    }
    out
}

/// Relative speed per resampled point from resampled times (equal spacing:
/// speed is inversely proportional to the time step).
fn relative_speed(t: &[f32]) -> Vec<f32> {
    let n = t.len();
    let dts: Vec<f32> = (0..n)
        .map(|i| {
            let a = t[i.saturating_sub(1)];
            let b = t[(i + 1).min(n - 1)];
            (b - a).max(0.5)
        })
        .collect();
    let mut sorted = dts.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let med = sorted[n / 2].max(0.5);
    dts.iter().map(|dt| (med / dt).clamp(0.0, 3.0)).collect()
}

/// Monotonic alignment of letters to path points: mean squared distance,
/// plus `slow_w` times the relative speed where inner letters are placed
/// (letters are usually where the finger slows down or turns).
fn alignment_cost(letters: &[Pt], path: &[Pt], speed: &[f32], slow_w: f32) -> f32 {
    let n = path.len();
    let l = letters.len();
    if l == 0 {
        return f32::MAX;
    }
    let mut prev = vec![f32::MAX; n];
    // First letter anchored to the start region.
    for (i, p) in path.iter().enumerate().take(n / 4 + 1) {
        let d = letters[0].d(*p);
        prev[i] = d * d + i as f32 * 0.01;
    }
    for (k, letter) in letters.iter().enumerate().skip(1) {
        let mut cur = vec![f32::MAX; n];
        let mut best = f32::MAX;
        #[allow(clippy::needless_range_loop)] // `i` walks three parallel slices
        for i in 0..n {
            best = best.min(prev[i]);
            if best < f32::MAX {
                let d = letter.d(path[i]);
                let sp = if k + 1 < l { slow_w * speed[i] } else { 0.0 };
                cur[i] = best + d * d + sp;
            }
        }
        prev = cur;
    }
    // Last letter anchored to the end region.
    let tail = prev[n - 1 - n / 6..].iter().copied().fold(f32::MAX, f32::min);
    tail / l as f32
}

#[derive(Default)]
pub struct ShapeDecoder {
    pub params: DecodeParams,
}

impl ShapeDecoder {
    fn prepare(&self, trace: &[Pt], times: Option<&[f32]>) -> Option<Prepared> {
        if trace.len() < 2 {
            return None;
        }
        let shifted: Vec<Pt>;
        let trace = if self.params.y_offset != 0.0 {
            shifted = trace.iter().map(|p| Pt { x: p.x, y: p.y - self.params.y_offset }).collect();
            &shifted[..]
        } else {
            trace
        };
        let length: f32 = trace.windows(2).map(|w| w[0].d(w[1])).sum();
        let path = resample(trace, RESAMPLE);
        let speed = match times {
            Some(t) if t.len() == trace.len() && t.last() > t.first() => {
                relative_speed(&resample_times(trace, t, RESAMPLE))
            }
            _ => vec![1.0; RESAMPLE],
        };
        let norm = normalize(&path);
        Some(Prepared { start: trace[0], end: *trace.last().unwrap(), path, speed, norm, length })
    }

    /// Words whose letters the gesture visits in order, from one dictionary.
    fn feasible(&self, dict: &Dict, keys: &KeyMap, g: &Prepared, out: &mut Vec<(u32, u8)>) {
        let r = self.params.pass_radius;
        let path = &g.path;
        let n = path.len();
        // next[k][i]: first path index >= i that passes within `r` of key k (n = never).
        let key_list: Vec<Pt> = keys.distinct_positions();
        let mut next: Vec<Vec<u8>> = Vec::with_capacity(key_list.len());
        for kp in &key_list {
            let mut v = vec![n as u8; n + 1];
            for i in (0..n).rev() {
                v[i] = if path[i].d(*kp) <= r { i as u8 } else { v[i + 1] };
            }
            next.push(v);
        }
        let key_index = |p: Pt| key_list.iter().position(|q| q.d(p) < 1e-3);
        // DFS state: node, next usable path index, last key index, depth.
        let root = dict.node(0);
        let mut stack: Vec<(u32, u8, Option<usize>, u8)> = Vec::new();
        for (cid, child) in dict.children(&root) {
            if let Some(p) = keys.pos(child.ch)
                && p.d(g.start) <= self.params.end_radius
            {
                stack.push((cid, 0, key_index(p), 1));
            }
        }
        let mut expansions = 0;
        while let Some((id, idx, last, depth)) = stack.pop() {
            expansions += 1;
            if expansions > self.params.max_expansions {
                break;
            }
            let node = dict.node(id);
            if node.freq > 0
                && depth >= 2
                && let Some(lk) = last
                && key_list[lk].d(g.end) <= self.params.end_radius
            {
                out.push((id, node.freq));
            }
            if depth >= 24 {
                continue;
            }
            for (cid, child) in dict.children(&node) {
                if child.max_freq == 0 {
                    continue;
                }
                if child.ch == '\'' || child.ch == '’' || child.ch == '-' {
                    stack.push((cid, idx, last, depth));
                    continue;
                }
                let Some(p) = keys.pos(child.ch) else { continue };
                let Some(k) = key_index(p) else { continue };
                if last == Some(k) {
                    // Double letter: same key, same place on the path.
                    stack.push((cid, idx, last, depth + 1));
                    continue;
                }
                let j = next[k][idx as usize];
                if (j as usize) < n {
                    stack.push((cid, j, Some(k), depth + 1));
                }
            }
        }
    }

    /// Cheap pre-score used to pick candidates for full scoring.
    fn quick_cost(&self, letters: &[Pt], g: &Prepared) -> f32 {
        let p = &self.params;
        let ends = letters[0].d(g.start).powi(2) + letters[letters.len() - 1].d(g.end).powi(2);
        let tlen: f32 = letters.windows(2).map(|w| w[0].d(w[1])).sum();
        let len_pen = ((g.length - tlen).abs() / (tlen + 1.0)).min(2.0);
        p.w_align * alignment_cost(letters, &g.path, &g.speed, p.w_slow) + p.w_len * len_pen + p.w_ends * ends
    }

    fn shape_cost(&self, letters: &[Pt], g: &Prepared) -> f32 {
        let tmpl = resample(letters, RESAMPLE);
        let loc = mean_dist(&g.path, &tmpl);
        let shape = mean_dist(&g.norm, &normalize(&tmpl));
        let align = alignment_cost(letters, &g.path, &g.speed, self.params.w_slow);
        let tlen: f32 = letters.windows(2).map(|w| w[0].d(w[1])).sum();
        let len_pen = ((g.length - tlen).abs() / (tlen + 1.0)).min(2.0);
        let p = &self.params;
        let ends = letters[0].d(g.start).powi(2) + letters[letters.len() - 1].d(g.end).powi(2);
        p.w_align * align + p.w_loc * loc + p.w_shape * shape + p.w_len * len_pen + p.w_ends * ends
    }

    /// Decode a gesture. `trace` is in key-width units. `active` are
    /// (language index, log prior) pairs; `prev` is the previous word.
    pub fn decode(
        &self,
        trace: &[Pt],
        keys: &KeyMap,
        langs: &[Language],
        active: &[(usize, f32)],
        prev: Option<&str>,
    ) -> Vec<GestureCandidate> {
        self.decode_timed(trace, None, keys, langs, active, prev)
    }

    /// Like [`decode`](Self::decode) with per-point timestamps in ms.
    pub fn decode_timed(
        &self,
        trace: &[Pt],
        times: Option<&[f32]>,
        keys: &KeyMap,
        langs: &[Language],
        active: &[(usize, f32)],
        prev: Option<&str>,
    ) -> Vec<GestureCandidate> {
        let Some(g) = self.prepare(trace, times) else { return Vec::new() };
        let mut scored: Vec<GestureCandidate> = Vec::new();
        for &(li, prior) in active {
            let lang = &langs[li];
            let mut words: Vec<(String, u8)> = Vec::new();
            let prof = std::env::var_os("GLISSANDO_SWIPE_PROFILE").is_some();
            let t0 = std::time::Instant::now();
            if let Some(d) = &lang.dict {
                let mut ids = Vec::new();
                self.feasible(d, keys, &g, &mut ids);
                words.extend(ids.into_iter().map(|(id, f)| (d.word(id), f)));
            }
            if prof {
                eprintln!("feasible: {} words in {:.2} ms", words.len(), t0.elapsed().as_secs_f64() * 1000.0);
            }
            for (w, u) in lang.user.words.iter() {
                if u.blocked || !(u.pinned || u.count >= glissando_lang::user::LEARN_THRESHOLD) {
                    continue;
                }
                if w.chars().count() >= 2 && !words.iter().any(|(x, _)| x == w) {
                    words.push((w.clone(), 0));
                }
            }
            // Stage 1: cheap alignment score, keep the most promising words.
            let mut pre: Vec<(f32, String, u8, Vec<Pt>)> = Vec::new();
            for (w, f) in words {
                if lang.user.is_blocked(&w) {
                    continue;
                }
                let Some(letters) = word_keys(&w, keys) else { continue };
                if letters.is_empty() {
                    continue;
                }
                if letters[0].d(g.start) > self.params.end_radius
                    || letters.last().unwrap().d(g.end) > self.params.end_radius
                {
                    continue;
                }
                let freq = if f > 0 { logf(f) } else { lang.dict_logf(&w).unwrap_or(-6.0) };
                // Stage 0: O(word length) bound from ends and path length only.
                let p = &self.params;
                let ends = letters[0].d(g.start).powi(2) + letters[letters.len() - 1].d(g.end).powi(2);
                let tlen: f32 = letters.windows(2).map(|w| w[0].d(w[1])).sum();
                let len_pen = ((g.length - tlen).abs() / (tlen + 1.0)).min(2.0);
                let q0 = -(p.w_len * len_pen + p.w_ends * ends) * p.w_geom + freq;
                pre.push((q0, w, f, letters));
            }
            if pre.len() > STAGE1 {
                pre.select_nth_unstable_by(STAGE1, |a, b| b.0.total_cmp(&a.0));
                pre.truncate(STAGE1);
            }
            for e in pre.iter_mut() {
                let freq = if e.2 > 0 { logf(e.2) } else { lang.dict_logf(&e.1).unwrap_or(-6.0) };
                e.0 = -self.quick_cost(&e.3, &g) * self.params.w_geom + freq;
            }
            if pre.len() > STAGE2 {
                pre.select_nth_unstable_by(STAGE2, |a, b| b.0.total_cmp(&a.0));
                pre.truncate(STAGE2);
            }
            if std::env::var_os("GLISSANDO_SWIPE_PROFILE").is_some() {
                eprintln!("stage1 done at {:.2} ms", t0.elapsed().as_secs_f64() * 1000.0);
            }
            for (_, w, f, letters) in pre {
                let cost = self.shape_cost(&letters, &g);
                let freq = if f > 0 { logf(f) } else { lang.dict_logf(&w).unwrap_or(-6.0) };
                let ctx = context_bonus(lang, prev, &w);
                let mut punct = if w.contains(['\'', '’', '-']) { self.params.punct_penalty } else { 0.0 };
                if w.chars().count() > 1 && w.chars().all(|c| !c.is_lowercase()) {
                    punct += self.params.caps_penalty;
                }
                let score = -cost * self.params.w_geom + freq + lang.user.boost(&w) + ctx + prior - punct;
                scored.push(GestureCandidate { word: w, score, lang: li });
            }
        }
        scored.sort_by(|a, b| b.score.total_cmp(&a.score));
        let mut seen: Vec<String> = Vec::new();
        scored.retain(|c| {
            let k = c.word.to_lowercase();
            if seen.contains(&k) {
                false
            } else {
                seen.push(k);
                true
            }
        });
        scored.truncate(self.params.max_results);
        scored
    }
}

fn context_bonus(lang: &Language, prev: Option<&str>, word: &str) -> f32 {
    let Some(prev) = prev else { return 0.0 };
    let mut b = 0.0;
    if let Some(d) = &lang.dict
        && let (Some((p, _)), Some((w, _))) = (d.lookup_any_case(prev), d.lookup(word))
        && let Some(f) = d.bigram(p, w)
    {
        b += 0.5 + f as f32 / 255.0 * 2.0;
    }
    let c = lang.user.bigram(prev, word);
    if c > 0 {
        b += 0.6 + (c as f32).ln() * 0.6;
    }
    b
}

/// Synthesise a plausible human gesture for `word` (tests and benchmarks):
/// a smoothed polyline through the key centres with seeded noise.
pub fn synth_gesture(word: &str, keys: &KeyMap, noise: f32, seed: u64) -> Vec<Pt> {
    let mut rng = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    let mut rnd = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        ((rng >> 11) as f64 / (1u64 << 53) as f64) as f32 * 2.0 - 1.0
    };
    let Some(letters) = word_keys(word, keys) else { return Vec::new() };
    let pts: Vec<Pt> = letters.iter().map(|p| Pt { x: p.x + rnd() * noise, y: p.y + rnd() * noise * 0.8 }).collect();
    let mut out = Vec::new();
    if pts.len() == 1 {
        return vec![pts[0], Pt { x: pts[0].x + 0.05, y: pts[0].y }];
    }
    // Corner cutting: humans do not hit key centres exactly.
    for w in pts.windows(2) {
        let steps = ((w[0].d(w[1]) / 0.15).ceil() as usize).max(2);
        for s in 0..steps {
            let t = s as f32 / steps as f32;
            let sag = (t * std::f32::consts::PI).sin() * noise * 0.5 * rnd();
            out.push(Pt { x: w[0].x + (w[1].x - w[0].x) * t + sag, y: w[0].y + (w[1].y - w[0].y) * t + sag });
        }
    }
    out.push(*pts.last().unwrap());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use glissando_lang::dict::DictBuilder;
    use glissando_lang::user::UserModel;

    pub fn qwerty() -> KeyMap {
        let rows = ["qwertyuiop", "asdfghjkl", "zxcvbnm"];
        let offs = [0.0, 0.5, 1.5];
        let mut keys = Vec::new();
        for (r, row) in rows.iter().enumerate() {
            for (i, c) in row.chars().enumerate() {
                keys.push((c, offs[r] + i as f32 + 0.5, r as f32 * 1.4 + 0.7));
            }
        }
        KeyMap::new(keys)
    }

    fn lang(words: &[(&str, u8)]) -> Language {
        let mut b = DictBuilder::default();
        for (w, f) in words {
            b.add_word(w, *f);
        }
        Language::new("en", Some(Dict::from_bytes(b.build()).unwrap()), UserModel::in_memory())
    }

    #[test]
    fn resample_is_equidistant() {
        let r = resample(&[Pt { x: 0.0, y: 0.0 }, Pt { x: 10.0, y: 0.0 }], 11);
        assert_eq!(r.len(), 11);
        assert!((r[5].x - 5.0).abs() < 1e-3);
    }

    #[test]
    fn decodes_clean_gestures() {
        let keys = qwerty();
        let words = [
            ("hello", 200),
            ("help", 190),
            ("world", 190),
            ("word", 200),
            ("the", 230),
            ("they", 210),
            ("this", 220),
            ("keyboard", 150),
            ("good", 200),
            ("food", 170),
            ("quick", 160),
            ("brown", 150),
        ];
        let l = vec![lang(&words)];
        let dec = ShapeDecoder::default();
        for (w, _) in words {
            for seed in 0..3 {
                let g = synth_gesture(w, &keys, 0.18, seed);
                let r = dec.decode(&g, &keys, &l, &[(0, 0.0)], None);
                assert!(!r.is_empty(), "{w}: no result");
                let top3: Vec<&str> = r.iter().take(3).map(|c| c.word.as_str()).collect();
                assert!(top3.contains(&w), "{w} (seed {seed}) not in {top3:?}");
            }
        }
    }

    #[test]
    fn cyrillic_and_folded_letters() {
        let rows = ["йцукенгшщзх", "фывапролджэ", "ячсмитьбю"];
        let offs = [0.0, 0.0, 1.0];
        let mut k = Vec::new();
        for (r, row) in rows.iter().enumerate() {
            for (i, c) in row.chars().enumerate() {
                k.push((c, offs[r] + i as f32 + 0.5, r as f32 * 1.4 + 0.7));
            }
        }
        let keys = KeyMap::new(k);
        let l = vec![lang(&[("привет", 200), ("ещё", 200), ("подъезд", 150), ("пока", 190)])];
        let dec = ShapeDecoder::default();
        for w in ["привет", "ещё", "подъезд"] {
            let g = synth_gesture(w, &keys, 0.12, 7);
            let r = dec.decode(&g, &keys, &l, &[(0, 0.0)], None);
            assert_eq!(r.first().map(|c| c.word.as_str()), Some(w));
        }
    }
}
