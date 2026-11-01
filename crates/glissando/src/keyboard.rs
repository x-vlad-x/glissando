//! The keyboard controller: owns layouts, the touch machine, the typing
//! engine and the glide decoder, and turns touches into [`Action`]s for the
//! platform layer and a [`View`] for the renderer. Free of Wayland so it can
//! be tested headless.

use crate::icons::Icon;
use crate::panels::{PanelHit, PanelKind, PanelState};
use crate::render::{KeyLabel, KeyView, Popup, PopupEntry, StripItem, StripView, View, popup_layout};
use crate::theme::Metrics;
use glissando_config::{Config, OneHanded};
use glissando_core::geometry::{GeometryParams, LayerGeometry, Rect};
use glissando_core::layout::{KeyAction, KeyDef, KeyStyle, Layer, Layout, RowDef, parse_row};
use glissando_core::text::CasePattern;
use glissando_core::touch::{KeyKind, TouchConfig, TouchEvent, TouchMachine};
use glissando_lang::engine::{Engine, FieldInfo, Op, Purpose, SuggestionKind};
use glissando_lang::search::SpatialModel;
use glissando_swipe::{KeyMap, Pt, ShapeDecoder, clean_trace};

/// Vertical distance between rows in key widths on a typical phone layout
/// (row pitch / key pitch), used to normalise touch coordinates.
const ROW_ASPECT: f32 = 1.35;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavKey {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Tab,
    Escape,
}

/// Ctrl/Alt latched from the terminal keys for the next key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
}

impl Mods {
    pub fn any(&self) -> bool {
        self.ctrl || self.alt
    }
}

/// A key sent together with modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChordKey {
    Char(char),
    Nav(NavKey),
    Backspace,
    Enter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Haptic {
    Key,
    LongPress,
    Tick,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Text operations for the focused client.
    Ops(Vec<Op>),
    Nav(NavKey),
    /// A key with Ctrl and/or Alt held.
    Chord(ChordKey, Mods),
    Haptic(Haptic),
    Hide,
    OpenSettings,
    /// The desired surface height changed.
    Resize,
    /// Persist a configuration change made from the keyboard itself.
    SaveConfig,
    /// Paste the given clipboard history entry.
    Paste(String),
    /// Start local speech recognition in the given language ("auto" allowed).
    VoiceStart(String),
    VoiceStop,
    VoiceCancel,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VoiceUi {
    Listening(f32),
    Processing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shift {
    Off,
    Once,
    Locked,
}

#[derive(Debug, Clone, PartialEq)]
enum PopupItem {
    Text(String),
    Action(KeyAction),
    Layout(usize),
}

#[derive(Debug, Clone)]
struct OpenPopup {
    items: Vec<PopupItem>,
    popup: Popup,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Target {
    Keys,
    Strip(usize),
    Panel,
    Side(usize),
    Ignored,
}

/// One rendered key: the definition plus its geometry index.
#[derive(Debug, Clone)]
struct LiveKey {
    def: KeyDef,
}

pub struct Keyboard {
    pub config: Config,
    layouts: Vec<Layout>,
    current: usize,
    layer: String,
    shift: Shift,
    /// The user switched shift off while auto-capitalisation wanted it on.
    shift_manual_off: bool,
    mods: Mods,
    touch: TouchMachine,
    pub engine: Engine,
    decoder: ShapeDecoder,
    keymap: KeyMap,
    keys: Vec<LiveKey>,
    geom: LayerGeometry,
    width: f32,
    landscape: bool,
    metrics: Metrics,
    keys_rev: u64,
    pressed: Vec<usize>,
    preview: Option<(usize, String)>,
    popup: Option<OpenPopup>,
    targets: Vec<(u32, Target)>,
    strip_pressed: Option<usize>,
    strip_message: Option<(String, bool)>,
    trail: Vec<(f32, f32, u32)>,
    trail_until: u32,
    actions: Vec<Action>,
    im_active: bool,
    field: FieldInfo,
    symbol_typed: bool,
    pub panel: Option<PanelState>,
    /// Emoji search query while searching with the letter keys.
    emoji_search: Option<String>,
    voice: Option<VoiceUi>,
    /// Set by the platform when a speech model and engine are installed.
    pub voice_available: bool,
    side: Vec<Rect>,
    now: u32,
}

fn key_kind(k: &KeyDef) -> KeyKind {
    match &k.action {
        KeyAction::Text(t) => KeyKind::Text {
            letter: t.chars().count() == 1 && t.chars().all(|c| c.is_alphabetic()),
            popup: !k.popup_entries().is_empty(),
        },
        KeyAction::Space => KeyKind::Space,
        KeyAction::Backspace => KeyKind::Backspace,
        KeyAction::Shift => KeyKind::Shift,
        KeyAction::NextLanguage => KeyKind::Other { popup: true },
        KeyAction::Left | KeyAction::Right | KeyAction::Up | KeyAction::Down => KeyKind::Repeat,
        _ => KeyKind::Other { popup: !k.popup_entries().is_empty() },
    }
}

impl Keyboard {
    pub fn new(config: Config, layouts: Vec<Layout>, engine: Engine) -> Keyboard {
        assert!(!layouts.is_empty());
        let metrics = Metrics::new(false, config.height, config.font_scale);
        let mut kb = Keyboard {
            touch: TouchMachine::new(TouchConfig::default()),
            config,
            layouts,
            current: 0,
            layer: "letters".into(),
            shift: Shift::Off,
            shift_manual_off: false,
            mods: Mods::default(),
            engine,
            decoder: ShapeDecoder::default(),
            keymap: KeyMap::default(),
            keys: Vec::new(),
            geom: LayerGeometry::default(),
            width: 360.0,
            landscape: false,
            metrics,
            keys_rev: 1,
            pressed: Vec::new(),
            preview: None,
            popup: None,
            targets: Vec::new(),
            strip_pressed: None,
            strip_message: None,
            trail: Vec::new(),
            trail_until: 0,
            actions: Vec::new(),
            im_active: false,
            field: FieldInfo::default(),
            symbol_typed: false,
            panel: None,
            emoji_search: None,
            voice: None,
            voice_available: false,
            side: Vec::new(),
            now: 0,
        };
        kb.apply_config_to_touch();
        kb.select_layout(0);
        kb
    }

    fn apply_config_to_touch(&mut self) {
        let c = &self.config;
        let t = &mut self.touch.config;
        t.long_press_ms = c.long_press_ms;
        t.swipe_enabled = c.glide;
        t.space_cursor = c.glide_cursor;
        t.delete_glide = c.glide_delete;
        t.trackpad_on_space_long_press = c.trackpad_long_press_space;
    }

    pub fn set_config(&mut self, config: Config, layouts: Vec<Layout>) {
        let layout_id = self.layouts[self.current].id.clone();
        self.config = config;
        if !layouts.is_empty() {
            self.layouts = layouts;
        }
        self.apply_config_to_touch();
        let idx = self.layouts.iter().position(|l| l.id == layout_id).unwrap_or(0);
        self.metrics = Metrics::new(self.landscape, self.height_pref(), self.config.font_scale);
        self.select_layout(idx);
        self.actions.push(Action::Resize);
    }

    fn height_pref(&self) -> f32 {
        if self.landscape { self.config.height_landscape } else { self.config.height }
    }

    pub fn take_actions(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.actions)
    }

    /// Languages typed with the current layout: its own plus, in
    /// multilingual mode, those of other enabled layouts with the same script.
    fn layout_languages(&self) -> Vec<String> {
        let l = &self.layouts[self.current];
        let mut langs = l.languages.clone();
        if self.config.multilingual {
            let alpha = l.alphabet();
            let latin = alpha.iter().any(|c| c.is_ascii_alphabetic());
            for other in &self.layouts {
                let oa = other.alphabet();
                let other_latin = oa.iter().any(|c| c.is_ascii_alphabetic());
                if other.id != l.id && latin == other_latin {
                    for code in &other.languages {
                        if !langs.contains(code) {
                            langs.push(code.clone());
                        }
                    }
                }
            }
        }
        langs
    }

    pub fn select_layout(&mut self, idx: usize) {
        self.current = idx.min(self.layouts.len() - 1);
        self.layer = self.default_layer();
        self.rebuild();
        let spatial = SpatialModel::new(self.letter_centers());
        let langs = self.layout_languages();
        let ops = self.engine.set_layout(&langs, spatial);
        self.push_ops(ops);
        self.update_autoshift();
    }

    fn default_layer(&self) -> String {
        let name = match self.field.purpose {
            Purpose::Digits | Purpose::Pin => "digits",
            Purpose::Number => "numpad",
            Purpose::Phone => "phone",
            Purpose::Email => "letters@email",
            Purpose::Url => "letters@url",
            _ => "letters",
        };
        if self.layouts[self.current].layer(name).is_some() { name.to_string() } else { "letters".to_string() }
    }

    /// Surface position -> model units: x in key widths, y in row pitches
    /// scaled to a phone-like key aspect, so landscape (wide, low keys)
    /// does not make adjacent rows look closer than neighbouring keys.
    fn to_units(&self, x: f32, y: f32) -> (f32, f32) {
        let unit = self.geom.unit.max(1.0);
        let row = self.metrics.row_height.max(1.0);
        (x / unit, y / row * ROW_ASPECT)
    }

    fn letter_centers(&self) -> Vec<(char, f32, f32)> {
        self.keys
            .iter()
            .zip(&self.geom.keys)
            .filter_map(|(k, g)| {
                let c = k.def.action.letter()?;
                let (x, y) = g.visual.center();
                let (ux, uy) = self.to_units(x, y);
                Some((c, ux, uy))
            })
            .collect()
    }

    /// Current layer with runtime modifications (number row, language key).
    fn effective_layer(&self) -> Layer {
        let layout = &self.layouts[self.current];
        let mut layer = layout.layer(&self.layer).or_else(|| layout.layer("letters")).cloned().unwrap_or_default();
        let letters = self.layer.starts_with("letters");
        if letters && self.config.number_row {
            if let Some(first) = layer.rows.first_mut() {
                for k in first.keys.iter_mut() {
                    if k.hint.as_deref().map(|h| h.chars().all(|c| c.is_ascii_digit())).unwrap_or(false) {
                        k.hint = None;
                    }
                }
            }
            let units = layer.max_units();
            let mut row = parse_row("1 2 3 4 5 6 7 8 9 0").unwrap();
            let w = units / 10.0;
            for k in row.iter_mut() {
                k.width = w;
            }
            layer.rows.insert(0, RowDef { keys: row, height: 0.82 });
        }
        if self.terminal_row() && (letters || self.layer.starts_with("symbols")) {
            let mut row = parse_row("{esc} {tab} {ctrl} {alt} {left} {down} {up} {right}").unwrap();
            let w = layer.max_units() / row.len() as f32;
            for k in row.iter_mut() {
                k.width = w;
                k.label = match k.action {
                    KeyAction::Escape => Some("Esc".into()),
                    KeyAction::Ctrl => Some("Ctrl".into()),
                    KeyAction::Alt => Some("Alt".into()),
                    _ => None,
                };
            }
            layer.rows.insert(0, RowDef { keys: row, height: 0.82 });
        }
        if !self.config.hints {
            for r in layer.rows.iter_mut() {
                for k in r.keys.iter_mut() {
                    if k.action.letter().is_some() {
                        k.hint = None;
                    }
                }
            }
        }
        // The language key becomes the emoji key with a single layout.
        for r in layer.rows.iter_mut() {
            for k in r.keys.iter_mut() {
                if k.action == KeyAction::NextLanguage && self.layouts.len() < 2 {
                    k.action = KeyAction::Emoji;
                }
            }
        }
        layer
    }

    /// Esc, Tab, Ctrl, Alt and arrows above the keys in terminals (and when
    /// typing without an input-method client, which is mostly terminals too).
    fn terminal_row(&self) -> bool {
        self.config.terminal_keys && self.field.purpose == Purpose::Terminal
    }

    fn keys_area(&self) -> Rect {
        let top = self.metrics.strip_height;
        match self.config.one_handed {
            OneHanded::Off => Rect::new(0.0, top, self.width, 0.0),
            side => {
                let w = (self.width * 0.84).min(self.width - 48.0);
                let x = if side == OneHanded::Left { 0.0 } else { self.width - w };
                Rect::new(x, top, w, 0.0)
            }
        }
    }

    fn rebuild(&mut self) {
        let layer = self.effective_layer();
        let area = self.keys_area();
        let small = self.metrics.row_height < 46.0;
        let params = GeometryParams {
            area,
            row_height: self.metrics.row_height,
            gap_x: 6.0,
            gap_y: if small { 7.0 } else { 9.0 },
            pad_x: 3.0,
            pad_top: 3.0,
            pad_bottom: 5.0,
        };
        let geom = LayerGeometry::compute(&layer, &params);
        let defs: Vec<KeyDef> =
            layer.rows.iter().flat_map(|r| r.keys.iter()).filter(|k| k.style != KeyStyle::Invisible).cloned().collect();
        let kinds = defs.iter().map(key_kind).collect();
        let ev = self.touch.set_layer(geom.clone(), kinds);
        drop(ev);
        self.keys = defs.into_iter().map(|def| LiveKey { def }).collect();
        self.geom = geom;
        let unit = self.geom.unit.max(1.0);
        self.keymap = KeyMap::new(self.letter_centers());
        let _ = unit;
        self.pressed.clear();
        self.preview = None;
        self.popup = None;
        // Side buttons for one-handed mode.
        self.side.clear();
        if self.config.one_handed != OneHanded::Off {
            let top = self.metrics.strip_height;
            let h = self.geom.height;
            let (x, w) = if area.x > 0.0 { (0.0, area.x) } else { (area.right(), self.width - area.right()) };
            self.side.push(Rect::new(x, top + h * 0.15, w, h * 0.35));
            self.side.push(Rect::new(x, top + h * 0.5, w, h * 0.35));
        }
        self.keys_rev += 1;
    }

    pub fn resize(&mut self, width: f32, landscape: bool) {
        if (width - self.width).abs() < 0.5 && landscape == self.landscape {
            return;
        }
        self.width = width;
        self.landscape = landscape;
        self.metrics = Metrics::new(landscape, self.height_pref(), self.config.font_scale);
        self.rebuild();
        let spatial = SpatialModel::new(self.letter_centers());
        let langs = self.layout_languages();
        let ops = self.engine.set_layout(&langs, spatial);
        self.push_ops(ops);
    }

    pub fn desired_height(&self) -> f32 {
        (self.metrics.strip_height + self.geom.height).round()
    }

    // ---- input method state ----------------------------------------------

    pub fn activate(&mut self, field: FieldInfo) {
        self.im_active = true;
        self.field = field;
        self.engine.set_field(field);
        self.panel = None;
        self.emoji_search = None;
        self.layer = self.default_layer();
        self.shift = Shift::Off;
        self.shift_manual_off = false;
        self.mods = Mods::default();
        self.rebuild();
        self.update_autoshift();
        self.actions.push(Action::Resize);
    }

    pub fn deactivate(&mut self) {
        if self.voice.take().is_some() {
            self.actions.push(Action::VoiceCancel);
        }
        // The client has already dropped the preedit (GTK) or the compositor
        // committed it (phoc on a focus change). Typing it with virtual keys
        // would put the word into whatever has the focus now.
        self.engine.commit_composing();
        self.im_active = false;
        self.cancel_touches();
        self.engine.save_user_data();
    }

    /// Typing without an input-method client (shown manually): text goes
    /// through the virtual keyboard, so no composing or prediction.
    pub fn set_fallback_mode(&mut self) {
        self.im_active = false;
        self.field = FieldInfo { purpose: Purpose::Terminal, no_completion: true, ..Default::default() };
        self.engine.set_field(self.field);
        self.mods = Mods::default();
        self.layer = self.default_layer();
        self.rebuild();
        self.actions.push(Action::Resize);
    }

    pub fn surrounding(&mut self, before: &str, after: &str, selection: bool) {
        if self.engine.set_surrounding(before, after, selection) {
            self.update_autoshift();
        }
    }

    pub fn cancel_touches(&mut self) {
        let ev = self.touch.cancel_all();
        self.handle_events(ev);
        self.targets.clear();
        self.strip_pressed = None;
        self.popup = None;
        self.preview = None;
        self.pressed.clear();
    }

    // ---- shift -------------------------------------------------------------

    fn update_autoshift(&mut self) {
        let want = self.engine.autocap() && self.layer.starts_with("letters");
        let before = self.shift;
        match self.shift {
            Shift::Off if want && !self.shift_manual_off => self.shift = Shift::Once,
            Shift::Once if !want => self.shift = Shift::Off,
            _ => {}
        }
        if !want {
            self.shift_manual_off = false;
        }
        if before != self.shift {
            self.keys_rev += 1;
        }
    }

    fn set_shift(&mut self, s: Shift) {
        if s != self.shift {
            self.shift = s;
            self.keys_rev += 1;
        }
    }

    fn case_pattern(&self) -> CasePattern {
        match self.shift {
            Shift::Off => CasePattern::Lower,
            Shift::Once => CasePattern::Capitalized,
            Shift::Locked => CasePattern::Upper,
        }
    }

    fn cased(&self, t: &str) -> String {
        if self.shift != Shift::Off && t.chars().count() == 1 { t.to_uppercase() } else { t.to_string() }
    }

    // ---- actions -------------------------------------------------------------

    fn push_ops(&mut self, ops: Vec<Op>) {
        if !ops.is_empty() {
            self.actions.push(Action::Ops(ops));
        }
    }

    fn after_text(&mut self) {
        if self.shift == Shift::Once {
            self.set_shift(Shift::Off);
        }
        self.update_autoshift();
    }

    fn type_text(&mut self, text: &str, touch: Option<(f32, f32)>) {
        let t = self.cased(text);
        let ops = self.engine.text(&t, touch);
        self.push_ops(ops);
        if !self.layer.starts_with("letters") {
            self.symbol_typed = true;
            if text == "'" && self.layer == "symbols" {
                self.switch_layer("letters");
            }
        }
        self.after_text();
    }

    fn switch_layer(&mut self, name: &str) {
        let target = if name == "letters" { self.default_letters() } else { name.to_string() };
        if target != self.layer {
            self.layer = target;
            self.symbol_typed = false;
            self.rebuild();
            self.update_autoshift();
        }
    }

    fn default_letters(&self) -> String {
        let d = self.default_layer();
        if d.starts_with("letters") { d } else { "letters".into() }
    }

    fn next_layout(&mut self) {
        if self.layouts.len() > 1 {
            let n = (self.current + 1) % self.layouts.len();
            self.select_layout(n);
        }
    }

    /// Keys type into the emoji search query while searching.
    fn search_key(&mut self, action: &KeyAction) -> bool {
        let Some(q) = &mut self.emoji_search else { return false };
        match action {
            KeyAction::Text(t) => q.push_str(&t.to_lowercase()),
            KeyAction::Space => q.push(' '),
            KeyAction::Backspace => {
                if q.pop().is_none() {
                    self.emoji_search = None;
                    self.open_panel(PanelKind::Emoji);
                }
            }
            KeyAction::Enter | KeyAction::Hide => {
                self.emoji_search = None;
            }
            KeyAction::Shift | KeyAction::Layer(_) => return false,
            _ => {}
        }
        true
    }

    fn activate_key_action(&mut self, action: &KeyAction, touch: Option<(f32, f32)>) {
        if self.search_key(action) {
            return;
        }
        if self.mods.any()
            && let Some(key) = chord_key(action)
        {
            let ops = self.engine.commit_composing();
            self.push_ops(ops);
            self.actions.push(Action::Chord(key, self.mods));
            self.set_mods(Mods::default());
            return;
        }
        match action {
            KeyAction::Text(t) => self.type_text(&t.clone(), touch),
            KeyAction::Space => {
                let ops = self.engine.space();
                self.push_ops(ops);
                if self.symbol_typed && (self.layer == "symbols" || self.layer == "symbols2") {
                    self.switch_layer("letters");
                }
                self.after_text();
            }
            KeyAction::Backspace => {
                let ops = self.engine.backspace();
                self.push_ops(ops);
                self.update_autoshift();
            }
            KeyAction::Enter => {
                let ops = self.engine.enter();
                self.push_ops(ops);
                if self.shift == Shift::Once {
                    self.set_shift(Shift::Off);
                }
                self.update_autoshift();
            }
            KeyAction::Shift => {}
            KeyAction::Layer(l) => {
                let l = l.clone();
                self.switch_layer(&l);
            }
            KeyAction::NextLanguage => self.next_layout(),
            KeyAction::Emoji => self.open_panel(PanelKind::Emoji),
            KeyAction::Clipboard => self.open_panel(PanelKind::Clipboard),
            KeyAction::Settings => self.open_panel(PanelKind::Menu),
            KeyAction::OneHanded => self.toggle_one_handed(),
            KeyAction::Hide => self.actions.push(Action::Hide),
            KeyAction::Voice => {}
            KeyAction::Tab => self.nav(NavKey::Tab),
            KeyAction::Escape => self.nav(NavKey::Escape),
            KeyAction::Left => self.nav(NavKey::Left),
            KeyAction::Right => self.nav(NavKey::Right),
            KeyAction::Up => self.nav(NavKey::Up),
            KeyAction::Down => self.nav(NavKey::Down),
            KeyAction::Home => self.nav(NavKey::Home),
            KeyAction::End => self.nav(NavKey::End),
            KeyAction::Ctrl => self.set_mods(Mods { ctrl: !self.mods.ctrl, ..self.mods }),
            KeyAction::Alt => self.set_mods(Mods { alt: !self.mods.alt, ..self.mods }),
            KeyAction::Gap => {}
        }
    }

    fn set_mods(&mut self, m: Mods) {
        if m != self.mods {
            self.mods = m;
            self.keys_rev += 1;
        }
    }

    fn nav(&mut self, k: NavKey) {
        let ops = self.engine.commit_composing();
        self.push_ops(ops);
        self.actions.push(Action::Nav(k));
    }

    pub fn toggle_one_handed(&mut self) {
        self.config.one_handed = match self.config.one_handed {
            OneHanded::Off => OneHanded::Right,
            _ => OneHanded::Off,
        };
        self.rebuild();
        self.actions.push(Action::SaveConfig);
    }

    pub fn open_panel(&mut self, kind: PanelKind) {
        let ops = self.engine.commit_composing();
        self.push_ops(ops);
        let area = Rect::new(0.0, self.metrics.strip_height, self.width, self.geom.height);
        self.panel = Some(PanelState::new(kind, area, &self.config));
        self.keys_rev += 1;
    }

    pub fn close_panel(&mut self) {
        if self.panel.take().is_some() {
            self.keys_rev += 1;
        }
    }

    // ---- touch input ---------------------------------------------------------

    fn target_of(&self, id: u32) -> Option<Target> {
        self.targets.iter().find(|t| t.0 == id).map(|t| t.1)
    }

    fn strip_items(&self) -> Vec<(Rect, StripItem, StripTap)> {
        let h = self.metrics.strip_height;
        let w = self.width;
        let mut v = Vec::new();
        if self.panel.is_some() {
            v.push((Rect::new(0.0, 0.0, 52.0, h), StripItem::Icon(Icon::Keyboard), StripTap::ClosePanel));
            v.push((Rect::new(w - 52.0, 0.0, 52.0, h), StripItem::Icon(Icon::Hide), StripTap::Hide));
            return v;
        }
        if let Some(vs) = self.voice {
            let (msg, tap) = match vs {
                VoiceUi::Listening(level) => {
                    let bars = (level * 8.0).round() as usize;
                    (format!("{} {}", crate::i18n::tr("voice-listening"), "▮".repeat(bars.max(1))), StripTap::VoiceStop)
                }
                VoiceUi::Processing => (crate::i18n::tr("voice-processing").to_string(), StripTap::None),
            };
            v.push((Rect::new(0.0, 0.0, 52.0, h), StripItem::Icon(Icon::Mic), StripTap::VoiceStop));
            v.push((Rect::new(52.0, 0.0, w - 104.0, h), StripItem::Text { text: msg, emphasized: false }, tap));
            v.push((Rect::new(w - 52.0, 0.0, 52.0, h), StripItem::Icon(Icon::Close), StripTap::VoiceCancel));
            return v;
        }
        if let Some(q) = &self.emoji_search {
            v.push((Rect::new(0.0, 0.0, 44.0, h), StripItem::Icon(Icon::Left), StripTap::SearchBack));
            let qw = 116.0;
            v.push((
                Rect::new(44.0, 0.0, qw, h),
                StripItem::Text { text: format!("🔍 {q}▏"), emphasized: false },
                StripTap::None,
            ));
            let n = ((w - 44.0 - qw) / 44.0).floor().max(0.0) as usize;
            for (i, e) in crate::panels::search_emoji(q, n).into_iter().enumerate() {
                v.push((
                    Rect::new(44.0 + qw + i as f32 * 44.0, 0.0, 44.0, h),
                    StripItem::Text { text: e.clone(), emphasized: false },
                    StripTap::Emoji(e),
                ));
            }
            return v;
        }
        let sugg = self.engine.suggestions();
        // Emoji matching the word being typed get a narrow slot on the right.
        let word_emoji = if self.config.emoji_suggestions && !self.engine.composing().is_empty() {
            let best = sugg
                .iter()
                .find(|s| s.autocorrect)
                .map(|s| s.text.clone())
                .unwrap_or_else(|| self.engine.composing().to_string());
            crate::panels::emoji_for_word(&best).into_iter().next()
        } else {
            None
        };
        let w = if word_emoji.is_some() { w - 48.0 } else { w };
        if let Some(e) = &word_emoji {
            v.push((
                Rect::new(w, 0.0, 48.0, h),
                StripItem::Text { text: e.clone(), emphasized: false },
                StripTap::WordEmoji(e.clone()),
            ));
        }
        if !sugg.is_empty() {
            // Gboard order: second, best, third.
            let order: Vec<usize> = match sugg.len() {
                1 => vec![0],
                2 => vec![1, 0],
                _ => vec![1, 0, 2],
            };
            let n = order.len() as f32;
            let sw = w / n.max(3.0);
            let x0 = (w - sw * n) / 2.0;
            for (slot, &i) in order.iter().enumerate() {
                let s = &sugg[i];
                let emphasized = i == 0 && (s.autocorrect || s.kind == SuggestionKind::Swipe);
                let text = if s.kind == SuggestionKind::Verbatim && sugg.iter().any(|x| x.autocorrect) {
                    format!("“{}”", s.text)
                } else {
                    s.text.clone()
                };
                v.push((
                    Rect::new(x0 + slot as f32 * sw, 0.0, sw, h),
                    StripItem::Text { text, emphasized },
                    StripTap::Suggestion(i),
                ));
            }
            return v;
        }
        let tools = [
            (Icon::Menu, StripTap::Panel(PanelKind::Menu)),
            (Icon::Emoji, StripTap::Panel(PanelKind::Emoji)),
            (Icon::Clipboard, StripTap::Panel(PanelKind::Clipboard)),
            (Icon::OneHanded, StripTap::OneHanded),
        ];
        for (i, (icon, tap)) in tools.into_iter().enumerate() {
            v.push((Rect::new(4.0 + i as f32 * 50.0, 0.0, 50.0, h), StripItem::Icon(icon), tap));
        }
        v.push((Rect::new(w - 54.0, 0.0, 50.0, h), StripItem::Icon(Icon::Hide), StripTap::Hide));
        if self.voice_available && !self.field.private() {
            v.push((Rect::new(w - 104.0, 0.0, 50.0, h), StripItem::Icon(Icon::Mic), StripTap::Voice));
        }
        v
    }

    fn strip_tap(&mut self, tap: StripTap) {
        match tap {
            StripTap::Suggestion(i) => {
                let ops = self.engine.pick(i);
                self.push_ops(ops);
                self.after_text();
            }
            StripTap::Panel(k) => {
                if self.panel.as_ref().map(|p| p.kind) == Some(k) {
                    self.close_panel();
                } else {
                    self.open_panel(k);
                }
            }
            StripTap::ClosePanel => self.close_panel(),
            StripTap::Hide => self.actions.push(Action::Hide),
            StripTap::OneHanded => self.toggle_one_handed(),
            StripTap::SearchBack => {
                self.emoji_search = None;
                self.open_panel(PanelKind::Emoji);
            }
            StripTap::Emoji(e) => {
                crate::panels::RECENTS.with(|r| r.borrow_mut().used(&e));
                let ops = self.engine.insert_raw(&e);
                self.push_ops(ops);
            }
            StripTap::WordEmoji(e) => {
                let mut ops = self.engine.space();
                ops.extend(self.engine.insert_raw(&e));
                self.push_ops(ops);
                self.after_text();
            }
            StripTap::Voice => self.start_voice(),
            StripTap::VoiceStop => {
                if matches!(self.voice, Some(VoiceUi::Listening(_))) {
                    self.voice = Some(VoiceUi::Processing);
                    self.actions.push(Action::VoiceStop);
                }
            }
            StripTap::VoiceCancel => {
                self.voice = None;
                self.actions.push(Action::VoiceCancel);
            }
            StripTap::None => {}
        }
    }

    fn start_voice(&mut self) {
        let ops = self.engine.commit_composing();
        self.push_ops(ops);
        let lang = self
            .engine
            .language_probabilities()
            .into_iter()
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|l| l.0)
            .unwrap_or_else(|| "auto".into());
        self.voice = Some(VoiceUi::Listening(0.0));
        self.actions.push(Action::VoiceStart(lang));
    }

    /// A message from the speech helper (see glissando-voice).
    pub fn voice_event(&mut self, line: &str) {
        let (kind, rest) = line.split_once(' ').unwrap_or((line, ""));
        match kind {
            "level" => {
                if let Some(VoiceUi::Listening(_)) = self.voice {
                    self.voice = Some(VoiceUi::Listening(rest.parse().unwrap_or(0.0)));
                }
            }
            "processing" => {
                if self.voice.is_some() {
                    self.voice = Some(VoiceUi::Processing);
                }
            }
            "text" => {
                let was_active = self.voice.take().is_some();
                let t = rest.trim();
                if was_active && !t.is_empty() {
                    let before = self.engine.before().to_string();
                    let mut text = String::new();
                    if !before.is_empty() && !before.ends_with(char::is_whitespace) {
                        text.push(' ');
                    }
                    if self.engine.autocap() {
                        text.push_str(&glissando_core::text::capitalize(t));
                    } else {
                        text.push_str(t);
                    }
                    let ops = self.engine.insert_raw(&text);
                    self.push_ops(ops);
                    self.update_autoshift();
                }
            }
            "error" => {
                self.voice = None;
                self.strip_message = Some((rest.to_string(), true));
            }
            "end" => self.voice = None,
            _ => {}
        }
    }

    pub fn touch_down(&mut self, id: u32, x: f32, y: f32, t: u32) {
        self.now = t;
        let target = if let Some(i) = self.side.iter().position(|r| r.contains(x, y)) {
            Target::Side(i)
        } else if y < self.metrics.strip_height && !self.touch.is_active() {
            let items = self.strip_items();
            match items.iter().position(|(r, _, _)| r.contains(x, y)) {
                Some(i) => Target::Strip(i),
                None => Target::Ignored,
            }
        } else if self.panel.is_some() {
            Target::Panel
        } else {
            Target::Keys
        };
        self.targets.retain(|tg| tg.0 != id);
        self.targets.push((id, target));
        match target {
            Target::Keys => {
                self.strip_message = None;
                let ev = self.touch.down(id, x, y, t);
                self.handle_events(ev);
            }
            Target::Strip(i) => {
                self.strip_pressed = Some(i);
                self.actions.push(Action::Haptic(Haptic::Key));
            }
            Target::Panel => {
                if let Some(p) = &mut self.panel {
                    p.down(id, x, y, t);
                }
                self.actions.push(Action::Haptic(Haptic::Key));
            }
            Target::Side(_) => self.actions.push(Action::Haptic(Haptic::Key)),
            Target::Ignored => {}
        }
    }

    pub fn touch_motion(&mut self, id: u32, x: f32, y: f32, t: u32) {
        self.now = t;
        match self.target_of(id) {
            Some(Target::Keys) => {
                let ev = self.touch.motion(id, x, y, t);
                self.handle_events(ev);
            }
            Some(Target::Strip(i)) => {
                let items = self.strip_items();
                let inside = items.get(i).map(|(r, _, _)| r.inset(-8.0, -8.0).contains(x, y)).unwrap_or(false);
                self.strip_pressed = inside.then_some(i);
            }
            Some(Target::Panel) => {
                if let Some(p) = &mut self.panel {
                    p.motion(id, x, y, t);
                }
            }
            _ => {}
        }
    }

    pub fn touch_up(&mut self, id: u32, x: f32, y: f32, t: u32) {
        self.now = t;
        let target = self.target_of(id);
        self.targets.retain(|tg| tg.0 != id);
        match target {
            Some(Target::Keys) => {
                let ev = self.touch.up(id, t);
                self.handle_events(ev);
            }
            Some(Target::Strip(i)) => {
                let items = self.strip_items();
                if self.strip_pressed == Some(i)
                    && let Some((_, _, tap)) = items.get(i)
                {
                    let tap = tap.clone();
                    self.strip_tap(tap);
                }
                self.strip_pressed = None;
            }
            Some(Target::Panel) => {
                let hit = self.panel.as_mut().and_then(|p| p.up(id, x, y, t));
                if let Some(h) = hit {
                    self.panel_hit(h);
                }
            }
            Some(Target::Side(i))
                if self.side.get(i).map(|r| r.inset(-10.0, -10.0).contains(x, y)).unwrap_or(false) =>
            {
                if i == 0 {
                    self.config.one_handed = match self.config.one_handed {
                        OneHanded::Left => OneHanded::Right,
                        _ => OneHanded::Left,
                    };
                } else {
                    self.config.one_handed = OneHanded::Off;
                }
                self.rebuild();
                self.actions.push(Action::SaveConfig);
            }
            _ => {}
        }
    }

    pub fn touch_cancel(&mut self, id: u32) {
        let target = self.target_of(id);
        self.targets.retain(|tg| tg.0 != id);
        if target == Some(Target::Keys) {
            let ev = self.touch.cancel(id);
            self.handle_events(ev);
        }
        self.strip_pressed = None;
    }

    pub fn timer(&mut self, now: u32) {
        self.now = now;
        let ev = self.touch.timer(now);
        self.handle_events(ev);
        if let Some(p) = &mut self.panel {
            p.tick(now);
        }
    }

    pub fn next_deadline(&self) -> Option<u32> {
        let mut d = self.touch.next_deadline();
        if let Some(p) = &self.panel
            && let Some(pd) = p.next_deadline()
        {
            d = Some(d.map(|x| x.min(pd)).unwrap_or(pd));
        }
        d
    }

    /// True while something on screen still moves (trail fade-out, scrolling).
    pub fn animating(&self, now: u32) -> bool {
        now < self.trail_until || self.panel.as_ref().map(|p| p.animating()).unwrap_or(false)
    }

    fn panel_hit(&mut self, hit: PanelHit) {
        match hit {
            PanelHit::Commit(text) => {
                let ops = self.engine.insert_raw(&text);
                self.push_ops(ops);
            }
            PanelHit::Search => {
                self.close_panel();
                self.emoji_search = Some(String::new());
                self.switch_layer("letters");
            }
            PanelHit::Paste(text) => self.actions.push(Action::Paste(text)),
            PanelHit::Backspace => {
                let ops = self.engine.backspace();
                self.push_ops(ops);
            }
            PanelHit::Close => self.close_panel(),
            PanelHit::Config(c) => {
                let layouts = Vec::new();
                self.config = c;
                self.set_config(self.config.clone(), layouts);
                self.actions.push(Action::SaveConfig);
                if let Some(p) = &mut self.panel {
                    p.refresh(&self.config);
                }
            }
            PanelHit::OpenSettings => {
                self.close_panel();
                self.actions.push(Action::OpenSettings);
            }
            PanelHit::ClearPersonalData => {
                self.engine.clear_user_data();
                self.strip_message = Some(("✓".into(), false));
            }
            PanelHit::Redraw => {}
        }
    }

    fn popup_entries_for(&self, key: usize) -> Vec<PopupItem> {
        let def = &self.keys[key].def;
        match &def.action {
            KeyAction::NextLanguage | KeyAction::Space => {
                if self.layouts.len() < 2 && def.action == KeyAction::Space {
                    return Vec::new();
                }
                (0..self.layouts.len()).map(PopupItem::Layout).collect()
            }
            _ => def
                .popup_entries()
                .into_iter()
                .filter_map(|e| {
                    if let Some(name) = e.strip_prefix('{').and_then(|n| n.strip_suffix('}')) {
                        KeyAction::from_name(name).map(PopupItem::Action)
                    } else {
                        Some(PopupItem::Text(self.cased(&e)))
                    }
                })
                .collect(),
        }
    }

    fn open_popup(&mut self, key: usize) {
        let items = self.popup_entries_for(key);
        if items.is_empty() {
            return;
        }
        let anchor = self.geom.keys[key].visual;
        let cell_w = (anchor.w.min(self.geom.unit * 1.1)).max(self.geom.unit * 0.9).max(34.0);
        let cell_w = if items.iter().any(|i| matches!(i, PopupItem::Layout(_))) { cell_w.max(64.0) } else { cell_w };
        let cell_h = (self.metrics.row_height * 0.95).max(40.0);
        let bounds = Rect::new(0.0, 0.0, self.width, self.desired_height());
        let (rect, cells) = popup_layout(anchor, items.len(), bounds, cell_w, cell_h);
        let entries = items
            .iter()
            .map(|i| match i {
                PopupItem::Text(t) => PopupEntry::Text(t.clone()),
                PopupItem::Action(a) => PopupEntry::Icon(action_icon(a)),
                PopupItem::Layout(l) => PopupEntry::Text(self.layouts[*l].short.clone()),
            })
            .collect();
        let selected = match &self.keys[key].def.action {
            KeyAction::NextLanguage | KeyAction::Space => Some(self.current),
            _ => Some(0),
        };
        self.popup = Some(OpenPopup { items, popup: Popup { rect, cells, entries, selected } });
        self.preview = None;
    }

    fn popup_select(&mut self, x: f32, y: f32) {
        if let Some(p) = &mut self.popup {
            let r = p.popup.rect;
            // Generous vertical tolerance: the finger usually stays below the popup.
            let yy = y.clamp(r.y + 1.0, r.bottom() - 1.0);
            let best = p
                .popup
                .cells
                .iter()
                .enumerate()
                .min_by(|a, b| a.1.dist2(x, yy).total_cmp(&b.1.dist2(x, yy)))
                .map(|(i, _)| i);
            let far = y > r.bottom() + self.metrics.row_height * 1.5 || y < r.y - self.metrics.row_height;
            let sel = if far { None } else { best };
            if sel != p.popup.selected {
                p.popup.selected = sel;
                self.actions.push(Action::Haptic(Haptic::Tick));
            }
        }
    }

    fn popup_commit(&mut self) {
        let Some(p) = self.popup.take() else { return };
        let Some(sel) = p.popup.selected else { return };
        match p.items[sel].clone() {
            PopupItem::Text(t) => {
                let ops = self.engine.text(&t, None);
                self.push_ops(ops);
                self.after_text();
            }
            PopupItem::Action(a) => self.activate_key_action(&a, None),
            PopupItem::Layout(l) => {
                if l != self.current {
                    self.select_layout(l);
                }
            }
        }
    }

    /// Touch logs show which keys were pressed, so private fields stay out.
    fn input_loggable(&self) -> bool {
        !self.field.private()
    }

    pub fn log_input(&self) -> bool {
        self.input_loggable() && std::env::var_os("GLISSANDO_DEBUG_INPUT").is_some()
    }

    fn handle_events(&mut self, events: Vec<TouchEvent>) {
        if !events.is_empty() && self.log_input() {
            eprintln!("touch events {events:?}");
        }
        for e in events {
            match e {
                TouchEvent::Pressed { key } => {
                    if !self.pressed.contains(&key) {
                        self.pressed.push(key);
                    }
                    self.actions.push(Action::Haptic(Haptic::Key));
                    if self.config.key_popup
                        && let KeyAction::Text(t) = &self.keys[key].def.action
                        && t.chars().count() <= 2
                    {
                        self.preview = Some((key, self.cased(t)));
                    }
                }
                TouchEvent::Released { key } => {
                    self.pressed.retain(|&k| k != key);
                    if self.preview.as_ref().map(|p| p.0) == Some(key) {
                        self.preview = None;
                    }
                }
                TouchEvent::Activate { key, x, y } => {
                    let action = self.keys[key].def.action.clone();
                    if action == KeyAction::Shift {
                        let s = match self.shift {
                            Shift::Off => Shift::Once,
                            Shift::Once | Shift::Locked => {
                                self.shift_manual_off = true;
                                Shift::Off
                            }
                        };
                        self.set_shift(s);
                        continue;
                    }

                    let t = self.to_units(x, y);
                    self.activate_key_action(&action, Some(t));
                }
                TouchEvent::CapsLock { .. } => self.set_shift(Shift::Locked),
                TouchEvent::ShiftChordEnd { .. } => {
                    if self.shift != Shift::Locked {
                        self.set_shift(Shift::Off);
                    }
                }
                TouchEvent::LongPress { key, .. } => {
                    self.actions.push(Action::Haptic(Haptic::LongPress));
                    self.open_popup(key);
                }
                TouchEvent::PopupMove { x, y } => self.popup_select(x, y),
                TouchEvent::PopupCommit => self.popup_commit(),
                TouchEvent::Repeat { key, count } if self.keys[key].def.action != KeyAction::Backspace => {
                    if count > 1 {
                        let action = self.keys[key].def.action.clone();
                        self.activate_key_action(&action, None);
                    }
                }
                TouchEvent::Repeat { count, .. } => {
                    // The first repeat event duplicates the press-activation.
                    if count > 1 {
                        let ops = if count > 25 && count % 4 == 0 {
                            self.engine.delete_words(1)
                        } else if count > 25 {
                            Vec::new()
                        } else {
                            self.engine.backspace()
                        };
                        self.push_ops(ops);
                        self.actions.push(Action::Haptic(Haptic::Tick));
                    }
                }
                TouchEvent::CursorStart | TouchEvent::TrackpadStart => {
                    let ops = self.engine.commit_composing();
                    self.push_ops(ops);
                    self.preview = None;
                }
                TouchEvent::CursorMove { dx, dy } => {
                    let (kx, n) = if dx < 0 { (NavKey::Left, -dx) } else { (NavKey::Right, dx) };
                    for _ in 0..n {
                        self.actions.push(Action::Nav(kx));
                    }
                    let (ky, m) = if dy < 0 { (NavKey::Up, -dy) } else { (NavKey::Down, dy) };
                    for _ in 0..m {
                        self.actions.push(Action::Nav(ky));
                    }
                    self.actions.push(Action::Haptic(Haptic::Tick));
                }
                TouchEvent::CursorEnd => {}
                TouchEvent::DeletePreview { words } => {
                    self.strip_message = if words == 0 {
                        None
                    } else {
                        let text = self.engine.delete_preview(words);
                        let shown: String = text.chars().rev().take(40).collect::<Vec<_>>().into_iter().rev().collect();
                        Some((format!("⌫ {}", shown.trim()), true))
                    };
                    self.actions.push(Action::Haptic(Haptic::Tick));
                }
                TouchEvent::DeleteCommit { words } => {
                    self.strip_message = None;
                    let ops = self.engine.delete_words(words);
                    self.push_ops(ops);
                    self.update_autoshift();
                }
                TouchEvent::DeleteCancel => self.strip_message = None,
                TouchEvent::SwipeStart { .. } => {
                    self.preview = None;
                    self.pressed.clear();
                    self.trail.clear();
                    self.sync_trail();
                }
                TouchEvent::SwipeMove { .. } => self.sync_trail(),
                TouchEvent::SwipeEnd => {
                    self.sync_trail();
                    self.trail_until = self.now + 300;
                    self.decode_swipe();
                }
            }
        }
    }

    fn sync_trail(&mut self) {
        if !self.config.glide_trail {
            return;
        }
        self.trail = self.touch.trace().iter().map(|p| (p.x, p.y, p.t)).collect();
    }

    fn decode_swipe(&mut self) {
        let raw: Vec<(f32, f32)> = self.touch.trace().iter().map(|p| self.to_units(p.x, p.y)).collect();
        let pts: Vec<Pt> = clean_trace(&raw, 0.05);
        let prev = self.engine.context_words(1).pop();
        let cands =
            self.decoder.decode(&pts, &self.keymap, &self.engine.langs, &self.engine.active_priors(), prev.as_deref());
        let words: Vec<String> = cands.into_iter().map(|c| c.word).collect();
        if words.is_empty() {
            return;
        }
        if let Some(q) = &mut self.emoji_search {
            *q = words[0].to_lowercase();
            return;
        }
        let pattern = self.case_pattern();
        let ops = self.engine.swipe(words, pattern);
        self.push_ops(ops);
        if self.shift == Shift::Once {
            self.set_shift(Shift::Off);
        }
        self.update_autoshift();
    }

    // ---- view ----------------------------------------------------------------

    fn key_label(&self, def: &KeyDef) -> KeyLabel {
        if let Some(l) = &def.label {
            return KeyLabel::Text(l.clone());
        }
        match &def.action {
            KeyAction::Text(t) => {
                KeyLabel::Text(if self.layer.starts_with("letters") { self.cased(t) } else { t.clone() })
            }
            KeyAction::Shift => KeyLabel::Icon(match self.shift {
                Shift::Off => Icon::Shift,
                Shift::Once => Icon::ShiftOn,
                Shift::Locked => Icon::CapsLock,
            }),
            KeyAction::Backspace => KeyLabel::Icon(Icon::Backspace),
            KeyAction::Enter => KeyLabel::Icon(Icon::Enter),
            KeyAction::Space => {
                let langs = self.layout_languages();
                let l = &self.layouts[self.current];
                if langs.len() > 1 && self.layer.starts_with("letters") {
                    KeyLabel::Space(langs.iter().map(|c| c.to_uppercase()).collect::<Vec<_>>().join(" · "))
                } else if self.layer.starts_with("letters") {
                    KeyLabel::Space(l.name.clone())
                } else {
                    KeyLabel::None
                }
            }
            KeyAction::Layer(name) => KeyLabel::Text(match name.as_str() {
                "letters" => self.layouts[self.current].letters_label(),
                "symbols" => "?123".to_string(),
                "symbols2" => "=\\<".to_string(),
                "numpad" => "1234".to_string(),
                other => other.to_string(),
            }),
            a => KeyLabel::Icon(action_icon(a)),
        }
    }

    pub fn view(&mut self, now: u32) -> View {
        self.now = now;
        let keys = if self.panel.is_some() {
            Vec::new()
        } else {
            self.keys
                .iter()
                .zip(&self.geom.keys)
                .map(|(k, g)| KeyView {
                    rect: g.visual,
                    label: self.key_label(&k.def),
                    hint: k.def.hint.clone().filter(|_| self.config.hints),
                    style: match k.def.action {
                        KeyAction::Ctrl if self.mods.ctrl => KeyStyle::Latched,
                        KeyAction::Alt if self.mods.alt => KeyStyle::Latched,
                        _ => k.def.style,
                    },
                })
                .collect()
        };
        let items = self.strip_items();
        let dividers = items.iter().all(|(_, i, _)| matches!(i, StripItem::Text { .. }));
        let trail = if now < self.trail_until || self.touch.swiping() { self.trail.clone() } else { Vec::new() };
        let side_area = if self.side.len() == 2 {
            let i1 = Icon::OneHanded;
            Some((self.side[0], i1, self.side[1], Icon::Keyboard))
        } else {
            None
        };
        View {
            width: self.width,
            height: self.desired_height(),
            metrics: self.metrics,
            keys,
            keys_rev: self.keys_rev,
            pressed: self.pressed.clone(),
            preview: self.preview.clone(),
            popup: self.popup.as_ref().map(|p| p.popup.clone()),
            strip: StripView {
                items: items.into_iter().map(|(r, i, _)| (r, i)).collect(),
                dividers,
                message: self.strip_message.clone(),
            },
            strip_pressed: self.strip_pressed,
            trail,
            now,
            panel: self.panel.as_ref().map(|p| p.view()),
            side_area,
        }
    }

    /// The view after the most likely next shift change, for pre-rendering.
    pub fn alternate_view(&mut self, now: u32) -> Option<View> {
        if self.panel.is_some() || !self.layer.starts_with("letters") {
            return None;
        }
        let saved = (self.shift, self.keys_rev);
        self.shift = if self.shift == Shift::Off { Shift::Once } else { Shift::Off };
        let v = self.view(now);
        (self.shift, self.keys_rev) = saved;
        Some(v)
    }

    /// Simulate a press for render benchmarks.
    pub fn bench_press(&mut self, key: usize) {
        self.pressed = vec![key];
        self.preview = Some((key, "q".into()));
    }

    pub fn bench_trail(&mut self, now: u32) {
        let y = self.metrics.strip_height + self.metrics.row_height * 1.5;
        self.trail =
            (0..40).map(|i| (10.0 + i as f32 * 8.0, y + (i as f32 * 0.3).sin() * 30.0, now - 200 + i * 5)).collect();
        self.trail_until = now + 300;
    }

    pub fn toggle_shift_for_bench(&mut self) {
        let s = if self.shift == Shift::Off { Shift::Once } else { Shift::Off };
        self.set_shift(s);
    }

    /// Key rectangles for automated tests (`--dump-geometry`).
    pub fn geometry_dump(&self) -> String {
        let mut s = String::from("[\n");
        for (i, (k, g)) in self.keys.iter().zip(&self.geom.keys).enumerate() {
            let name = match &k.def.action {
                KeyAction::Text(t) => t.clone(),
                other => format!("{{{}}}", action_name(other)),
            };
            let (cx, cy) = g.visual.center();
            s.push_str(&format!(
                "  {{\"key\": {}, \"x\": {:.1}, \"y\": {:.1}, \"w\": {:.1}, \"h\": {:.1}}}{}\n",
                json_str(&name),
                cx,
                cy,
                g.visual.w,
                g.visual.h,
                if i + 1 < self.keys.len() { "," } else { "" }
            ));
        }
        s.push_str("]\n");
        s
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StripTap {
    Suggestion(usize),
    Panel(PanelKind),
    ClosePanel,
    Hide,
    OneHanded,
    /// Leave emoji search (back to the emoji panel).
    SearchBack,
    Voice,
    VoiceStop,
    VoiceCancel,
    /// Insert an emoji (search result).
    Emoji(String),
    /// Finish the current word and append the emoji.
    WordEmoji(String),
    None,
}

/// The key a tap sends when Ctrl/Alt are latched (None: typed as usual).
fn chord_key(a: &KeyAction) -> Option<ChordKey> {
    Some(match a {
        KeyAction::Text(t) => {
            let mut it = t.chars();
            let c = it.next()?;
            if it.next().is_some() {
                return None;
            }
            ChordKey::Char(c.to_lowercase().next().unwrap_or(c))
        }
        KeyAction::Space => ChordKey::Char(' '),
        KeyAction::Backspace => ChordKey::Backspace,
        KeyAction::Enter => ChordKey::Enter,
        KeyAction::Tab => ChordKey::Nav(NavKey::Tab),
        KeyAction::Escape => ChordKey::Nav(NavKey::Escape),
        KeyAction::Left => ChordKey::Nav(NavKey::Left),
        KeyAction::Right => ChordKey::Nav(NavKey::Right),
        KeyAction::Up => ChordKey::Nav(NavKey::Up),
        KeyAction::Down => ChordKey::Nav(NavKey::Down),
        KeyAction::Home => ChordKey::Nav(NavKey::Home),
        KeyAction::End => ChordKey::Nav(NavKey::End),
        _ => return None,
    })
}

fn json_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn action_name(a: &KeyAction) -> String {
    match a {
        KeyAction::Shift => "shift".into(),
        KeyAction::Backspace => "backspace".into(),
        KeyAction::Enter => "enter".into(),
        KeyAction::Space => "space".into(),
        KeyAction::Layer(l) => format!("layer={l}"),
        KeyAction::NextLanguage => "lang".into(),
        KeyAction::Emoji => "emoji".into(),
        other => format!("{other:?}").to_lowercase(),
    }
}

fn action_icon(a: &KeyAction) -> Icon {
    match a {
        KeyAction::NextLanguage => Icon::Globe,
        KeyAction::Emoji => Icon::Emoji,
        KeyAction::Clipboard => Icon::Clipboard,
        KeyAction::Settings => Icon::Settings,
        KeyAction::Voice => Icon::Mic,
        KeyAction::Tab => Icon::Tab,
        KeyAction::Hide => Icon::Hide,
        KeyAction::OneHanded => Icon::OneHanded,
        KeyAction::Left => Icon::Left,
        KeyAction::Right => Icon::Right,
        KeyAction::Up => Icon::Up,
        KeyAction::Down => Icon::Down,
        KeyAction::Escape => Icon::Close,
        KeyAction::Home => Icon::Left,
        KeyAction::End => Icon::Right,
        _ => Icon::Keyboard,
    }
}

trait LettersLabel {
    fn letters_label(&self) -> String;
}

impl LettersLabel for Layout {
    /// "ABC" / "АБВ" style label for returning to the letters layer.
    fn letters_label(&self) -> String {
        let a = self.alphabet();
        if a.iter().any(|c| ('а'..='я').contains(c)) { "АБВ".into() } else { "ABC".into() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glissando_core::layout::SymbolSet;
    use glissando_lang::engine::Settings;
    use glissando_lang::model::Language;
    use glissando_lang::user::UserModel;

    pub fn test_keyboard(layouts: &[&str]) -> Keyboard {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/layouts");
        let sym = SymbolSet::parse(&std::fs::read_to_string(format!("{dir}/symbols.toml")).unwrap()).unwrap();
        let ls: Vec<Layout> = layouts
            .iter()
            .map(|id| Layout::parse(&std::fs::read_to_string(format!("{dir}/{id}.toml")).unwrap(), &sym).unwrap())
            .collect();
        let langs = layouts.iter().map(|l| Language::new(l, None, UserModel::in_memory())).collect();
        let engine = Engine::new(langs, Settings::default());
        let mut kb = Keyboard::new(Config::default(), ls, engine);
        kb.resize(360.0, false);
        kb.activate(FieldInfo::default());
        kb.take_actions();
        kb
    }

    fn center(kb: &Keyboard, name: &str) -> (f32, f32) {
        let i = kb
            .keys
            .iter()
            .position(|k| match &k.def.action {
                KeyAction::Text(t) => t == name,
                a => format!("{{{}}}", action_name(a)) == name,
            })
            .unwrap_or_else(|| panic!("no key {name}"));
        kb.geom.keys[i].visual.center()
    }

    #[test]
    fn space_scrub_sends_arrows() {
        let mut kb = test_keyboard(&["en"]);
        let mut t = 100;
        for k in ["a", "b", "{space}", "c", "d", "{space}"] {
            let (x, y) = center(&kb, k);
            kb.touch_down(9, x, y, t);
            kb.touch_up(9, x, y, t + 40);
            t += 200;
        }
        kb.take_actions();
        let (x, y) = center(&kb, "{space}");
        kb.touch_down(1, x, y, 1000);
        for i in 1..=10 {
            kb.touch_motion(1, x - 6.0 * i as f32, y, 1000 + i * 10);
        }
        kb.touch_up(1, x - 60.0, y, 1200);
        let navs = kb.take_actions().into_iter().filter(|a| matches!(a, Action::Nav(NavKey::Left))).count();
        assert!(navs >= 3, "{navs}");
    }

    #[test]
    fn touches_in_private_fields_are_not_loggable() {
        let mut kb = test_keyboard(&["en"]);
        assert!(kb.input_loggable());
        kb.activate(FieldInfo { purpose: Purpose::Password, ..FieldInfo::default() });
        assert!(!kb.input_loggable());
        kb.activate(FieldInfo { sensitive: true, ..FieldInfo::default() });
        assert!(!kb.input_loggable());
    }

    fn tap(kb: &mut Keyboard, name: &str, t: u32) -> Vec<Action> {
        let (x, y) = center(kb, name);
        kb.touch_down(1, x, y, t);
        kb.touch_up(1, x, y, t + 40);
        kb.take_actions()
    }

    #[test]
    fn terminal_keys_send_chords() {
        let mut kb = test_keyboard(&["en"]);
        let has = |kb: &Keyboard, a: KeyAction| kb.keys.iter().any(|k| k.def.action == a);
        assert!(!has(&kb, KeyAction::Ctrl), "no terminal row in normal fields");
        kb.activate(FieldInfo { purpose: Purpose::Terminal, ..Default::default() });
        kb.take_actions();
        for a in [KeyAction::Escape, KeyAction::Tab, KeyAction::Ctrl, KeyAction::Alt, KeyAction::Left, KeyAction::Up] {
            assert!(has(&kb, a.clone()), "{a:?}");
        }
        assert!(tap(&mut kb, "{escape}", 100).contains(&Action::Nav(NavKey::Escape)));
        tap(&mut kb, "{ctrl}", 200);
        let ctrl = kb.keys.iter().position(|k| k.def.action == KeyAction::Ctrl).unwrap();
        assert_eq!(kb.view(300).keys[ctrl].style, KeyStyle::Latched);
        let acts = tap(&mut kb, "c", 400);
        assert!(acts.contains(&Action::Chord(ChordKey::Char('c'), Mods { ctrl: true, alt: false })), "{acts:?}");
        assert_eq!(kb.view(500).keys[ctrl].style, KeyStyle::Function, "one-shot");
        tap(&mut kb, "{alt}", 600);
        let acts = tap(&mut kb, "{left}", 700);
        assert!(
            acts.contains(&Action::Chord(ChordKey::Nav(NavKey::Left), Mods { ctrl: false, alt: true })),
            "{acts:?}"
        );
        let acts = tap(&mut kb, "c", 800);
        assert!(acts.iter().any(|a| matches!(a, Action::Ops(_))), "typed normally again: {acts:?}");
    }
}

#[cfg(test)]
mod typing_tests {
    use super::tests::test_keyboard;
    use super::*;

    #[test]
    fn typing_hello_updates_preedit_every_key() {
        let mut kb = test_keyboard(&["en"]);
        crate::panels::init_emoji(
            crate::emoji::EmojiDb::load(std::path::Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../data/emoji/emoji.tsv"
            ))),
            crate::emoji::Recents::default(),
        );
        let mut last = String::new();
        let mut t = 100;
        for k in ["h", "e", "l", "l", "o"] {
            let i = kb.keys.iter().position(|x| x.def.action == KeyAction::Text(k.into())).unwrap();
            let (x, y) = kb.geom.keys[i].visual.center();
            kb.touch_down(1, x, y, t);
            kb.touch_up(1, x, y, t + 40);
            let _ = kb.view(t + 50);
            t += 200;
            for a in kb.take_actions() {
                if let Action::Ops(ops) = a {
                    for op in ops {
                        if let Op::Preedit(p) = op {
                            last = p;
                        }
                    }
                }
            }
        }
        assert_eq!(last, "Hello");
    }
}
