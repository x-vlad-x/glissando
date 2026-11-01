//! Combines dictionaries, personal models and context into ranked
//! candidates for one or more simultaneously active languages.

use crate::dict::Dict;
use crate::search::{InChar, SearchOpts, search};
use crate::user::UserModel;
use glissando_core::text::fold_char;

pub struct Language {
    pub code: String,
    pub dict: Option<Dict>,
    pub user: UserModel,
}

impl Language {
    pub fn new(code: &str, dict: Option<Dict>, user: UserModel) -> Language {
        Language { code: code.to_string(), dict, user }
    }

    /// Dictionary log frequency of `word` (any case for the first letter).
    pub fn dict_logf(&self, word: &str) -> Option<f32> {
        let d = self.dict.as_ref()?;
        d.lookup_any_case(word).map(|(_, f)| logf(f))
    }

    /// Whether the word is part of this language's vocabulary.
    pub fn knows(&self, word: &str) -> bool {
        if self.user.is_blocked(word) {
            return false;
        }
        self.dict_logf(word).is_some()
            || self.user.known(word).is_some()
            || self.user.known(&word.to_lowercase()).is_some()
    }

    fn context_bonus(&self, prev: Option<&str>, word: &str) -> f32 {
        let Some(prev) = prev else { return 0.0 };
        let mut bonus = 0.0;
        if let Some(d) = &self.dict
            && let (Some((p, _)), Some((w, _))) = (d.lookup_any_case(prev), d.lookup(word))
            && let Some(f) = d.bigram(p, w)
        {
            bonus += 0.6 + f as f32 / 255.0 * 2.4;
        }
        let c = self.user.bigram(prev, word);
        if c > 0 {
            bonus += 0.8 + (c as f32).ln() * 0.8;
        }
        bonus
    }
}

/// Dictionary frequency (AOSP 0..=255 log scale) as a log score.
pub fn logf(f: u8) -> f32 {
    (f as f32 - 255.0) / 18.0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandKind {
    Exact,
    Correction,
    Completion,
    Prediction,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub text: String,
    pub score: f32,
    /// Index into the language list.
    pub lang: usize,
    pub kind: CandKind,
    /// Edit/spatial cost relative to the input (0 for predictions).
    pub cost: f32,
}

/// Weighted edit distance of `input` against `word`, used for the small
/// personal dictionary. `allow_prefix` makes trailing word characters free
/// (completion), returning the number of completed characters.
pub fn fuzzy_cost(input: &[InChar], word: &str, allow_prefix: bool) -> Option<(f32, usize)> {
    let costs = crate::search::Costs::default();
    let w: Vec<char> = word.chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect();
    let n = input.len();
    let m = w.len();
    if m + 3 < n || (!allow_prefix && n + 3 < m) {
        return None;
    }
    let inf = f32::INFINITY;
    let mut dp = vec![vec![inf; m + 1]; n + 1];
    dp[0][0] = 0.0;
    for i in 0..=n {
        for j in 0..=m {
            let cur = dp[i][j];
            if cur == inf {
                continue;
            }
            if i < n && j < m {
                let mc = input[i]
                    .alts
                    .iter()
                    .find(|a| a.0 == w[j] || a.0 == fold_char(w[j]))
                    .map(|a| a.1 + if a.0 == w[j] { 0.0 } else { costs.diacritic })
                    .unwrap_or(costs.substitute);
                dp[i + 1][j + 1] = dp[i + 1][j + 1].min(cur + mc);
            }
            if i < n {
                let rep = i > 0 && input[i].typed == input[i - 1].typed;
                dp[i + 1][j] = dp[i + 1][j].min(cur + if rep { costs.double } else { costs.insert });
            }
            if j < m {
                let dbl = j > 0 && w[j] == w[j - 1];
                dp[i][j + 1] = dp[i][j + 1].min(cur + if dbl { costs.double } else { costs.omit });
            }
        }
    }
    let mut best = (dp[n][m], 0usize);
    if allow_prefix && n > 0 {
        #[allow(clippy::needless_range_loop)] // `j` is also the cost of the rest of the word
        for j in n.saturating_sub(1)..m {
            let c = dp[n][j] + costs.complete + costs.complete_per_char * (m - j) as f32;
            if c < best.0 {
                best = (c, m - j);
            }
        }
    }
    (best.0 < 9.0).then_some(best)
}

pub struct Ranker<'a> {
    pub langs: &'a [Language],
    /// (language index, log prior)
    pub active: &'a [(usize, f32)],
}

impl Ranker<'_> {
    /// Candidates for a typed word. `typed` is the composing text as shown.
    pub fn candidates(&self, input: &[InChar], prev: Option<&str>, max: usize) -> Vec<Candidate> {
        let mut out: Vec<Candidate> = Vec::new();
        let opts = SearchOpts::for_len(input.len());
        for &(li, prior) in self.active {
            let lang = &self.langs[li];
            if let Some(d) = &lang.dict {
                for m in search(d, input, &opts) {
                    let word = d.word(m.node);
                    if lang.user.is_blocked(&word) {
                        continue;
                    }
                    let kind = if m.completion {
                        CandKind::Completion
                    } else if m.cost < 0.01 {
                        CandKind::Exact
                    } else {
                        CandKind::Correction
                    };
                    let score =
                        -m.cost + logf(m.freq) + lang.user.boost(&word) + lang.context_bonus(prev, &word) + prior;
                    out.push(Candidate { text: word, score, lang: li, kind, cost: m.cost });
                }
            }
            for (w, u) in lang.user.words.iter() {
                if u.blocked || !(u.pinned || u.count >= crate::user::LEARN_THRESHOLD) {
                    continue;
                }
                if let Some((cost, extra)) = fuzzy_cost(input, w, true) {
                    if cost > opts.max_cost + 1.0 {
                        continue;
                    }
                    let kind = if extra > 0 {
                        CandKind::Completion
                    } else if cost < 0.01 {
                        CandKind::Exact
                    } else {
                        CandKind::Correction
                    };
                    let base = lang.dict_logf(w).unwrap_or(-7.0);
                    let score = -cost + base + lang.user.boost(w) + lang.context_bonus(prev, w) + prior;
                    out.push(Candidate { text: w.clone(), score, lang: li, kind, cost });
                }
            }
        }
        dedup_sort(&mut out, max);
        out
    }

    /// Next-word predictions after `prev`.
    pub fn predictions(&self, prev: &str, max: usize) -> Vec<Candidate> {
        let mut out = Vec::new();
        for &(li, prior) in self.active {
            let lang = &self.langs[li];
            if let Some(d) = &lang.dict
                && let Some((p, _)) = d.lookup_any_case(prev)
            {
                for (n, f) in d.successors(p).into_iter().take(12) {
                    let w = d.word(n);
                    if lang.user.is_blocked(&w) {
                        continue;
                    }
                    let score = f as f32 / 255.0 * 3.0 + logf(d.node(n).freq) * 0.3 + lang.user.boost(&w) + prior;
                    out.push(Candidate { text: w, score, lang: li, kind: CandKind::Prediction, cost: 0.0 });
                }
            }
            for (w, c) in lang.user.successors(prev).into_iter().take(12) {
                let score = 1.0 + (c as f32).ln() * 1.2 + prior;
                out.push(Candidate { text: w, score, lang: li, kind: CandKind::Prediction, cost: 0.0 });
            }
        }
        dedup_sort(&mut out, max);
        out
    }
}

fn dedup_sort(v: &mut Vec<Candidate>, max: usize) {
    v.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut seen: Vec<String> = Vec::new();
    v.retain(|c| {
        let k = c.text.to_lowercase();
        if seen.contains(&k) {
            false
        } else {
            seen.push(k);
            true
        }
    });
    v.truncate(max);
}

/// Tracks which of several active languages the user is writing in.
#[derive(Debug, Clone)]
pub struct LanguageDetector {
    probs: Vec<f32>,
}

impl LanguageDetector {
    pub fn new(n: usize) -> LanguageDetector {
        LanguageDetector { probs: vec![1.0 / n.max(1) as f32; n] }
    }

    pub fn reset(&mut self, n: usize) {
        if self.probs.len() != n {
            *self = LanguageDetector::new(n);
        }
    }

    /// Update with a committed word; `known[i]` says whether language i knows it.
    pub fn observe(&mut self, known: &[bool]) {
        if known.iter().all(|k| *k) || known.iter().all(|k| !*k) {
            return;
        }
        for (p, k) in self.probs.iter_mut().zip(known) {
            *p = 0.75 * *p + 0.25 * if *k { 1.0 } else { 0.08 };
        }
        let s: f32 = self.probs.iter().sum();
        for p in self.probs.iter_mut() {
            *p /= s;
        }
    }

    /// Log priors relative to the uniform distribution.
    pub fn log_priors(&self) -> Vec<f32> {
        let n = self.probs.len() as f32;
        self.probs.iter().map(|p| (p * n).ln().max(-3.0) * 1.2).collect()
    }

    pub fn probs(&self) -> &[f32] {
        &self.probs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dict::test_dict;

    #[test]
    fn detector_shifts_towards_known_language() {
        let mut d = LanguageDetector::new(2);
        for _ in 0..4 {
            d.observe(&[false, true]);
        }
        assert!(d.probs()[1] > 0.8);
        let p = d.log_priors();
        assert!(p[1] > 0.0 && p[0] < 0.0);
    }

    #[test]
    fn multilingual_ranking_uses_prior() {
        let en = Language::new("en", Some(test_dict(&[("die", 120), ("the", 230)], &[])), UserModel::in_memory());
        let de = Language::new("de", Some(test_dict(&[("die", 240), ("der", 240)], &[])), UserModel::in_memory());
        let langs = vec![en, de];
        let input: Vec<InChar> = "die".chars().map(InChar::exact).collect();
        let r = Ranker { langs: &langs, active: &[(0, 0.0), (1, 0.0)] };
        let c = r.candidates(&input, None, 3);
        assert_eq!(c[0].text, "die");
        assert_eq!(c[0].lang, 1);
    }

    #[test]
    fn user_words_are_suggested() {
        let mut user = UserModel::in_memory();
        user.pin("Glissando");
        let en = Language::new("en", Some(test_dict(&[("glass", 150)], &[])), user);
        let langs = vec![en];
        let input: Vec<InChar> = "gliss".chars().map(InChar::exact).collect();
        let r = Ranker { langs: &langs, active: &[(0, 0.0)] };
        let c = r.candidates(&input, None, 3);
        assert_eq!(c[0].text, "Glissando");
        assert_eq!(c[0].kind, CandKind::Completion);
    }

    #[test]
    fn predictions_from_bigrams() {
        let en = Language::new(
            "en",
            Some(test_dict(
                &[("good", 200), ("morning", 180), ("night", 170)],
                &[("good", "morning", 200), ("good", "night", 150)],
            )),
            UserModel::in_memory(),
        );
        let langs = vec![en];
        let r = Ranker { langs: &langs, active: &[(0, 0.0)] };
        let p = r.predictions("Good", 3);
        assert_eq!(p[0].text, "morning");
        assert_eq!(p[1].text, "night");
    }
}
