//! Full-keyboard panels: emoji picker, clipboard history and the quick menu.

use crate::clipboard;
use crate::emoji::{EmojiDb, Recents, group_icon};
use crate::i18n::tr;
use crate::icons::Icon;
use crate::render::{Panel, PanelCell};
use glissando_config::{Config, OneHanded, ThemeMode};
use glissando_core::geometry::Rect;
use std::cell::{OnceCell, RefCell};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelKind {
    Emoji,
    Clipboard,
    Menu,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PanelHit {
    Commit(String),
    Paste(String),
    Backspace,
    Close,
    Config(Config),
    OpenSettings,
    ClearPersonalData,
    /// Start emoji search with the keyboard.
    Search,
    Redraw,
}

#[derive(Debug, Clone, PartialEq)]
enum CellAction {
    Emoji(String),
    Group(usize),
    Recent,
    Clip(String),
    ClearClips,
    Close,
    Search,
    Backspace,
    Toggle(MenuItem),
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuItem {
    Settings,
    OneHanded,
    NumberRow,
    Glide,
    Theme,
    Taller,
    Shorter,
    Clipboard,
    ClearData,
}

thread_local! {
    static EMOJI: OnceCell<EmojiDb> = const { OnceCell::new() };
    pub static RECENTS: RefCell<Recents> = RefCell::new(Recents::default());
}

pub fn init_emoji(db: EmojiDb, recents: Recents) {
    EMOJI.with(|c| {
        let _ = c.set(db);
    });
    RECENTS.with(|r| *r.borrow_mut() = recents);
}

fn with_db<R>(f: impl FnOnce(&EmojiDb) -> R) -> R {
    EMOJI.with(|c| f(c.get_or_init(EmojiDb::default)))
}

pub fn search_emoji(query: &str, max: usize) -> Vec<String> {
    with_db(|db| db.search(query, max).into_iter().map(|e| e.text.clone()).collect())
}

pub fn emoji_for_word(word: &str) -> Vec<String> {
    with_db(|db| db.for_word(word, 2).into_iter().map(|e| e.text.clone()).collect())
}

pub struct PanelState {
    pub kind: PanelKind,
    area: Rect,
    pub(crate) config: Config,
    cells: Vec<(Rect, PanelCell, CellAction)>,
    footer: Vec<(Rect, PanelCell, CellAction)>,
    /// Content offset of each emoji group (for tabs).
    group_offsets: Vec<(Option<usize>, f32)>,
    scroll: f32,
    max_scroll: f32,
    drag: Option<Drag>,
    velocity: f32,
    last_tick: u32,
    flinging: bool,
    long_press_at: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
struct Drag {
    id: u32,
    y0: f32,
    scroll0: f32,
    last_y: f32,
    last_t: u32,
    moved: bool,
    long_pressed: bool,
}

const FOOTER_H: f32 = 44.0;

impl PanelState {
    pub fn new(kind: PanelKind, area: Rect, config: &Config) -> PanelState {
        let mut p = PanelState {
            kind,
            area,
            config: config.clone(),
            cells: Vec::new(),
            footer: Vec::new(),
            group_offsets: Vec::new(),
            scroll: 0.0,
            max_scroll: 0.0,
            drag: None,
            velocity: 0.0,
            last_tick: 0,
            flinging: false,
            long_press_at: None,
        };
        p.build();
        p
    }

    pub fn refresh(&mut self, config: &Config) {
        self.config = config.clone();
        let s = self.scroll;
        self.build();
        self.scroll = s.min(self.max_scroll);
    }

    fn content_rect(&self) -> Rect {
        Rect::new(self.area.x, self.area.y, self.area.w, self.area.h - FOOTER_H)
    }

    fn build(&mut self) {
        self.cells.clear();
        self.footer.clear();
        self.group_offsets.clear();
        let c = self.content_rect();
        let fy = c.bottom();
        match self.kind {
            PanelKind::Emoji => {
                let cols = (c.w / 44.0).floor().max(6.0) as usize;
                let cw = c.w / cols as f32;
                let ch = 46.0;
                let mut y = c.y + 4.0;
                let push_row = |items: Vec<String>, y: &mut f32, cells: &mut Vec<(Rect, PanelCell, CellAction)>| {
                    for (i, e) in items.into_iter().enumerate() {
                        let col = i % cols;
                        if col == 0 && i > 0 {
                            *y += ch;
                        }
                        cells.push((
                            Rect::new(c.x + col as f32 * cw, *y, cw, ch),
                            PanelCell::Emoji(e.clone()),
                            CellAction::Emoji(e),
                        ));
                    }
                    *y += ch + 8.0;
                };
                let recents: Vec<String> = RECENTS.with(|r| r.borrow().items().cloned().collect());
                if !recents.is_empty() {
                    self.group_offsets.push((None, y - c.y - 4.0));
                    push_row(recents, &mut y, &mut self.cells);
                }
                let groups: Vec<(String, Vec<String>)> = with_db(|db| {
                    db.groups
                        .iter()
                        .enumerate()
                        .map(|(gi, g)| (g.clone(), db.in_group(gi).map(|e| e.text.clone()).collect()))
                        .collect()
                });
                for (gi, (_, items)) in groups.iter().enumerate() {
                    self.group_offsets.push((Some(gi), y - c.y - 4.0));
                    push_row(items.clone(), &mut y, &mut self.cells);
                }
                self.max_scroll = (y - c.bottom()).max(0.0);
                // Footer: ABC, group tabs, backspace.
                let tabs: Vec<(PanelCell, CellAction)> =
                    std::iter::once((PanelCell::Icon { icon: Icon::Recent, active: false }, CellAction::Recent))
                        .chain(groups.iter().enumerate().map(|(gi, (g, _))| {
                            (PanelCell::Tab { text: group_icon(g).to_string(), active: false }, CellAction::Group(gi))
                        }))
                        .collect();
                let side = 52.0;
                self.footer.push((
                    Rect::new(c.x, fy, side, FOOTER_H),
                    PanelCell::Button { text: "ABC".into() },
                    CellAction::Close,
                ));
                self.footer.push((
                    Rect::new(c.x + side, fy, 40.0, FOOTER_H),
                    PanelCell::Icon { icon: Icon::Search, active: false },
                    CellAction::Search,
                ));
                let left = side + 40.0;
                let tw = (c.w - left - side) / tabs.len() as f32;
                for (i, (cell, act)) in tabs.into_iter().enumerate() {
                    self.footer.push((Rect::new(c.x + left + i as f32 * tw, fy, tw, FOOTER_H), cell, act));
                }
                self.footer.push((
                    Rect::new(c.right() - side, fy, side, FOOTER_H),
                    PanelCell::Icon { icon: Icon::Backspace, active: false },
                    CellAction::Backspace,
                ));
            }
            PanelKind::Clipboard => {
                let items = clipboard::HISTORY.with(|h| h.borrow_mut().items());
                let enabled = clipboard::HISTORY.with(|h| h.borrow().enabled);
                let cols = 2;
                let cw = c.w / cols as f32;
                let ch = 66.0;
                if items.is_empty() {
                    let msg = if enabled { tr("clipboard-empty") } else { tr("clipboard-off") };
                    self.cells.push((
                        Rect::new(c.x + 8.0, c.y + 8.0, c.w - 16.0, ch),
                        PanelCell::Text { text: msg.into(), secondary: None },
                        CellAction::None,
                    ));
                }
                for (i, clip) in items.iter().enumerate() {
                    let r = Rect::new(c.x + (i % cols) as f32 * cw, c.y + 4.0 + (i / cols) as f32 * ch, cw, ch);
                    let preview: String = clip.text.chars().take(160).collect();
                    self.cells.push((
                        r,
                        PanelCell::Text { text: preview, secondary: clip.pinned.then(|| "📌".to_string()) },
                        CellAction::Clip(clip.text.clone()),
                    ));
                }
                let rows = items.len().div_ceil(cols) as f32;
                self.max_scroll = (rows * ch + 8.0 - c.h).max(0.0);
                self.footer.push((
                    Rect::new(c.x, fy, 72.0, FOOTER_H),
                    PanelCell::Button { text: "ABC".into() },
                    CellAction::Close,
                ));
                self.footer.push((
                    Rect::new(c.right() - 140.0, fy, 140.0, FOOTER_H),
                    PanelCell::Button { text: tr("clear").into() },
                    CellAction::ClearClips,
                ));
            }
            PanelKind::Menu => {
                let cfg = &self.config;
                let theme = match cfg.theme {
                    ThemeMode::System => tr("theme-system"),
                    ThemeMode::Light => tr("theme-light"),
                    ThemeMode::Dark => tr("theme-dark"),
                };
                let onoff = |b: bool| if b { "✓ " } else { "" };
                let items = [
                    (format!("⚙ {}", tr("settings")), MenuItem::Settings),
                    (format!("{}{}", onoff(cfg.one_handed != OneHanded::Off), tr("one-handed")), MenuItem::OneHanded),
                    (format!("{}{}", onoff(cfg.number_row), tr("number-row")), MenuItem::NumberRow),
                    (format!("{}{}", onoff(cfg.glide), tr("glide")), MenuItem::Glide),
                    (theme.to_string(), MenuItem::Theme),
                    (format!("{}{}", onoff(cfg.clipboard_history), tr("clipboard-history")), MenuItem::Clipboard),
                    (format!("▲ {}", tr("taller")), MenuItem::Taller),
                    (format!("▼ {}", tr("shorter")), MenuItem::Shorter),
                    (tr("clear-learned").to_string(), MenuItem::ClearData),
                ];
                let cols = 3;
                let cw = c.w / cols as f32;
                let rows = items.len().div_ceil(cols);
                let ch = ((c.h - 8.0) / rows as f32).min(64.0);
                for (i, (text, item)) in items.into_iter().enumerate() {
                    let r = Rect::new(c.x + (i % cols) as f32 * cw, c.y + 4.0 + (i / cols) as f32 * ch, cw, ch);
                    self.cells.push((r, PanelCell::Button { text }, CellAction::Toggle(item)));
                }
                self.max_scroll = 0.0;
                self.footer.push((
                    Rect::new(c.x, fy, 72.0, FOOTER_H),
                    PanelCell::Button { text: "ABC".into() },
                    CellAction::Close,
                ));
            }
        }
    }

    fn active_group(&self) -> Option<CellAction> {
        let mut cur = None;
        for (g, off) in &self.group_offsets {
            if *off <= self.scroll + 2.0 {
                cur = Some(*g);
            }
        }
        cur.map(|g| match g {
            Some(i) => CellAction::Group(i),
            None => CellAction::Recent,
        })
    }

    pub fn view(&self) -> Panel {
        let active = self.active_group();
        let footer = self
            .footer
            .iter()
            .map(|(r, cell, act)| {
                let cell = match cell {
                    PanelCell::Tab { text, .. } => {
                        PanelCell::Tab { text: text.clone(), active: Some(act) == active.as_ref() }
                    }
                    PanelCell::Icon { icon, .. } if *act == CellAction::Recent => {
                        PanelCell::Icon { icon: *icon, active: Some(act) == active.as_ref() }
                    }
                    other => other.clone(),
                };
                (*r, cell)
            })
            .collect();
        Panel {
            rect: self.content_rect(),
            cells: self.cells.iter().map(|(r, c, _)| (*r, c.clone())).collect(),
            scroll: self.scroll,
            header: Vec::new(),
            footer,
        }
    }

    pub fn animating(&self) -> bool {
        self.flinging
    }

    pub fn next_deadline(&self) -> Option<u32> {
        self.long_press_at
    }

    pub fn down(&mut self, id: u32, _x: f32, y: f32, t: u32) {
        self.flinging = false;
        self.velocity = 0.0;
        self.drag =
            Some(Drag { id, y0: y, scroll0: self.scroll, last_y: y, last_t: t, moved: false, long_pressed: false });
        self.long_press_at = Some(t + 450);
    }

    pub fn motion(&mut self, id: u32, _x: f32, y: f32, t: u32) {
        let Some(d) = &mut self.drag else { return };
        if d.id != id {
            return;
        }
        if (y - d.y0).abs() > 10.0 {
            d.moved = true;
            self.long_press_at = None;
        }
        if d.moved {
            let dt = t.saturating_sub(d.last_t).max(1) as f32;
            let v = (d.last_y - y) / dt;
            self.velocity = 0.6 * v + 0.4 * self.velocity;
            self.scroll = (d.scroll0 + d.y0 - y).clamp(0.0, self.max_scroll);
        }
        d.last_y = y;
        d.last_t = t;
    }

    /// Long press on a clipboard entry toggles its pin.
    pub fn tick(&mut self, now: u32) {
        if let (Some(at), Some(d)) = (self.long_press_at, &mut self.drag)
            && now >= at
            && !d.moved
        {
            d.long_pressed = true;
            self.long_press_at = None;
            if self.kind == PanelKind::Clipboard
                && let Some((_, _, CellAction::Clip(text))) =
                    self.cells.iter().find(|(r, _, _)| r.contains(r.x + 1.0, d.y0 + self.scroll)).cloned()
            {
                clipboard::HISTORY.with(|h| h.borrow_mut().toggle_pin(&text));
                self.build();
            }
        }
        if self.flinging {
            let dt = now.saturating_sub(self.last_tick).clamp(1, 50) as f32;
            self.scroll = (self.scroll + self.velocity * dt).clamp(0.0, self.max_scroll);
            self.velocity *= (0.995f32).powf(dt);
            if self.velocity.abs() < 0.02 || self.scroll <= 0.0 || self.scroll >= self.max_scroll {
                self.flinging = false;
            }
        }
        self.last_tick = now;
    }

    pub fn up(&mut self, id: u32, x: f32, y: f32, t: u32) -> Option<PanelHit> {
        let d = self.drag.take()?;
        self.long_press_at = None;
        if d.id != id {
            return None;
        }
        if d.moved {
            if self.velocity.abs() > 0.15 {
                self.flinging = true;
                self.last_tick = t;
            }
            return Some(PanelHit::Redraw);
        }
        if d.long_pressed {
            return Some(PanelHit::Redraw);
        }
        if let Some((_, _, act)) = self.footer.iter().find(|(r, _, _)| r.contains(x, y)).cloned() {
            return self.act(act);
        }
        let content = self.content_rect();
        if !content.contains(x, y) {
            return None;
        }
        let cy = y + self.scroll;
        let act = self.cells.iter().find(|(r, _, _)| r.contains(x, cy)).map(|c| c.2.clone())?;
        self.act(act)
    }

    fn act(&mut self, act: CellAction) -> Option<PanelHit> {
        match act {
            CellAction::Emoji(e) => {
                if self.config.emoji_recent {
                    RECENTS.with(|r| r.borrow_mut().used(&e));
                }
                Some(PanelHit::Commit(e))
            }
            CellAction::Group(g) => {
                if let Some((_, off)) = self.group_offsets.iter().find(|(gg, _)| *gg == Some(g)) {
                    self.scroll = off.min(self.max_scroll);
                }
                Some(PanelHit::Redraw)
            }
            CellAction::Recent => {
                self.scroll = 0.0;
                Some(PanelHit::Redraw)
            }
            CellAction::Clip(t) => Some(PanelHit::Paste(t)),
            CellAction::ClearClips => {
                clipboard::HISTORY.with(|h| h.borrow_mut().clear());
                self.build();
                Some(PanelHit::Redraw)
            }
            CellAction::Close => Some(PanelHit::Close),
            CellAction::Search => Some(PanelHit::Search),
            CellAction::Backspace => Some(PanelHit::Backspace),
            CellAction::Toggle(item) => {
                let mut c = self.config.clone();
                match item {
                    MenuItem::Settings => return Some(PanelHit::OpenSettings),
                    MenuItem::ClearData => return Some(PanelHit::ClearPersonalData),
                    MenuItem::OneHanded => {
                        c.one_handed = if c.one_handed == OneHanded::Off { OneHanded::Right } else { OneHanded::Off }
                    }
                    MenuItem::NumberRow => c.number_row = !c.number_row,
                    MenuItem::Glide => c.glide = !c.glide,
                    MenuItem::Theme => {
                        c.theme = match c.theme {
                            ThemeMode::System => ThemeMode::Light,
                            ThemeMode::Light => ThemeMode::Dark,
                            ThemeMode::Dark => ThemeMode::System,
                        }
                    }
                    MenuItem::Clipboard => c.clipboard_history = !c.clipboard_history,
                    MenuItem::Taller => c.height = (c.height + 0.08).min(1.4),
                    MenuItem::Shorter => c.height = (c.height - 0.08).max(0.7),
                }
                Some(PanelHit::Config(c))
            }
            CellAction::None => None,
        }
    }
}
