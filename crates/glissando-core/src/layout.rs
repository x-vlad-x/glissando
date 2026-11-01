//! Keyboard layout model and the compact TOML layout format.
//!
//! A layout file describes the letter layer of one script/language. Symbol
//! and numeric layers are shared and come from `symbols.toml`. Rows are
//! written as whitespace separated tokens:
//!
//! * `q` – a key that types `q` (multi-character labels such as `.com` work too)
//! * `q:1.5` – the same key, 1.5 key units wide
//! * `{shift:1.5}` – a function key (see [`KeyAction::from_name`])
//! * `{gap:0.5}` – empty space
//!
//! Long-press alternates and hint labels are given per key in `[alternates]`
//! and `hints` rows; the first alternate is the one pre-selected when the
//! popup opens (Gboard shows the hint symbol there).

use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum KeyAction {
    /// Insert text. Letters follow shift state.
    Text(String),
    Shift,
    Backspace,
    Enter,
    Space,
    /// Switch to another layer of the current layout (`letters`, `symbols`, ...).
    Layer(String),
    NextLanguage,
    Emoji,
    Clipboard,
    Settings,
    Voice,
    Tab,
    Escape,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    /// One-shot modifiers for the next key (terminal keys).
    Ctrl,
    Alt,
    Hide,
    /// Toggle one-handed mode.
    OneHanded,
    Gap,
}

impl KeyAction {
    pub fn from_name(name: &str) -> Option<KeyAction> {
        Some(match name {
            "shift" => KeyAction::Shift,
            "backspace" => KeyAction::Backspace,
            "enter" => KeyAction::Enter,
            "space" => KeyAction::Space,
            "lang" => KeyAction::NextLanguage,
            "emoji" => KeyAction::Emoji,
            "clipboard" => KeyAction::Clipboard,
            "settings" => KeyAction::Settings,
            "voice" => KeyAction::Voice,
            "tab" => KeyAction::Tab,
            "esc" => KeyAction::Escape,
            "left" => KeyAction::Left,
            "right" => KeyAction::Right,
            "up" => KeyAction::Up,
            "down" => KeyAction::Down,
            "home" => KeyAction::Home,
            "end" => KeyAction::End,
            "ctrl" => KeyAction::Ctrl,
            "alt" => KeyAction::Alt,
            "hide" => KeyAction::Hide,
            "onehand" => KeyAction::OneHanded,
            "gap" => KeyAction::Gap,
            "comma" => KeyAction::Text(",".into()),
            "period" => KeyAction::Text(".".into()),
            _ => {
                let layer = name.strip_prefix("layer=")?;
                KeyAction::Layer(layer.to_string())
            }
        })
    }

    pub fn is_text(&self) -> bool {
        matches!(self, KeyAction::Text(_))
    }

    /// True for keys that type a single letter (used by swipe typing).
    pub fn letter(&self) -> Option<char> {
        match self {
            KeyAction::Text(t) => {
                let mut it = t.chars();
                let c = it.next()?;
                if it.next().is_none() && c.is_alphabetic() { Some(c) } else { None }
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyStyle {
    /// Regular character key.
    Normal,
    /// Shift, backspace, layer switches...
    Function,
    /// The enter/action key.
    Accent,
    Space,
    Invisible,
    /// A modifier that is on for the next key.
    Latched,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KeyDef {
    pub action: KeyAction,
    /// Label override; `None` means derive from the action.
    pub label: Option<String>,
    pub width: f32,
    /// Small secondary label (top-right), also the default long-press choice.
    pub hint: Option<String>,
    /// Long-press popup entries.
    pub alternates: Vec<String>,
    pub style: KeyStyle,
}

impl KeyDef {
    pub fn text(t: &str) -> KeyDef {
        KeyDef {
            action: KeyAction::Text(t.to_string()),
            label: None,
            width: 1.0,
            hint: None,
            alternates: Vec::new(),
            style: KeyStyle::Normal,
        }
    }

    pub fn special(action: KeyAction, width: f32) -> KeyDef {
        let style = match action {
            KeyAction::Enter => KeyStyle::Accent,
            KeyAction::Space => KeyStyle::Space,
            KeyAction::Gap => KeyStyle::Invisible,
            KeyAction::Text(_) => KeyStyle::Normal,
            _ => KeyStyle::Function,
        };
        KeyDef { action, label: None, width, hint: None, alternates: Vec::new(), style }
    }

    /// Popup content for long press: the hint first, then alternates.
    /// Entries written as `{name}` are actions (see [`KeyAction::from_name`]).
    pub fn popup_entries(&self) -> Vec<String> {
        let mut v = Vec::new();
        if let Some(h) = &self.hint {
            v.push(h.clone());
        }
        for a in &self.alternates {
            if !v.contains(a) {
                v.push(a.clone());
            }
        }
        v
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RowDef {
    pub keys: Vec<KeyDef>,
    /// Relative row height (1.0 = normal).
    pub height: f32,
}

impl RowDef {
    pub fn units(&self) -> f32 {
        self.keys.iter().map(|k| k.width).sum()
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Layer {
    pub rows: Vec<RowDef>,
}

impl Layer {
    pub fn max_units(&self) -> f32 {
        self.rows.iter().map(|r| r.units()).fold(0.0, f32::max)
    }

    pub fn keys(&self) -> impl Iterator<Item = &KeyDef> {
        self.rows.iter().flat_map(|r| r.keys.iter())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub id: String,
    pub name: String,
    /// Short label used on the space bar / language key, e.g. "EN".
    pub short: String,
    /// Language codes whose dictionaries are used with this layout.
    pub languages: Vec<String>,
    pub swipe: bool,
    pub layers: BTreeMap<String, Layer>,
}

impl Layout {
    pub fn layer(&self, name: &str) -> Option<&Layer> {
        self.layers.get(name)
    }

    /// Letters available on the main layer (lowercase), for swipe and the spatial model.
    pub fn alphabet(&self) -> Vec<char> {
        let mut v: Vec<char> =
            self.layer("letters").map(|l| l.keys().filter_map(|k| k.action.letter()).collect()).unwrap_or_default();
        v.dedup();
        v
    }
}

#[derive(Debug)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseError {}

#[derive(Deserialize)]
struct FileLayer {
    rows: Vec<FileRow>,
    #[serde(default)]
    bottom: Option<String>,
}

#[derive(Deserialize)]
struct FileRow {
    keys: String,
    #[serde(default)]
    hints: Option<String>,
    #[serde(default)]
    height: Option<f32>,
}

#[derive(Deserialize)]
struct FileLayout {
    id: String,
    name: String,
    short: String,
    #[serde(default)]
    languages: Vec<String>,
    #[serde(default = "default_true")]
    swipe: bool,
    #[serde(default)]
    alternates: BTreeMap<String, String>,
    /// Layers defined by this file (usually just `letters`).
    layers: BTreeMap<String, FileLayer>,
}

#[derive(Deserialize)]
struct FileSymbols {
    #[serde(default)]
    alternates: BTreeMap<String, String>,
    #[serde(default)]
    bottom: BTreeMap<String, String>,
    layers: BTreeMap<String, FileLayer>,
}

fn default_true() -> bool {
    true
}

/// Shared symbol layers and bottom row templates.
#[derive(Debug, Clone, Default)]
pub struct SymbolSet {
    pub layers: BTreeMap<String, Layer>,
    pub bottom_rows: BTreeMap<String, String>,
    pub alternates: BTreeMap<String, String>,
}

impl SymbolSet {
    pub fn parse(src: &str) -> Result<SymbolSet, ParseError> {
        let f: FileSymbols = toml::from_str(src).map_err(|e| ParseError(e.to_string()))?;
        let mut set =
            SymbolSet { layers: BTreeMap::new(), bottom_rows: f.bottom.clone(), alternates: f.alternates.clone() };
        for (name, fl) in &f.layers {
            set.layers.insert(name.clone(), build_layer(fl, &f.alternates, &f.bottom)?);
        }
        Ok(set)
    }
}

/// Split a popup alternates spec. Entries are separated by spaces; a spec
/// without spaces is split into characters (grapheme-naive, which is fine for
/// the accented letters we use).
fn split_alternates(spec: &str) -> Vec<String> {
    if spec.contains(' ') {
        spec.split_whitespace().map(|s| s.to_string()).collect()
    } else {
        spec.chars().map(|c| c.to_string()).collect()
    }
}

fn parse_token(tok: &str) -> Result<KeyDef, ParseError> {
    if let Some(inner) = tok.strip_prefix('{').and_then(|t| t.strip_suffix('}')) {
        let (name, width) = split_width(inner)?;
        // `{text=.com:1.5}` lets a text key contain characters reserved by the syntax.
        if let Some(text) = name.strip_prefix("text=") {
            let mut k = KeyDef::text(text);
            k.width = width;
            return Ok(k);
        }
        let action = KeyAction::from_name(name).ok_or_else(|| ParseError(format!("unknown special key {{{name}}}")))?;
        let mut k = KeyDef::special(action, width);
        if let KeyAction::Text(_) = k.action {
            k.style = KeyStyle::Normal;
        }
        Ok(k)
    } else {
        let (text, width) = split_width(tok)?;
        let mut k = KeyDef::text(text);
        k.width = width;
        Ok(k)
    }
}

fn split_width(tok: &str) -> Result<(&str, f32), ParseError> {
    // A trailing `:<number>` is a width; a lone ':' is the colon key.
    if let Some(pos) = tok.rfind(':')
        && pos > 0
    {
        let (name, w) = (&tok[..pos], &tok[pos + 1..]);
        if w == "*" {
            return Ok((name, FILL));
        }
        if let Ok(w) = w.parse::<f32>() {
            return Ok((name, w));
        }
    }
    Ok((tok, 1.0))
}

/// Width marker for keys that take the remaining space of their row.
const FILL: f32 = -1.0;

pub fn parse_row(keys: &str) -> Result<Vec<KeyDef>, ParseError> {
    keys.split_whitespace().map(parse_token).collect()
}

fn build_layer(
    fl: &FileLayer,
    alternates: &BTreeMap<String, String>,
    bottoms: &BTreeMap<String, String>,
) -> Result<Layer, ParseError> {
    let mut layer = Layer::default();
    let mut rows: Vec<&FileRow> = fl.rows.iter().collect();
    let bottom_row;
    if let Some(b) = &fl.bottom {
        let spec = bottoms.get(b).cloned().unwrap_or_else(|| b.clone());
        bottom_row = FileRow { keys: spec, hints: None, height: None };
        rows.push(&bottom_row);
    }
    for fr in rows {
        let mut keys = parse_row(&fr.keys)?;
        if let Some(h) = &fr.hints {
            let hints: Vec<&str> = h.split_whitespace().collect();
            let mut hi = 0;
            for k in keys.iter_mut() {
                if k.action.is_text() {
                    if let Some(h) = hints.get(hi)
                        && *h != "_"
                    {
                        k.hint = Some(h.to_string());
                    }
                    hi += 1;
                }
            }
        }
        for k in keys.iter_mut() {
            if let KeyAction::Text(t) = &k.action
                && let Some(spec) = alternates.get(t.as_str())
            {
                k.alternates = split_alternates(spec);
            }
        }
        layer.rows.push(RowDef { keys, height: fr.height.unwrap_or(1.0) });
    }
    // Resolve fill widths against the widest row without fill keys.
    let target =
        layer.rows.iter().filter(|r| r.keys.iter().all(|k| k.width != FILL)).map(|r| r.units()).fold(0.0, f32::max);
    for row in layer.rows.iter_mut() {
        let fills = row.keys.iter().filter(|k| k.width == FILL).count();
        if fills > 0 {
            let fixed: f32 = row.keys.iter().filter(|k| k.width != FILL).map(|k| k.width).sum();
            let w = ((target - fixed) / fills as f32).max(1.0);
            for k in row.keys.iter_mut().filter(|k| k.width == FILL) {
                k.width = w;
            }
        }
    }
    Ok(layer)
}

impl Layout {
    /// Parse a layout file. Symbol layers from `symbols` are merged in unless
    /// the layout defines a layer of the same name.
    pub fn parse(src: &str, symbols: &SymbolSet) -> Result<Layout, ParseError> {
        let f: FileLayout = toml::from_str(src).map_err(|e| ParseError(e.to_string()))?;
        let mut alternates = symbols.alternates.clone();
        alternates.extend(f.alternates.clone());
        let mut layers = BTreeMap::new();
        for (name, fl) in &f.layers {
            layers.insert(name.clone(), build_layer(fl, &alternates, &symbols.bottom_rows)?);
            // Purpose specific variants (email/url fields) reuse the rows with another bottom row.
            if let Some(bottom) = &fl.bottom {
                for tname in symbols.bottom_rows.keys() {
                    if let Some(variant) = tname.strip_prefix(&format!("{bottom}_")) {
                        let vl = FileLayer {
                            rows: fl
                                .rows
                                .iter()
                                .map(|r| FileRow { keys: r.keys.clone(), hints: r.hints.clone(), height: r.height })
                                .collect(),
                            bottom: Some(tname.clone()),
                        };
                        layers
                            .insert(format!("{name}@{variant}"), build_layer(&vl, &alternates, &symbols.bottom_rows)?);
                    }
                }
            }
        }
        for (name, l) in &symbols.layers {
            layers.entry(name.clone()).or_insert_with(|| l.clone());
        }
        if !layers.contains_key("letters") {
            return Err(ParseError(format!("layout {} has no letters layer", f.id)));
        }
        Ok(Layout { id: f.id, name: f.name, short: f.short, languages: f.languages, swipe: f.swipe, layers })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYM: &str = r#"
[alternates]
"." = ", ? ! ' \" - : ; @ ( ) #"
[bottom]
letters = "{layer=symbols:1.5} {comma} {lang} {space:*} {period} {enter:1.5}"
letters_email = "{layer=symbols:1.5} @ {lang} {space:*} {period} {enter:1.5}"
[layers.symbols]
rows = [{ keys = "1 2 3 4 5 6 7 8 9 0" }]
bottom = "{layer=letters:1.5} {comma} {space:5} {period} {enter:1.5}"
"#;

    const EN: &str = r#"
id = "en"
name = "English"
short = "EN"
languages = ["en"]
[alternates]
e = "èéêëē"
[layers.letters]
rows = [
  { keys = "q w e r t y u i o p", hints = "1 2 3 4 5 6 7 8 9 0" },
  { keys = "{gap:0.5} a s d f g h j k l {gap:0.5}" },
  { keys = "{shift:1.5} z x c v b n m {backspace:1.5}" },
]
bottom = "letters"
"#;

    #[test]
    fn parses_layout_with_symbols() {
        let sym = SymbolSet::parse(SYM).unwrap();
        let l = Layout::parse(EN, &sym).unwrap();
        let letters = l.layer("letters").unwrap();
        assert_eq!(letters.rows.len(), 4);
        assert_eq!(letters.rows[0].units(), 10.0);
        assert_eq!(letters.rows[1].units(), 10.0);
        assert_eq!(letters.rows[2].units(), 10.0);
        assert_eq!(letters.rows[3].units(), 10.0);
        let e = &letters.rows[0].keys[2];
        assert_eq!(e.hint.as_deref(), Some("3"));
        assert_eq!(e.popup_entries()[0], "3");
        assert!(e.popup_entries().contains(&"é".to_string()));
        assert!(l.layer("symbols").is_some());
        assert_eq!(l.alphabet().len(), 26);
        let period = &letters.rows[3].keys[4];
        assert_eq!(period.action, KeyAction::Text(".".into()));
        assert!(period.alternates.contains(&"?".to_string()));
        assert_eq!(letters.rows[3].keys[0].action, KeyAction::Layer("symbols".into()));
        assert_eq!(letters.rows[3].keys[3].width, 4.0);
        let email = l.layer("letters@email").unwrap();
        assert_eq!(email.rows[3].keys[1].action, KeyAction::Text("@".into()));
    }

    #[test]
    fn shipped_layouts_parse() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/layouts");
        let sym = SymbolSet::parse(&std::fs::read_to_string(format!("{dir}/symbols.toml")).unwrap()).unwrap();
        for (id, letters, units) in [("en", 26, 10.0), ("de", 29, 11.0), ("ru", 31, 11.0)] {
            let l = Layout::parse(&std::fs::read_to_string(format!("{dir}/{id}.toml")).unwrap(), &sym).unwrap();
            assert_eq!(l.alphabet().len(), letters, "{id}");
            let lt = l.layer("letters").unwrap();
            for r in &lt.rows {
                assert!((r.units() - units).abs() < 0.01, "{id}: row {} units", r.units());
            }
            for name in ["symbols", "symbols2", "numpad", "phone", "digits", "letters@url", "letters@email"] {
                assert!(l.layer(name).is_some(), "{id} {name}");
            }
        }
    }

    #[test]
    fn colon_key_and_widths() {
        let r = parse_row(": {text=.com:1.5} a:2").unwrap();
        assert_eq!(r[0].action, KeyAction::Text(":".into()));
        assert_eq!(r[1].action, KeyAction::Text(".com".into()));
        assert_eq!(r[1].width, 1.5);
        assert_eq!(r[2].width, 2.0);
    }
}
