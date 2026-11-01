//! Drawing of the keyboard with cairo + pango.
//!
//! The key layer is rendered once into a cached surface whenever its content
//! changes (layer switch, shift state, theme, size); each frame only blits it
//! and paints the dynamic parts on top: pressed keys, key preview, popups,
//! the suggestion strip and the glide trail.

use crate::icons::{self, Icon};
use crate::theme::{Metrics, Rgba, Theme};
use glissando_core::geometry::Rect;
use glissando_core::layout::KeyStyle;

#[derive(Debug, Clone, PartialEq)]
pub enum KeyLabel {
    Text(String),
    Icon(Icon),
    /// Space bar: language name, drawn small.
    Space(String),
    None,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KeyView {
    pub rect: Rect,
    pub label: KeyLabel,
    pub hint: Option<String>,
    pub style: KeyStyle,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PopupEntry {
    Text(String),
    Icon(Icon),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Popup {
    pub rect: Rect,
    pub cells: Vec<Rect>,
    pub entries: Vec<PopupEntry>,
    pub selected: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StripItem {
    Text { text: String, emphasized: bool },
    Icon(Icon),
}

#[derive(Debug, Clone, PartialEq)]
pub struct StripView {
    pub items: Vec<(Rect, StripItem)>,
    /// Draw thin dividers between text items.
    pub dividers: bool,
    pub message: Option<(String, bool)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Panel {
    pub rect: Rect,
    pub cells: Vec<(Rect, PanelCell)>,
    pub scroll: f32,
    pub header: Vec<(Rect, PanelCell)>,
    pub footer: Vec<(Rect, PanelCell)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PanelCell {
    Emoji(String),
    Text { text: String, secondary: Option<String> },
    Icon { icon: Icon, active: bool },
    Tab { text: String, active: bool },
    Button { text: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub width: f32,
    pub height: f32,
    pub metrics: Metrics,
    pub keys: Vec<KeyView>,
    /// Changes whenever `keys` change; drives the cache.
    pub keys_rev: u64,
    pub pressed: Vec<usize>,
    pub preview: Option<(usize, String)>,
    pub popup: Option<Popup>,
    pub strip: StripView,
    pub strip_pressed: Option<usize>,
    pub trail: Vec<(f32, f32, u32)>,
    pub now: u32,
    pub panel: Option<Panel>,
    /// One-handed mode: area left empty next to the keys.
    pub side_area: Option<(Rect, Icon, Rect, Icon)>,
}

pub struct Renderer {
    /// Rendered key layers, most recent first, keyed by a content hash.
    cache: Vec<(u64, cairo::ImageSurface)>,
    font_family: String,
}

const CACHE_SLOTS: usize = 4;

fn hash_keys(view: &View, w: i32, h: i32, theme: &Theme, font: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hs = std::collections::hash_map::DefaultHasher::new();
    (w, h, theme.dark, font).hash(&mut hs);
    format!("{:?}", theme.key_border).hash(&mut hs);
    format!("{:?}", view.metrics).hash(&mut hs);
    for k in &view.keys {
        for v in [k.rect.x, k.rect.y, k.rect.w, k.rect.h] {
            v.to_bits().hash(&mut hs);
        }
        format!("{:?}{:?}{:?}", k.label, k.hint, k.style).hash(&mut hs);
    }
    hs.finish()
}

const TRAIL_MS: f32 = 280.0;

impl Renderer {
    pub fn new(font_family: &str) -> Renderer {
        Renderer { cache: Vec::new(), font_family: font_family.to_string() }
    }

    /// Load the label and colour emoji fonts once, off the input path.
    pub fn warm_fonts(&self) {
        if let Ok(s) = cairo::ImageSurface::create(cairo::Format::ARgb32, 64, 64) {
            if let Ok(cr) = cairo::Context::new(&s) {
                for t in ["Aa Яя ß", "😀👋🍕"] {
                    let l = self.layout(&cr, t, 20.0, false);
                    pangocairo::functions::show_layout(&cr, &l);
                }
            }
        }
    }

    pub fn set_font(&mut self, family: &str) {
        if family != self.font_family {
            self.font_family = family.to_string();
            self.cache.clear();
        }
    }

    /// Render the key layer of `view` into the cache without presenting it
    /// (used while idle to make the next shift/layer switch instant).
    pub fn prewarm(&mut self, pw: i32, ph: i32, scale: f64, view: &View, theme: &Theme) -> bool {
        let key = hash_keys(view, pw, ph, theme, &self.font_family);
        if self.cache.iter().any(|c| c.0 == key) {
            return false;
        }
        let s = self.render_keys(pw, ph, scale, view, theme);
        if self.cache.len() >= CACHE_SLOTS {
            self.cache.remove(0);
        }
        // Least recently used position: the current layer stays the newest.
        self.cache.insert(0, (key, s));
        true
    }

    fn render_keys(&self, pw: i32, ph: i32, scale: f64, view: &View, theme: &Theme) -> cairo::ImageSurface {
        let s = cairo::ImageSurface::create(cairo::Format::ARgb32, pw, ph).expect("cache surface");
        {
            let cr = cairo::Context::new(&s).expect("cairo");
            cr.scale(scale, scale);
            theme.background.set(&cr);
            cr.paint().ok();
            for k in &view.keys {
                self.draw_key(&cr, k, theme, &view.metrics, false);
            }
        }
        s
    }

    /// Render into `surface` whose pixel size is the logical size times `scale`.
    pub fn render(&mut self, surface: &cairo::ImageSurface, scale: f64, view: &View, theme: &Theme) {
        let pw = surface.width();
        let ph = surface.height();
        let key = hash_keys(view, pw, ph, theme, &self.font_family);
        match self.cache.iter().position(|c| c.0 == key) {
            Some(i) => {
                let c = self.cache.remove(i);
                self.cache.push(c);
            }
            None => {
                let s = self.render_keys(pw, ph, scale, view, theme);
                self.cache.push((key, s));
                if self.cache.len() > CACHE_SLOTS {
                    self.cache.remove(0);
                }
            }
        }
        let cr = cairo::Context::new(surface).expect("cairo");
        if let Some((_, cached)) = self.cache.last() {
            cr.set_source_surface(cached, 0.0, 0.0).ok();
            cr.set_operator(cairo::Operator::Source);
            cr.paint().ok();
            cr.set_operator(cairo::Operator::Over);
        }
        cr.scale(scale, scale);
        // Strip background (covers stale cache content of the strip area).
        theme.background.set(&cr);
        cr.rectangle(0.0, 0.0, view.width as f64, view.metrics.strip_height as f64);
        cr.fill().ok();
        self.draw_strip(&cr, view, theme);
        if let Some(panel) = &view.panel {
            self.draw_panel(&cr, panel, theme, &view.metrics);
        }
        if let Some((side, i1, side2, i2)) = &view.side_area {
            theme.hint.set(&cr);
            let (cx, cy) = side.center();
            icons::draw(&cr, *i1, cx as f64, cy as f64, 26.0);
            let (cx, cy) = side2.center();
            icons::draw(&cr, *i2, cx as f64, cy as f64, 26.0);
        }
        if view.panel.is_none() {
            for &i in &view.pressed {
                if let Some(k) = view.keys.get(i) {
                    self.draw_key(&cr, k, theme, &view.metrics, true);
                }
            }
        }
        self.draw_trail(&cr, view, theme);
        if let Some((i, text)) = &view.preview {
            if let Some(k) = view.keys.get(*i) {
                self.draw_preview(&cr, k, text, theme, &view.metrics);
            }
        }
        if let Some(p) = &view.popup {
            self.draw_popup(&cr, p, theme, &view.metrics);
        }
        surface.flush();
    }

    fn layout(&self, cr: &cairo::Context, text: &str, size: f32, bold: bool) -> pango::Layout {
        let layout = pangocairo::functions::create_layout(cr);
        let mut fd = pango::FontDescription::from_string(&self.font_family);
        fd.set_absolute_size(size as f64 * pango::SCALE as f64);
        if bold {
            fd.set_weight(pango::Weight::Semibold);
        }
        layout.set_font_description(Some(&fd));
        layout.set_text(text);
        layout
    }

    fn text_centered(&self, cr: &cairo::Context, text: &str, cx: f64, cy: f64, size: f32, bold: bool, max_w: f64) {
        let layout = self.layout(cr, text, size, bold);
        let (_, logical) = layout.pixel_extents();
        let (mut w, h) = (logical.width() as f64, logical.height() as f64);
        cr.save().ok();
        if w > max_w && max_w > 0.0 {
            let f = max_w / w;
            cr.translate(cx, cy);
            cr.scale(f, f);
            cr.translate(-cx, -cy);
            w = max_w / f;
        }
        cr.move_to(cx - w / 2.0, cy - h / 2.0);
        pangocairo::functions::show_layout(cr, &layout);
        cr.restore().ok();
    }

    fn key_colors(&self, style: KeyStyle, theme: &Theme, pressed: bool) -> (Rgba, Rgba) {
        match (style, pressed) {
            (KeyStyle::Accent, false) => (theme.accent, theme.on_accent),
            (KeyStyle::Accent, true) => (theme.accent_pressed, theme.on_accent),
            (KeyStyle::Latched, false) => (theme.accent, theme.on_accent),
            (KeyStyle::Latched, true) => (theme.accent_pressed, theme.on_accent),
            (KeyStyle::Function, false) => (theme.key_function, theme.label),
            (KeyStyle::Function, true) => (theme.key_function_pressed, theme.label),
            (_, false) => (theme.key, theme.label),
            (_, true) => (theme.key_pressed, theme.label),
        }
    }

    fn draw_key(&self, cr: &cairo::Context, k: &KeyView, theme: &Theme, m: &Metrics, pressed: bool) {
        if k.style == KeyStyle::Invisible {
            return;
        }
        let r = k.rect;
        let (bg, fg) = self.key_colors(k.style, theme, pressed);
        let radius = if k.style == KeyStyle::Accent { (r.h / 2.0).min(m.radius * 2.6) } else { m.radius };
        // Soft bottom edge gives keys some depth without heavy shadows.
        if !theme.dark && k.style != KeyStyle::Accent {
            theme.shadow.alpha(0.10).set(cr);
            icons::rounded(cr, r.x as f64, r.y as f64 + 1.0, r.w as f64, r.h as f64, radius as f64);
            cr.fill().ok();
        }
        bg.set(cr);
        icons::rounded(cr, r.x as f64, r.y as f64, r.w as f64, r.h as f64, radius as f64);
        cr.fill_preserve().ok();
        if let Some(b) = theme.key_border {
            b.set(cr);
            cr.set_line_width(1.0);
            cr.stroke().ok();
        } else {
            cr.new_path();
        }
        let (cx, cy) = r.center();
        let (cx, cy) = (cx as f64, cy as f64);
        fg.set(cr);
        match &k.label {
            KeyLabel::Text(t) => {
                let size = if t.chars().count() > 2 { m.small_label_size } else { m.label_size };
                self.text_centered(cr, t, cx, cy, size, false, r.w as f64 - 6.0);
            }
            KeyLabel::Icon(i) => {
                let sz = (r.h as f64 * 0.5).min(24.0);
                icons::draw(cr, *i, cx, cy, sz);
            }
            KeyLabel::Space(t) => {
                theme.hint.set(cr);
                self.text_centered(cr, t, cx, cy, m.small_label_size * 0.95, false, r.w as f64 - 8.0);
            }
            KeyLabel::None => {}
        }
        if let Some(h) = &k.hint {
            theme.hint.set(cr);
            self.text_centered(
                cr,
                h,
                (r.right() - m.hint_size * 0.75) as f64,
                (r.y + m.hint_size * 0.95) as f64,
                m.hint_size,
                false,
                0.0,
            );
        }
    }

    fn draw_strip(&self, cr: &cairo::Context, view: &View, theme: &Theme) {
        let m = &view.metrics;
        if let Some((msg, warn)) = &view.strip.message {
            if *warn { theme.delete_highlight } else { theme.strip_text }.set(cr);
            self.text_centered(
                cr,
                msg,
                view.width as f64 / 2.0,
                m.strip_height as f64 / 2.0,
                m.strip_font_size,
                false,
                view.width as f64 - 24.0,
            );
            return;
        }
        let n = view.strip.items.len();
        for (i, (r, item)) in view.strip.items.iter().enumerate() {
            if view.strip_pressed == Some(i) {
                theme.key_pressed.alpha(0.7).set(cr);
                icons::rounded(cr, r.x as f64 + 2.0, r.y as f64 + 4.0, r.w as f64 - 4.0, r.h as f64 - 8.0, 8.0);
                cr.fill().ok();
            }
            let (cx, cy) = r.center();
            match item {
                StripItem::Text { text, emphasized } => {
                    theme.strip_text.set(cr);
                    self.text_centered(
                        cr,
                        text,
                        cx as f64,
                        cy as f64,
                        m.strip_font_size,
                        *emphasized,
                        r.w as f64 - 10.0,
                    );
                    if view.strip.dividers && i + 1 < n {
                        theme.strip_divider.set(cr);
                        cr.rectangle(r.right() as f64 - 0.5, r.y as f64 + r.h as f64 * 0.28, 1.0, r.h as f64 * 0.44);
                        cr.fill().ok();
                    }
                }
                StripItem::Icon(icon) => {
                    theme.hint.set(cr);
                    icons::draw(cr, *icon, cx as f64, cy as f64, 22.0);
                }
            }
        }
    }

    fn draw_trail(&self, cr: &cairo::Context, view: &View, theme: &Theme) {
        let pts = &view.trail;
        if pts.len() < 2 {
            return;
        }
        let w = view.metrics.trail_width as f64;
        cr.set_line_cap(cairo::LineCap::Round);
        for seg in pts.windows(2) {
            let age = view.now.saturating_sub(seg[1].2) as f32;
            let life = 1.0 - (age / TRAIL_MS).clamp(0.0, 1.0);
            if life <= 0.0 {
                continue;
            }
            theme.trail.alpha(0.25 + 0.65 * life as f64).set(cr);
            cr.set_line_width(w * (0.3 + 0.7 * life as f64));
            cr.move_to(seg[0].0 as f64, seg[0].1 as f64);
            cr.line_to(seg[1].0 as f64, seg[1].1 as f64);
            cr.stroke().ok();
        }
    }

    fn draw_preview(&self, cr: &cairo::Context, k: &KeyView, text: &str, theme: &Theme, m: &Metrics) {
        let r = preview_rect(k.rect);
        theme.shadow.set(cr);
        icons::rounded(cr, r.x as f64, r.y as f64 + 2.0, r.w as f64, r.h as f64, m.radius as f64 * 1.3);
        cr.fill().ok();
        theme.popup.set(cr);
        icons::rounded(cr, r.x as f64, r.y as f64, r.w as f64, r.h as f64, m.radius as f64 * 1.3);
        cr.fill().ok();
        theme.label.set(cr);
        let (cx, _) = r.center();
        self.text_centered(
            cr,
            text,
            cx as f64,
            (r.y + r.h * 0.40) as f64,
            m.label_size * 1.35,
            false,
            r.w as f64 - 4.0,
        );
    }

    fn draw_popup(&self, cr: &cairo::Context, p: &Popup, theme: &Theme, m: &Metrics) {
        let r = p.rect;
        theme.shadow.set(cr);
        icons::rounded(cr, r.x as f64, r.y as f64 + 3.0, r.w as f64, r.h as f64, m.radius as f64 * 1.4);
        cr.fill().ok();
        theme.popup.set(cr);
        icons::rounded(cr, r.x as f64, r.y as f64, r.w as f64, r.h as f64, m.radius as f64 * 1.4);
        cr.fill().ok();
        for (i, (cell, e)) in p.cells.iter().zip(&p.entries).enumerate() {
            let sel = p.selected == Some(i);
            if sel {
                theme.popup_selected.set(cr);
                icons::rounded(
                    cr,
                    cell.x as f64 + 2.0,
                    cell.y as f64 + 2.0,
                    cell.w as f64 - 4.0,
                    cell.h as f64 - 4.0,
                    m.radius as f64,
                );
                cr.fill().ok();
            }
            if sel { theme.on_popup_selected } else { theme.label }.set(cr);
            let (cx, cy) = cell.center();
            match e {
                PopupEntry::Text(t) => {
                    let size = if t.chars().count() > 2 { m.small_label_size } else { m.label_size };
                    self.text_centered(cr, t, cx as f64, cy as f64, size, false, cell.w as f64 - 4.0)
                }
                PopupEntry::Icon(i) => icons::draw(cr, *i, cx as f64, cy as f64, 22.0),
            }
        }
    }

    fn draw_panel(&self, cr: &cairo::Context, panel: &Panel, theme: &Theme, m: &Metrics) {
        let r = panel.rect;
        theme.background.set(cr);
        cr.rectangle(r.x as f64, r.y as f64, r.w as f64, r.h as f64);
        cr.fill().ok();
        let draw_cell = |cr: &cairo::Context, rect: &Rect, cell: &PanelCell| {
            let (cx, cy) = rect.center();
            match cell {
                PanelCell::Emoji(e) => {
                    theme.label.set(cr);
                    self.text_centered(cr, e, cx as f64, cy as f64, (rect.h * 0.62).min(34.0), false, 0.0);
                }
                PanelCell::Text { text, secondary } => {
                    theme.key.set(cr);
                    icons::rounded(
                        cr,
                        rect.x as f64 + 3.0,
                        rect.y as f64 + 3.0,
                        rect.w as f64 - 6.0,
                        rect.h as f64 - 6.0,
                        m.radius as f64,
                    );
                    cr.fill().ok();
                    theme.label.set(cr);
                    let layout = self.layout(cr, text, m.small_label_size, false);
                    layout.set_width(((rect.w - 22.0) as i32) * pango::SCALE);
                    layout.set_height(((rect.h - 16.0) as i32) * pango::SCALE);
                    layout.set_ellipsize(pango::EllipsizeMode::End);
                    layout.set_wrap(pango::WrapMode::WordChar);
                    cr.move_to(rect.x as f64 + 11.0, rect.y as f64 + 9.0);
                    pangocairo::functions::show_layout(cr, &layout);
                    if let Some(s) = secondary {
                        theme.hint.set(cr);
                        let l2 = self.layout(cr, s, m.hint_size, false);
                        let (_, lr) = l2.pixel_extents();
                        cr.move_to(
                            (rect.right() - 10.0) as f64 - lr.width() as f64,
                            (rect.bottom() - 8.0) as f64 - lr.height() as f64,
                        );
                        pangocairo::functions::show_layout(cr, &l2);
                    }
                }
                PanelCell::Icon { icon, active } => {
                    if *active { theme.accent } else { theme.hint }.set(cr);
                    icons::draw(cr, *icon, cx as f64, cy as f64, 22.0);
                }
                PanelCell::Tab { text, active } => {
                    if *active {
                        theme.key_function.set(cr);
                        icons::rounded(
                            cr,
                            rect.x as f64 + 2.0,
                            rect.y as f64 + 4.0,
                            rect.w as f64 - 4.0,
                            rect.h as f64 - 8.0,
                            (rect.h as f64 - 8.0) / 2.0,
                        );
                        cr.fill().ok();
                    }
                    theme.label.set(cr);
                    self.text_centered(
                        cr,
                        text,
                        cx as f64,
                        cy as f64,
                        (rect.h * 0.5).min(22.0),
                        false,
                        rect.w as f64 - 4.0,
                    );
                }
                PanelCell::Button { text } => {
                    theme.key_function.set(cr);
                    icons::rounded(
                        cr,
                        rect.x as f64 + 3.0,
                        rect.y as f64 + 3.0,
                        rect.w as f64 - 6.0,
                        rect.h as f64 - 6.0,
                        m.radius as f64,
                    );
                    cr.fill().ok();
                    theme.label.set(cr);
                    self.text_centered(cr, text, cx as f64, cy as f64, m.small_label_size, false, rect.w as f64 - 10.0);
                }
            }
        };
        cr.save().ok();
        cr.rectangle(r.x as f64, r.y as f64, r.w as f64, r.h as f64);
        cr.clip();
        for (rect, cell) in &panel.cells {
            let rr = Rect::new(rect.x, rect.y - panel.scroll, rect.w, rect.h);
            if rr.bottom() < r.y || rr.y > r.bottom() {
                continue;
            }
            draw_cell(cr, &rr, cell);
        }
        cr.restore().ok();
        for (rect, cell) in panel.header.iter().chain(panel.footer.iter()) {
            draw_cell(cr, rect, cell);
        }
    }
}

/// Bubble shown above a pressed key.
pub fn preview_rect(k: Rect) -> Rect {
    let w = k.w.max(k.h * 0.8) + 8.0;
    let h = k.h * 1.25;
    Rect::new(k.x + k.w / 2.0 - w / 2.0, (k.y - h - 4.0).max(0.0), w, h)
}

/// Lays out the long-press popup above `anchor`, inside `bounds`.
pub fn popup_layout(anchor: Rect, n: usize, bounds: Rect, cell_w: f32, cell_h: f32) -> (Rect, Vec<Rect>) {
    let max_cols = ((bounds.w - 8.0) / cell_w).floor().max(1.0) as usize;
    let cols = n.clamp(1, max_cols.min(8));
    let rows = n.div_ceil(cols);
    let w = cols as f32 * cell_w + 8.0;
    let h = rows as f32 * cell_h + 8.0;
    let mut x = anchor.x + anchor.w / 2.0 - cell_w / 2.0 - 4.0;
    x = x.min(bounds.right() - w - 2.0).max(bounds.x + 2.0);
    let y = (anchor.y - h + 2.0).max(bounds.y);
    let mut cells = Vec::with_capacity(n);
    for i in 0..n {
        // Fill the bottom row first so the default entry is nearest the finger.
        let row = rows - 1 - i / cols;
        let col = i % cols;
        cells.push(Rect::new(x + 4.0 + col as f32 * cell_w, y + 4.0 + row as f32 * cell_h, cell_w, cell_h));
    }
    (Rect::new(x, y, w, h), cells)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popup_stays_in_bounds() {
        let bounds = Rect::new(0.0, 0.0, 360.0, 260.0);
        let (r, cells) = popup_layout(Rect::new(330.0, 60.0, 30.0, 44.0), 10, bounds, 36.0, 44.0);
        assert!(r.x >= 0.0 && r.right() <= 360.0);
        assert_eq!(cells.len(), 10);
        assert!(cells[0].bottom() > cells[9].bottom(), "first entry in the bottom row");
    }

    #[test]
    fn renders_without_panicking() {
        let m = Metrics::new(false, 1.0, 1.0);
        let view = View {
            width: 360.0,
            height: 260.0,
            metrics: m,
            keys: vec![
                KeyView {
                    rect: Rect::new(4.0, 50.0, 30.0, 44.0),
                    label: KeyLabel::Text("q".into()),
                    hint: Some("1".into()),
                    style: KeyStyle::Normal,
                },
                KeyView {
                    rect: Rect::new(40.0, 50.0, 60.0, 44.0),
                    label: KeyLabel::Icon(Icon::Backspace),
                    hint: None,
                    style: KeyStyle::Function,
                },
                KeyView {
                    rect: Rect::new(110.0, 50.0, 60.0, 44.0),
                    label: KeyLabel::Icon(Icon::Enter),
                    hint: None,
                    style: KeyStyle::Accent,
                },
            ],
            keys_rev: 1,
            pressed: vec![0],
            preview: Some((0, "q".into())),
            popup: None,
            strip: StripView {
                items: vec![(
                    Rect::new(0.0, 0.0, 120.0, 44.0),
                    StripItem::Text { text: "hello".into(), emphasized: true },
                )],
                dividers: true,
                message: None,
            },
            strip_pressed: None,
            trail: vec![(10.0, 60.0, 0), (50.0, 80.0, 10), (90.0, 70.0, 20)],
            now: 30,
            panel: None,
            side_area: None,
        };
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 720, 520).unwrap();
        let mut r = Renderer::new("Sans");
        r.render(&surface, 2.0, &view, &Theme::light());
        r.render(&surface, 2.0, &view, &Theme::dark());
        let data = surface.take_data().unwrap();
        // Something other than the background got drawn.
        let first = &data[0..4].to_vec();
        assert!(data.chunks(4).any(|p| p != first.as_slice()));
    }
}
