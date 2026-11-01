//! The typing engine: turns key presses into text operations while keeping
//! the composing word, suggestions, auto-correction, smart punctuation and
//! personalisation in sync. It knows nothing about Wayland; the caller
//! applies the returned [`Op`]s (see `glissando::im`).

use crate::model::{CandKind, Candidate, Language, LanguageDetector, Ranker};
use crate::search::{InChar, SpatialModel};
use glissando_core::text::{
    CasePattern, apply_case, case_pattern, chars_for_words_back, is_word_char, previous_words, sentence_start,
    tail_bytes, trailing_word,
};
use std::collections::VecDeque;

/// A text operation for the client application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Commit(String),
    /// Delete text before the cursor. `bytes` is valid when the context is known.
    DeleteBefore {
        chars: usize,
        bytes: usize,
    },
    /// The composing text (preedit) after this batch.
    Preedit(String),
    Key(Key),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Backspace,
    Enter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Purpose {
    #[default]
    Normal,
    Alpha,
    Digits,
    Number,
    Phone,
    Url,
    Email,
    Name,
    Password,
    Pin,
    Date,
    Time,
    Datetime,
    Terminal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FieldInfo {
    pub purpose: Purpose,
    /// Passwords and other sensitive input: no suggestions, no learning.
    pub sensitive: bool,
    pub multiline: bool,
    /// The client asked for lowercase / all caps / title case.
    pub lowercase: bool,
    pub uppercase: bool,
    pub titlecase: bool,
    /// The client explicitly allows or disables completion/spellcheck.
    pub no_completion: bool,
}

impl FieldInfo {
    pub fn private(&self) -> bool {
        self.sensitive || matches!(self.purpose, Purpose::Password | Purpose::Pin)
    }

    fn wants_correction(&self) -> bool {
        !self.private()
            && !self.no_completion
            && matches!(self.purpose, Purpose::Normal | Purpose::Alpha | Purpose::Name)
    }

    fn wants_suggestions(&self) -> bool {
        !self.private()
            && !self.no_completion
            && !matches!(
                self.purpose,
                Purpose::Digits | Purpose::Number | Purpose::Phone | Purpose::Pin | Purpose::Terminal
            )
    }

    fn wants_autocap(&self) -> bool {
        !self.private() && !self.lowercase && matches!(self.purpose, Purpose::Normal | Purpose::Alpha | Purpose::Name)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub suggestions: bool,
    pub autocorrect: bool,
    pub autocap: bool,
    pub autospace: bool,
    pub double_space_period: bool,
    pub next_word: bool,
    pub learn: bool,
    pub preedit: bool,
    pub autospace_after_punct: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            suggestions: true,
            autocorrect: true,
            autocap: true,
            autospace: true,
            double_space_period: true,
            next_word: true,
            learn: true,
            preedit: true,
            autospace_after_punct: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestionKind {
    /// Exactly what was typed.
    Verbatim,
    Word,
    Prediction,
    Swipe,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    pub text: String,
    pub kind: SuggestionKind,
    /// Will replace the typed word on space/punctuation.
    pub autocorrect: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Composing {
    Typed,
    Swipe,
}

#[derive(Debug, Clone, PartialEq)]
struct Undo {
    /// What we inserted (including any trailing separator).
    inserted: String,
    original: String,
    lang: Option<usize>,
}

pub struct Engine {
    pub langs: Vec<Language>,
    /// Indices of languages active for the current layout.
    active: Vec<usize>,
    detector: LanguageDetector,
    spatial: SpatialModel,
    pub settings: Settings,
    field: FieldInfo,
    composing: String,
    inputs: Vec<InChar>,
    kind: Composing,
    before: String,
    after: String,
    context_known: bool,
    /// Text around the cursor after each of our recent edits, oldest first.
    /// A late surrounding-text report that matches one of them is an echo
    /// of our own typing, not an edit by the user.
    sent: VecDeque<(String, String)>,
    /// The client has a selection (anchor != cursor).
    selection: bool,
    /// The next surrounding-text report follows an edit whose result we
    /// cannot predict (typing over a selection) and is taken as it is.
    trust_next: bool,
    suggestions: Vec<Suggestion>,
    undo: Option<Undo>,
    auto_space: bool,
    last_was_space: bool,
    /// Manual shift/caps state of the composing word is kept in `composing`.
    swipe_shift: CasePattern,
}

const CONTEXT_KEEP: usize = 300;
const SENT_KEEP: usize = 32;
/// Shorter contexts that agree on this many characters are taken as the same
/// text cut at different points (clients send windows of varying size).
const CONTEXT_MATCH: usize = 64;
/// Characters on each side of the composing word that must match to take
/// it as committed in place.
const WORD_CONTEXT: usize = 16;

fn is_sentence_punct(s: &str) -> bool {
    matches!(s, "." | "," | "!" | "?" | ";" | ":" | "…")
}

impl Engine {
    pub fn new(langs: Vec<Language>, settings: Settings) -> Engine {
        let n = langs.len();
        Engine {
            langs,
            active: (0..n.min(1)).collect(),
            detector: LanguageDetector::new(n.min(1)),
            spatial: SpatialModel::default(),
            settings,
            field: FieldInfo::default(),
            composing: String::new(),
            inputs: Vec::new(),
            kind: Composing::Typed,
            before: String::new(),
            after: String::new(),
            context_known: false,
            sent: VecDeque::new(),
            selection: false,
            trust_next: false,
            suggestions: Vec::new(),
            undo: None,
            auto_space: false,
            last_was_space: false,
            swipe_shift: CasePattern::Lower,
        }
    }

    /// Choose the languages used with the current layout and its key positions.
    pub fn set_layout(&mut self, codes: &[String], spatial: SpatialModel) -> Vec<Op> {
        let ops = self.commit_composing();
        self.active = codes.iter().filter_map(|c| self.langs.iter().position(|l| &l.code == c)).collect();
        self.detector.reset(self.active.len());
        self.spatial = spatial;
        self.refresh_suggestions();
        ops
    }

    pub fn active_languages(&self) -> Vec<&str> {
        self.active.iter().map(|&i| self.langs[i].code.as_str()).collect()
    }

    pub fn language_probabilities(&self) -> Vec<(String, f32)> {
        self.active.iter().zip(self.detector.probs()).map(|(&i, &p)| (self.langs[i].code.clone(), p)).collect()
    }

    pub fn field(&self) -> FieldInfo {
        self.field
    }

    /// A new text field got focus (or its type changed).
    pub fn set_field(&mut self, field: FieldInfo) {
        self.field = field;
        self.reset();
    }

    /// Forget all transient state and the text context (focus change).
    pub fn reset(&mut self) {
        self.drop_composing();
        self.before.clear();
        self.after.clear();
        self.context_known = false;
        self.sent.clear();
        self.selection = false;
        self.trust_next = false;
    }

    /// The user edited the text or moved the cursor: whatever the client did
    /// with the preedit, the composing word is gone.
    fn drop_composing(&mut self) {
        self.composing.clear();
        self.inputs.clear();
        self.undo = None;
        self.auto_space = false;
        self.last_was_space = false;
        self.refresh_suggestions();
    }

    /// Surrounding text reported by the client; `selection` when the anchor
    /// differs from the cursor. Returns true when the report shows an edit
    /// or cursor move by the user rather than an echo of our own typing.
    ///
    /// The client's text change cause is not consulted: Qt never sets it,
    /// so every change claims to come from the input method, and a missed
    /// cursor move makes the next key continue the old word at the new
    /// position ("hel" left behind, "help" typed elsewhere). The text itself
    /// is compared with what our own edits must have produced instead; late
    /// reports of earlier states are ignored.
    pub fn set_surrounding(&mut self, before: &str, after: &str, selection: bool) -> bool {
        self.selection = selection;
        if !self.context_known || std::mem::take(&mut self.trust_next) {
            self.take_context(before, after);
            return false;
        }
        if same_before(before, &self.before) && same_after(after, &self.after) {
            if before.len() > self.before.len() {
                self.before = keep_tail(before);
            }
            if after.len() > self.after.len() {
                self.after = after.chars().take(CONTEXT_KEEP).collect();
            }
            return false;
        }
        if self.sent.iter().any(|(b, a)| same_before(before, b) && same_after(after, a)) {
            return false;
        }
        // The text changed while composing. Whether the word is gone depends
        // on who changed it: Qt commits the preedit in place on a click, and
        // phoc commits it on every touch on the client. GTK keeps showing it
        // while it deletes a selection as the preedit starts, or while the
        // app replaces the text, and the next key must extend the word, not
        // replace it ("m", then "A" instead of "ma").
        let committed = !self.composing.is_empty() && self.committed_in_place(before, after);
        self.take_context(before, after);
        if self.composing.is_empty() || committed {
            self.drop_composing();
        }
        true
    }

    /// Does the reported text hold the composing word as committed text
    /// where the cursor was?
    fn committed_in_place(&self, before: &str, after: &str) -> bool {
        let n = self.before.chars().count();
        let left: String = self.before.chars().skip(n.saturating_sub(WORD_CONTEXT)).collect();
        let right: String = self.after.chars().take(WORD_CONTEXT).collect();
        let word = format!("{left}{}{right}", self.composing);
        format!("{before}{after}").contains(&word)
    }

    fn take_context(&mut self, before: &str, after: &str) {
        self.before = keep_tail(before);
        self.after = after.chars().take(CONTEXT_KEEP).collect();
        self.context_known = true;
        self.sent.clear();
        self.remember_sent();
    }

    fn remember_sent(&mut self) {
        if self.sent.back().map(|(b, a)| *b == self.before && *a == self.after).unwrap_or(false) {
            return;
        }
        if self.sent.len() >= SENT_KEEP {
            self.sent.pop_front();
        }
        self.sent.push_back((self.before.clone(), self.after.clone()));
    }

    pub fn composing(&self) -> &str {
        &self.composing
    }

    pub fn suggestions(&self) -> &[Suggestion] {
        &self.suggestions
    }

    pub fn before(&self) -> &str {
        &self.before
    }

    /// Should the shift key be engaged automatically right now?
    pub fn autocap(&self) -> bool {
        if !self.settings.autocap || !self.field.wants_autocap() || !self.composing.is_empty() {
            return false;
        }
        if self.field.uppercase {
            return true;
        }
        if self.field.titlecase || self.field.purpose == Purpose::Name {
            let t = self.before.chars().last();
            return t.map(|c| c.is_whitespace()).unwrap_or(true);
        }
        sentence_start(&self.before)
    }

    fn private(&self) -> bool {
        self.field.private()
    }

    fn use_preedit(&self) -> bool {
        self.settings.preedit && !self.private() && self.field.purpose != Purpose::Terminal
    }

    // ---- local context bookkeeping -------------------------------------

    fn note_commit(&mut self, s: &str) {
        self.before.push_str(s);
        self.before = keep_tail(&self.before);
        self.remember_sent();
    }

    fn note_delete(&mut self, chars: usize) -> usize {
        let bytes = tail_bytes(&self.before, chars);
        let keep = self.before.len() - bytes;
        self.before.truncate(keep);
        self.remember_sent();
        bytes
    }

    fn delete_before(&mut self, chars: usize, ops: &mut Vec<Op>) {
        if chars == 0 {
            return;
        }
        let bytes = self.note_delete(chars);
        ops.push(Op::DeleteBefore { chars, bytes });
    }

    fn commit(&mut self, s: &str, ops: &mut Vec<Op>) {
        if s.is_empty() {
            return;
        }
        self.note_commit(s);
        match ops.last_mut() {
            Some(Op::Commit(prev)) => prev.push_str(s),
            _ => ops.push(Op::Commit(s.to_string())),
        }
    }

    fn finish_batch(&mut self, mut ops: Vec<Op>) -> Vec<Op> {
        if self.use_preedit() && !self.composing.is_empty() {
            ops.push(Op::Preedit(self.composing.clone()));
        } else if self.use_preedit() {
            ops.push(Op::Preedit(String::new()));
        }
        self.refresh_suggestions();
        ops
    }

    // ---- language helpers ------------------------------------------------

    fn ranker_active(&self) -> Vec<(usize, f32)> {
        let pri = self.detector.log_priors();
        self.active.iter().enumerate().map(|(k, &i)| (i, pri.get(k).copied().unwrap_or(0.0))).collect()
    }

    fn prev_word(&self) -> Option<String> {
        previous_words(&self.before, 1).pop()
    }

    fn known_anywhere(&self, w: &str) -> bool {
        self.active.iter().any(|&i| self.langs[i].knows(w))
    }

    fn learn(&mut self, word: &str, prev: Option<&str>, lang: Option<usize>) {
        if !self.settings.learn || self.private() || word.is_empty() {
            return;
        }
        let known: Vec<bool> = self.active.iter().map(|&i| self.langs[i].knows(word)).collect();
        self.detector.observe(&known);
        let target =
            lang.or_else(|| self.active.iter().zip(&known).find(|(_, k)| **k).map(|(i, _)| *i)).or_else(|| {
                let probs = self.detector.probs();
                self.active.iter().zip(probs).max_by(|a, b| a.1.total_cmp(b.1)).map(|(i, _)| *i)
            });
        if let Some(t) = target {
            // Store words as typed unless they are sentence-initial capitals of a lowercase word.
            let stored = if case_pattern(word) == CasePattern::Capitalized
                && !self.langs[t].dict.as_ref().map(|d| d.lookup(word).is_some()).unwrap_or(false)
                && self.langs[t].knows(&word.to_lowercase())
            {
                word.to_lowercase()
            } else {
                word.to_string()
            };
            self.langs[t].user.learn(&stored, prev);
        }
    }

    pub fn save_user_data(&mut self) {
        for l in self.langs.iter_mut() {
            if let Err(e) = l.user.save() {
                eprintln!("glissando: saving personal dictionary failed: {e}");
            }
        }
    }

    pub fn clear_user_data(&mut self) {
        for l in self.langs.iter_mut() {
            l.user.clear();
        }
    }

    pub fn user_data_dirty(&self) -> bool {
        self.langs.iter().any(|l| l.user.is_dirty())
    }

    /// Add a word to the personal dictionary of the most likely language.
    pub fn pin_word(&mut self, word: &str) {
        if let Some(&i) = self.active.first() {
            self.langs[i].user.pin(word);
        }
    }

    pub fn block_word(&mut self, word: &str) {
        for &i in &self.active.clone() {
            self.langs[i].user.block(word);
        }
        self.refresh_suggestions();
    }

    // ---- suggestions -----------------------------------------------------

    fn refresh_suggestions(&mut self) {
        self.suggestions.clear();
        if !self.settings.suggestions || !self.field.wants_suggestions() {
            return;
        }
        if self.composing.is_empty() {
            if self.settings.next_word && !self.before.is_empty() && !self.before.ends_with('\n') {
                let ends_ws = self.before.ends_with(char::is_whitespace);
                if ends_ws {
                    if let Some(prev) = self.prev_word() {
                        let ranker = Ranker { langs: &self.langs, active: &self.ranker_active() };
                        let cap = self.autocap();
                        self.suggestions = ranker
                            .predictions(&prev, 3)
                            .into_iter()
                            .map(|c| Suggestion {
                                text: if cap { apply_case(&c.text, CasePattern::Capitalized) } else { c.text },
                                kind: SuggestionKind::Prediction,
                                autocorrect: false,
                            })
                            .collect();
                    }
                }
            }
            return;
        }
        if self.kind == Composing::Swipe {
            return; // set by `swipe`
        }
        let pattern = case_pattern(&self.composing);
        let prev = self.prev_word();
        let ranker = Ranker { langs: &self.langs, active: &self.ranker_active() };
        let cands = ranker.candidates(&self.inputs, prev.as_deref(), 6);
        let typed = self.composing.clone();
        let typed_known = self.known_anywhere(&typed);
        let mut list: Vec<Suggestion> = Vec::new();
        let corr = self.settings.autocorrect && self.field.wants_correction();
        let best = cands.first();
        let mut auto: Option<String> = None;
        if let Some(b) = best {
            let bt = case_for(&b.text, pattern);
            if corr && bt != typed && should_autocorrect(&typed, typed_known, b, cands.get(1), &cands) {
                auto = Some(bt);
            }
        }
        if let Some(a) = &auto {
            list.push(Suggestion { text: a.clone(), kind: SuggestionKind::Word, autocorrect: true });
            list.push(Suggestion { text: typed.clone(), kind: SuggestionKind::Verbatim, autocorrect: false });
        } else {
            list.push(Suggestion { text: typed.clone(), kind: SuggestionKind::Verbatim, autocorrect: false });
        }
        for c in &cands {
            let t = case_for(&c.text, pattern);
            if list.iter().any(|s| s.text.to_lowercase() == t.to_lowercase()) {
                continue;
            }
            list.push(Suggestion { text: t, kind: SuggestionKind::Word, autocorrect: false });
            if list.len() >= 3 {
                break;
            }
        }
        // Display order: the strongest candidate first. If the verbatim
        // word is unknown and something else ranks higher, show that first.
        if auto.is_none() && !typed_known && list.len() > 1 {
            if let Some(b) = best {
                if b.kind != CandKind::Exact {
                    list.swap(0, 1);
                }
            }
        }
        self.suggestions = list;
    }

    // ---- key handling ----------------------------------------------------

    /// Commit the composing word as is (cursor moves, layout switches...).
    pub fn commit_composing(&mut self) -> Vec<Op> {
        let mut ops = Vec::new();
        if !self.composing.is_empty() {
            let w = std::mem::take(&mut self.composing);
            self.inputs.clear();
            if self.use_preedit() {
                self.commit(&w, &mut ops);
                ops.push(Op::Preedit(String::new()));
            }
            let prev = self.prev_word_before(&w);
            self.learn(&w, prev.as_deref(), None);
        }
        self.undo = None;
        self.auto_space = false;
        self.refresh_suggestions();
        ops
    }

    /// Previous word before the word that was just appended to `before`.
    fn prev_word_before(&self, word: &str) -> Option<String> {
        let b = self.before.strip_suffix(word).unwrap_or(&self.before);
        previous_words(b, 1).pop()
    }

    /// Finish the composing word, optionally auto-correcting it. Returns the final word.
    fn finish_word(&mut self, correct: bool, ops: &mut Vec<Op>) -> Option<String> {
        if self.composing.is_empty() {
            return None;
        }
        let typed = std::mem::take(&mut self.composing);
        self.inputs.clear();
        let mut out = typed.clone();
        if correct {
            if let Some(s) = self.suggestions.iter().find(|s| s.autocorrect) {
                out = s.text.clone();
            }
        }
        if self.use_preedit() {
            self.commit(&out, ops);
        } else if out != typed {
            // Direct mode: the typed word is already in the document.
            let n = typed.chars().count();
            self.delete_before(n, ops);
            self.commit(&out, ops);
        }
        let prev = self.prev_word_before(&out);
        if out != typed {
            self.undo = Some(Undo { inserted: out.clone(), original: typed, lang: None });
        } else {
            self.undo = None;
        }
        self.learn(&out, prev.as_deref(), None);
        Some(out)
    }

    /// Insert text verbatim (emoji, clipboard): finishes the composing word first.
    pub fn insert_raw(&mut self, text: &str) -> Vec<Op> {
        let mut ops = self.commit_composing();
        // commit_composing ends with a preedit reset; append after it.
        self.commit(text, &mut ops);
        self.undo = None;
        self.auto_space = false;
        self.last_was_space = false;
        self.refresh_suggestions();
        ops
    }

    /// A text key was tapped. `text` is already cased; `touch` is the touch
    /// point in key units for the spatial model.
    pub fn text(&mut self, text: &str, touch: Option<(f32, f32)>) -> Vec<Op> {
        let mut ops = Vec::new();
        self.last_was_space = false;
        let mut chars = text.chars();
        let single = match (chars.next(), chars.next()) {
            (Some(c), None) => Some(c),
            _ => None,
        };
        let wordish = single.map(|c| is_word_char(c) && c != '-').unwrap_or(false);
        if self.kind == Composing::Swipe && !self.composing.is_empty() {
            if wordish {
                // Typing right after a glide starts a new word.
                self.finish_word(false, &mut ops);
                self.commit(" ", &mut ops);
                self.auto_space = false;
            }
        }
        if self.selection && self.composing.is_empty() {
            // The client replaces the selection with whatever we send.
            self.trust_next = true;
        }
        if wordish && !self.private() && self.field.purpose != Purpose::Terminal {
            let c = single.unwrap();
            if self.composing.is_empty() {
                self.kind = Composing::Typed;
                self.undo = None;
                self.auto_space = false;
            }
            self.composing.push(c);
            self.inputs.push(self.spatial.input_char(c, touch));
            if !self.use_preedit() {
                self.commit(&c.to_string(), &mut ops);
            }
            return self.finish_batch(ops);
        }
        // Punctuation, symbols, digits in private fields...
        let sep = is_sentence_punct(text);
        let correct = sep && text != "'" && self.settings.autocorrect && self.field.wants_correction();
        let had_word = self.finish_word(correct, &mut ops).is_some();
        if sep && self.auto_space && self.before.ends_with(' ') && !had_word {
            // "word |" + "," => "word, |"
            self.delete_before(1, &mut ops);
            self.commit(text, &mut ops);
            self.commit(" ", &mut ops);
            self.auto_space = true;
        } else {
            self.commit(text, &mut ops);
            if sep && self.settings.autospace_after_punct && matches!(text, "." | "," | "!" | "?" | ";" | ":") {
                self.commit(" ", &mut ops);
                self.auto_space = true;
            } else {
                self.auto_space = false;
            }
        }
        if !had_word {
            self.undo = None;
        } else if let Some(u) = &mut self.undo {
            u.inserted.push_str(text);
        }
        self.finish_batch(ops)
    }

    pub fn space(&mut self) -> Vec<Op> {
        let mut ops = Vec::new();
        let correct = self.settings.autocorrect && self.field.wants_correction() && self.kind == Composing::Typed;
        let had_word = self.finish_word(correct, &mut ops).is_some();
        if !had_word
            && self.settings.double_space_period
            && (self.last_was_space || self.auto_space)
            && self.field.wants_correction()
            && double_space_applies(&self.before)
        {
            self.delete_before(1, &mut ops);
            self.commit(". ", &mut ops);
            self.last_was_space = false;
            self.auto_space = false;
            self.undo = None;
            return self.finish_batch(ops);
        }
        self.commit(" ", &mut ops);
        if let Some(u) = &mut self.undo {
            u.inserted.push(' ');
        }
        if !had_word {
            self.undo = None;
        }
        self.auto_space = false;
        self.last_was_space = true;
        self.finish_batch(ops)
    }

    pub fn enter(&mut self) -> Vec<Op> {
        let mut ops = Vec::new();
        self.finish_word(false, &mut ops);
        self.undo = None;
        self.auto_space = false;
        self.last_was_space = false;
        if self.use_preedit() {
            ops.push(Op::Preedit(String::new()));
        }
        ops.push(Op::Key(Key::Enter));
        if self.field.multiline {
            self.note_commit("\n");
        }
        self.refresh_suggestions();
        ops
    }

    pub fn backspace(&mut self) -> Vec<Op> {
        let mut ops = Vec::new();
        self.last_was_space = false;
        if self.selection && self.composing.is_empty() {
            return self.delete_selection();
        }
        if !self.composing.is_empty() {
            if self.kind == Composing::Swipe {
                // A glided word is removed as a whole.
                let n = self.composing.chars().count();
                self.composing.clear();
                if !self.use_preedit() {
                    self.delete_before(n, &mut ops);
                }
            } else {
                self.composing.pop();
                self.inputs.pop();
                if !self.use_preedit() {
                    self.delete_before(1, &mut ops);
                }
            }
            return self.finish_batch(ops);
        }
        if let Some(u) = self.undo.take() {
            if self.before.ends_with(&u.inserted) && self.context_ok() {
                // Revert the auto-correction and keep the original word.
                let n = u.inserted.chars().count();
                self.delete_before(n, &mut ops);
                for &i in &self.active.clone() {
                    self.langs[i].user.insist(&u.original);
                }
                let _ = u.lang;
                self.recompose(&u.original, &mut ops);
                return self.finish_batch(ops);
            }
        }
        if self.auto_space && self.before.ends_with(' ') {
            self.auto_space = false;
            self.delete_before(1, &mut ops);
            let w = trailing_word(&self.before).to_string();
            if !w.is_empty() && self.use_preedit() && self.context_ok() {
                let n = w.chars().count();
                self.delete_before(n, &mut ops);
                self.recompose(&w, &mut ops);
            }
            return self.finish_batch(ops);
        }
        // Backspace into a word re-opens it for editing.
        if self.use_preedit() && self.context_ok() && self.after_is_boundary() {
            let w = trailing_word(&self.before).to_string();
            if w.chars().count() >= 2 && self.before.ends_with(&w) {
                let n = w.chars().count();
                self.delete_before(n, &mut ops);
                let mut w2 = w.clone();
                w2.pop();
                self.recompose(&w2, &mut ops);
                return self.finish_batch(ops);
            }
        }
        self.note_delete(1);
        if self.use_preedit() && !ops.is_empty() {
            ops.push(Op::Preedit(String::new()));
        }
        ops.push(Op::Key(Key::Backspace));
        self.refresh_suggestions();
        ops
    }

    fn context_ok(&self) -> bool {
        self.context_known
    }

    /// Backspace with a selection deletes the selection: a plain key event,
    /// never delete_surrounding_text (Qt drops the whole update then).
    fn delete_selection(&mut self) -> Vec<Op> {
        self.undo = None;
        self.auto_space = false;
        self.trust_next = true;
        let mut ops = Vec::new();
        if self.use_preedit() {
            ops.push(Op::Preedit(String::new()));
        }
        ops.push(Op::Key(Key::Backspace));
        self.refresh_suggestions();
        ops
    }

    fn after_is_boundary(&self) -> bool {
        self.after.chars().next().map(|c| !is_word_char(c)).unwrap_or(true)
    }

    fn recompose(&mut self, word: &str, _ops: &mut [Op]) {
        self.composing = word.to_string();
        self.kind = Composing::Typed;
        self.inputs = word.chars().map(|c| self.spatial.input_char(c, None)).collect();
        self.undo = None;
        self.auto_space = false;
    }

    /// Delete `n` words before the cursor (backspace glide).
    pub fn delete_words(&mut self, n: u32) -> Vec<Op> {
        let mut ops = Vec::new();
        if self.selection && self.composing.is_empty() {
            return self.delete_selection();
        }
        if !self.composing.is_empty() {
            let had = self.composing.chars().count();
            self.composing.clear();
            self.inputs.clear();
            if !self.use_preedit() {
                self.delete_before(had, &mut ops);
            }
            if n <= 1 {
                return self.finish_batch(ops);
            }
            return self.delete_words_known(n - 1, ops);
        }
        self.delete_words_known(n, ops)
    }

    fn delete_words_known(&mut self, n: u32, mut ops: Vec<Op>) -> Vec<Op> {
        self.undo = None;
        self.auto_space = false;
        if self.context_ok() {
            let chars = chars_for_words_back(&self.before, n);
            self.delete_before(chars, &mut ops);
        } else {
            for _ in 0..n {
                ops.push(Op::Key(Key::Backspace));
            }
        }
        self.finish_batch(ops)
    }

    /// Preview for the backspace glide: the text that would be deleted.
    pub fn delete_preview(&self, n: u32) -> String {
        let mut s = self.before.clone();
        s.push_str(&self.composing);
        let chars = chars_for_words_back(&s, n);
        let start = s.len() - tail_bytes(&s, chars);
        s[start..].to_string()
    }

    /// A suggestion from the strip was tapped.
    pub fn pick(&mut self, idx: usize) -> Vec<Op> {
        let mut ops = Vec::new();
        let Some(s) = self.suggestions.get(idx).cloned() else { return ops };
        let space = self.settings.autospace && !matches!(self.field.purpose, Purpose::Url | Purpose::Email);
        if self.composing.is_empty() {
            // Next word prediction.
            if !self.before.is_empty() && !self.before.ends_with(char::is_whitespace) {
                self.commit(" ", &mut ops);
            }
            let prev = self.prev_word();
            self.commit(&s.text, &mut ops);
            self.learn(&s.text, prev.as_deref(), None);
        } else {
            let typed = std::mem::take(&mut self.composing);
            self.inputs.clear();
            if !self.use_preedit() {
                let n = typed.chars().count();
                self.delete_before(n, &mut ops);
            }
            self.commit(&s.text, &mut ops);
            let prev = self.prev_word_before(&s.text);
            if s.kind == SuggestionKind::Verbatim {
                for &i in &self.active.clone() {
                    self.langs[i].user.insist(&s.text);
                }
            }
            self.learn(&s.text, prev.as_deref(), None);
        }
        self.undo = None;
        if space && !self.after.starts_with(' ') {
            self.commit(" ", &mut ops);
            self.auto_space = true;
        }
        self.kind = Composing::Typed;
        self.last_was_space = false;
        self.finish_batch(ops)
    }

    /// Result of a glide. `cands` are words ranked best first. `shift`
    /// is the case pattern requested by the shift key.
    pub fn swipe(&mut self, cands: Vec<String>, shift: CasePattern) -> Vec<Op> {
        let mut ops = Vec::new();
        if cands.is_empty() {
            return ops;
        }
        let was_swipe = self.kind == Composing::Swipe && !self.composing.is_empty();
        if !self.composing.is_empty() {
            self.finish_word(!was_swipe && self.settings.autocorrect && self.field.wants_correction(), &mut ops);
            self.commit(" ", &mut ops);
        } else if self
            .before
            .chars()
            .last()
            .map(|c| is_word_char(c) || matches!(c, '.' | ',' | '!' | '?' | ';' | ':'))
            .unwrap_or(false)
        {
            self.commit(" ", &mut ops);
        }
        self.auto_space = false;
        self.last_was_space = false;
        self.undo = None;
        let pattern = if shift == CasePattern::Lower && self.autocap_ignoring_composing() {
            CasePattern::Capitalized
        } else {
            shift
        };
        self.swipe_shift = pattern;
        let cased: Vec<String> = cands.iter().map(|c| case_for(c, pattern)).collect();
        self.kind = Composing::Swipe;
        self.composing = cased[0].clone();
        self.inputs.clear();
        if !self.use_preedit() {
            let c = self.composing.clone();
            self.commit(&c, &mut ops);
        }
        let mut ops = self.finish_batch(ops);
        self.suggestions = cased
            .iter()
            .take(3)
            .map(|t| Suggestion { text: t.clone(), kind: SuggestionKind::Swipe, autocorrect: false })
            .collect();
        if self.use_preedit() {
            // finish_batch already queued the preedit.
        }
        ops.retain(|_| true);
        ops
    }

    fn autocap_ignoring_composing(&self) -> bool {
        self.settings.autocap && self.field.wants_autocap() && sentence_start(&self.before)
    }

    /// Replace the glided word with another candidate (strip tap after a glide).
    pub fn swipe_alternatives(&self) -> &[Suggestion] {
        &self.suggestions
    }

    /// Words to feed into a swipe decoder for context (most recent last).
    pub fn context_words(&self, n: usize) -> Vec<String> {
        previous_words(&self.before, n)
    }

    pub fn language(&self, i: usize) -> &Language {
        &self.langs[i]
    }

    /// Active languages with their current log priors (for external decoders).
    pub fn active_priors(&self) -> Vec<(usize, f32)> {
        self.ranker_active()
    }

    pub fn active_indices(&self) -> &[usize] {
        &self.active
    }

    pub fn log_priors(&self) -> Vec<f32> {
        self.detector.log_priors()
    }

    pub fn spatial(&self) -> &SpatialModel {
        &self.spatial
    }

    /// Candidate list for unit tests and diagnostics.
    pub fn rank(&self, word: &str) -> Vec<Candidate> {
        let input: Vec<InChar> = word.chars().map(|c| self.spatial.input_char(c, None)).collect();
        let ranker = Ranker { langs: &self.langs, active: &self.ranker_active() };
        ranker.candidates(&input, None, 6)
    }
}

/// Do two views of the text before the cursor agree? Either may be cut off
/// at the start: we keep a tail, clients send a window (or the line).
fn same_before(a: &str, b: &str) -> bool {
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let Some(rest) = long.strip_suffix(short) else { return false };
    rest.is_empty() || rest.ends_with('\n') || short.chars().count() >= CONTEXT_MATCH
}

/// Same for the text after the cursor, which may be cut off at the end.
fn same_after(a: &str, b: &str) -> bool {
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let Some(rest) = long.strip_prefix(short) else { return false };
    rest.is_empty() || rest.starts_with('\n') || short.chars().count() >= CONTEXT_MATCH
}

fn keep_tail(s: &str) -> String {
    let n = s.chars().count();
    if n <= CONTEXT_KEEP { s.to_string() } else { s.chars().skip(n - CONTEXT_KEEP).collect() }
}

fn double_space_applies(before: &str) -> bool {
    let mut it = before.chars().rev();
    it.next() == Some(' ')
        && it.next().map(|c| c.is_alphanumeric() || matches!(c, ')' | '"' | '\'' | '»')).unwrap_or(false)
}

/// Apply the typed word's case pattern to a dictionary word, keeping
/// dictionary capitals (proper nouns, German nouns).
fn case_for(word: &str, pattern: CasePattern) -> String {
    match pattern {
        CasePattern::Lower => word.to_string(),
        p => apply_case(word, p),
    }
}

fn should_autocorrect(
    typed: &str,
    typed_known: bool,
    best: &Candidate,
    second: Option<&Candidate>,
    all: &[Candidate],
) -> bool {
    let n = typed.chars().count();
    if n < 2 || typed.chars().any(|c| c.is_ascii_digit()) {
        return false;
    }
    if best.kind == CandKind::Completion && best.cost > 1.5 {
        return false;
    }
    // A word typed in lowercase is never turned into a different proper noun
    // or acronym (only its own capitalisation may be fixed).
    if case_pattern(typed) == CasePattern::Lower
        && case_pattern(&best.text) != CasePattern::Lower
        && best.text.to_lowercase() != typed.to_lowercase()
    {
        return false;
    }
    if typed_known {
        // Only fix capitalisation of known words (e.g. "berlin" -> "Berlin")
        // or near-certain misspellings that happen to be rare words.
        if best.text.to_lowercase() == typed.to_lowercase() {
            return best.text != typed && case_pattern(typed) == CasePattern::Lower && best.kind == CandKind::Exact;
        }
        let typed_score = all
            .iter()
            .find(|c| c.text.to_lowercase() == typed.to_lowercase())
            .map(|c| c.score)
            .unwrap_or(f32::NEG_INFINITY);
        return best.score - typed_score > 6.0 && best.cost < 3.0;
    }
    // Two letters are too little evidence unless the target is a very common word.
    if n == 2 && best.score < -2.0 {
        return false;
    }
    // Unknown word: correct when the best candidate is clearly ahead.
    let margin = second.map(|s| best.score - s.score).unwrap_or(9.0);
    let max_cost = if n <= 3 { 2.6 } else { 5.0 };
    best.cost <= max_cost && (margin > 0.4 || best.cost < 1.5)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dict::test_dict;
    use crate::user::UserModel;

    fn qwerty() -> SpatialModel {
        let rows = ["qwertyuiop", "asdfghjkl", "zxcvbnm"];
        let offs = [0.0, 0.5, 1.5];
        let mut keys = Vec::new();
        for (r, row) in rows.iter().enumerate() {
            for (i, c) in row.chars().enumerate() {
                keys.push((c, offs[r] + i as f32 + 0.5, r as f32 * 1.3 + 0.65));
            }
        }
        SpatialModel::new(keys)
    }

    fn engine() -> Engine {
        let en = Language::new(
            "en",
            Some(test_dict(
                &[
                    ("hello", 200),
                    ("help", 190),
                    ("the", 230),
                    ("there", 200),
                    ("world", 190),
                    ("good", 200),
                    ("morning", 180),
                    ("I", 230),
                    ("am", 210),
                    ("in", 220),
                    ("Berlin", 150),
                ],
                &[("good", "morning", 220)],
            )),
            UserModel::in_memory(),
        );
        let mut e = Engine::new(vec![en], Settings::default());
        e.set_layout(&["en".to_string()], qwerty());
        e.set_surrounding("", "", false);
        e
    }

    /// Apply ops to a simple document model like a well behaved client.
    #[derive(Default)]
    struct Doc {
        text: String,
        preedit: String,
    }

    impl Doc {
        fn apply(&mut self, ops: &[Op]) {
            for op in ops {
                match op {
                    Op::Commit(s) => {
                        self.preedit.clear();
                        self.text.push_str(s)
                    }
                    Op::DeleteBefore { bytes, .. } => {
                        self.preedit.clear();
                        let l = self.text.len() - bytes;
                        self.text.truncate(l);
                    }
                    Op::Preedit(p) => self.preedit = p.clone(),
                    Op::Key(Key::Backspace) => {
                        self.text.pop();
                    }
                    Op::Key(Key::Enter) => self.text.push('\n'),
                }
            }
        }
        fn shown(&self) -> String {
            format!("{}{}", self.text, self.preedit)
        }
    }

    fn type_str(e: &mut Engine, d: &mut Doc, s: &str) {
        for c in s.chars() {
            let ops = match c {
                ' ' => e.space(),
                '\u{8}' => e.backspace(),
                '\n' => e.enter(),
                c => e.text(&c.to_string(), None),
            };
            d.apply(&ops);
        }
    }

    #[test]
    fn composes_and_autocorrects() {
        let mut e = engine();
        let mut d = Doc::default();
        assert!(e.autocap());
        type_str(&mut e, &mut d, "Helo");
        assert_eq!(d.preedit, "Helo");
        assert_eq!(e.suggestions()[0].text, "Hello");
        assert!(e.suggestions()[0].autocorrect);
        type_str(&mut e, &mut d, " ");
        assert_eq!(d.shown(), "Hello ");
        // Backspace reverts the correction.
        type_str(&mut e, &mut d, "\u{8}");
        assert_eq!(d.shown(), "Helo");
        assert_eq!(e.composing(), "Helo");
        type_str(&mut e, &mut d, " ");
        assert_eq!(d.shown(), "Helo ", "insisted word is not corrected again");
    }

    #[test]
    fn smart_punctuation_after_pick() {
        let mut e = engine();
        let mut d = Doc::default();
        type_str(&mut e, &mut d, "wor");
        let idx = e.suggestions().iter().position(|s| s.text == "world").unwrap();
        d.apply(&e.pick(idx));
        assert_eq!(d.shown(), "world ");
        type_str(&mut e, &mut d, ",");
        assert_eq!(d.shown(), "world, ");
    }

    #[test]
    fn double_space_period_and_autocap() {
        let mut e = engine();
        let mut d = Doc::default();
        type_str(&mut e, &mut d, "I am  ");
        assert_eq!(d.shown(), "I am. ");
        assert!(e.autocap());
    }

    #[test]
    fn next_word_prediction() {
        let mut e = engine();
        let mut d = Doc::default();
        type_str(&mut e, &mut d, "good ");
        assert_eq!(e.suggestions()[0].text, "morning");
        d.apply(&e.pick(0));
        assert_eq!(d.shown(), "good morning ");
    }

    #[test]
    fn backspace_recomposes_previous_word() {
        let mut e = engine();
        let mut d = Doc::default();
        type_str(&mut e, &mut d, "hello ");
        assert!(!e.set_surrounding(&d.text.clone(), "", false));
        // Manual space: plain backspace removes it.
        type_str(&mut e, &mut d, "\u{8}");
        assert_eq!(d.shown(), "hello");
        assert!(!e.set_surrounding(&d.text.clone(), "", false));
        type_str(&mut e, &mut d, "\u{8}");
        assert_eq!(d.shown(), "hell");
        assert_eq!(e.composing(), "hell");
    }

    #[test]
    fn swipe_flow() {
        let mut e = engine();
        let mut d = Doc::default();
        d.apply(&e.swipe(vec!["hello".into(), "help".into()], CasePattern::Lower));
        assert_eq!(d.shown(), "Hello");
        d.apply(&e.swipe(vec!["world".into()], CasePattern::Lower));
        assert_eq!(d.shown(), "Hello world");
        type_str(&mut e, &mut d, ".");
        assert_eq!(d.shown(), "Hello world.");
        d.apply(&e.swipe(vec!["there".into()], CasePattern::Lower));
        assert_eq!(d.shown(), "Hello world. There");
        // Backspace removes the whole glided word.
        type_str(&mut e, &mut d, "\u{8}");
        assert_eq!(d.shown(), "Hello world. ");
    }

    #[test]
    fn password_fields_are_private() {
        let mut e = engine();
        e.set_field(FieldInfo { purpose: Purpose::Password, ..Default::default() });
        let mut d = Doc::default();
        type_str(&mut e, &mut d, "helo ");
        assert_eq!(d.shown(), "helo ");
        assert!(e.suggestions().is_empty());
        assert!(e.langs[0].user.words.is_empty(), "nothing learned from passwords");
        assert!(!e.autocap());
    }

    #[test]
    fn learns_new_words() {
        let mut e = engine();
        let mut d = Doc::default();
        for _ in 0..2 {
            type_str(&mut e, &mut d, "glissando");
            let i = e.suggestions().iter().position(|s| s.kind == SuggestionKind::Verbatim).unwrap();
            d.apply(&e.pick(i));
        }
        type_str(&mut e, &mut d, "gliss");
        assert!(e.suggestions().iter().any(|s| s.text == "glissando"));
    }

    #[test]
    fn proper_noun_capitalisation() {
        let mut e = engine();
        let mut d = Doc::default();
        type_str(&mut e, &mut d, "I am in berlin ");
        assert_eq!(d.shown(), "I am in Berlin ");
    }

    #[test]
    fn no_autocorrect_into_proper_nouns() {
        let ru = Language::new(
            "ru",
            Some(test_dict(
                &[("Ле", 150), ("де", 60), ("дела", 200), ("день", 210), ("как", 230)],
                &[("как", "дела", 240)],
            )),
            UserModel::in_memory(),
        );
        let rows = ["йцукенгшщзх", "фывапролджэ", "ячсмитьбю"];
        let mut keys = Vec::new();
        for (r, row) in rows.iter().enumerate() {
            for (i, c) in row.chars().enumerate() {
                keys.push((c, i as f32 + 0.5 + r as f32 * 0.5, r as f32 * 1.35 + 0.65));
            }
        }
        let mut e = Engine::new(vec![ru], Settings::default());
        e.set_layout(&["ru".to_string()], SpatialModel::new(keys));
        e.set_surrounding("", "", false);
        let mut d = Doc::default();
        type_str(&mut e, &mut d, "как де");
        assert!(!e.suggestions().iter().any(|s| s.autocorrect && s.text == "Ле"), "{:?}", e.suggestions());
        assert!(e.suggestions().iter().any(|s| s.text == "дела"), "{:?}", e.suggestions());
    }

    #[test]
    fn cursor_move_without_change_cause_ends_the_word() {
        // Qt: the click commits the preedit where it was and moves the
        // cursor; the report claims to come from the input method.
        let mut e = engine();
        let mut d = Doc::default();
        type_str(&mut e, &mut d, "good hel");
        assert!(!e.set_surrounding("good ", "", false));
        assert_eq!(e.composing(), "hel");
        assert!(e.set_surrounding("", "good hel", false), "cursor moved to the start");
        assert_eq!(e.composing(), "");
        let ops = e.text("x", None);
        assert_eq!(ops, vec![Op::Preedit("x".into())]);
    }

    #[test]
    fn late_echoes_are_not_edits() {
        let mut e = engine();
        let mut d = Doc::default();
        type_str(&mut e, &mut d, "hello wor");
        // Reports for states we already went past arrive late.
        assert!(!e.set_surrounding("", "", false));
        assert!(!e.set_surrounding("hello", "", false));
        assert_eq!(e.composing(), "wor");
        assert!(!e.set_surrounding("hello ", "", false));
        type_str(&mut e, &mut d, "ld ");
        assert_eq!(d.shown(), "hello world ");
    }

    #[test]
    fn surrounding_text_may_be_cut() {
        let mut e = engine();
        let mut d = Doc::default();
        let long = "x".repeat(400) + " ";
        e.set_surrounding(&long, "", false);
        type_str(&mut e, &mut d, "wor");
        assert!(!e.set_surrounding(&long[200..], "", false), "shorter window");
        assert_eq!(e.composing(), "wor");
    }

    #[test]
    fn preedit_committed_by_the_compositor_ends_the_word() {
        // phoc commits the preedit when the client is touched (the map
        // next to a search field) without telling the input method.
        let mut e = engine();
        let mut d = Doc::default();
        type_str(&mut e, &mut d, "good hel");
        assert!(e.set_surrounding("good hel", "", false));
        assert_eq!(e.composing(), "");
        assert_eq!(e.text("p", None), vec![Op::Preedit("p".into())]);
    }

    #[test]
    fn text_changed_around_the_preedit_keeps_the_word() {
        // GTK selects the text of a focused search field without reporting
        // it and deletes the selection when the preedit starts.
        let mut e = engine();
        let mut d = Doc::default();
        e.set_surrounding("ZI-Klinik für abhängiges Verhalten", "", false);
        type_str(&mut e, &mut d, "m");
        assert!(e.set_surrounding("", "", false));
        assert_eq!(e.composing(), "m");
        assert_eq!(e.text("a", None), vec![Op::Preedit("ma".into())]);
        // The app replaces the text around the preedit.
        assert!(e.set_surrounding("Mannheim", "", false));
        assert_eq!(e.composing(), "ma");
    }

    #[test]
    fn backspace_with_selection_is_a_key() {
        let mut e = engine();
        let mut d = Doc::default();
        type_str(&mut e, &mut d, "hello ");
        assert!(!e.set_surrounding("hello ", "", true), "same text, now selected");
        assert_eq!(e.backspace(), vec![Op::Preedit(String::new()), Op::Key(Key::Backspace)]);
        // Whatever the client reports next is the new context.
        assert!(!e.set_surrounding("he", "", false));
    }

    #[test]
    fn delete_words_glide() {
        let mut e = engine();
        let mut d = Doc::default();
        type_str(&mut e, &mut d, "hello world ");
        assert_eq!(e.delete_preview(1), "world ");
        d.apply(&e.delete_words(1));
        assert_eq!(d.shown(), "hello ");
    }
}
