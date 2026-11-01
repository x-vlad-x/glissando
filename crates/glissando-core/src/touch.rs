//! Touch interpretation: taps, rollover, long press, auto-repeat, glide
//! typing detection, space bar cursor control and backspace glide delete.
//!
//! The machine is deliberately free of rendering and IO. The caller feeds
//! pointer events and timer ticks (`now` in milliseconds, any monotonic
//! origin) and acts on the returned [`TouchEvent`]s. Thresholds default to
//! values tuned to feel like mainstream phone keyboards (see
//! `docs/research/reference-behaviour.md`).

use crate::geometry::LayerGeometry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    /// Text key. `letter` keys may start glide typing; `popup` keys have long-press entries.
    Text {
        letter: bool,
        popup: bool,
    },
    Space,
    Backspace,
    Shift,
    /// Arrow keys: activate on release, repeat while held.
    Repeat,
    /// Everything else; activates on release.
    Other {
        popup: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct TouchConfig {
    pub long_press_ms: u32,
    pub repeat_delay_ms: u32,
    pub repeat_interval_ms: u32,
    pub double_tap_ms: u32,
    pub swipe_enabled: bool,
    /// Glide starts after leaving the start key and travelling this many key units...
    pub swipe_min_units: f32,
    /// ...or after travelling this many units regardless of the key boundary.
    pub swipe_force_units: f32,
    pub space_cursor: bool,
    pub cursor_trigger_px: f32,
    pub cursor_step_px: f32,
    pub delete_glide: bool,
    pub delete_trigger_px: f32,
    pub delete_word_px: f32,
    /// Vertical distance a scrub may wander off its row before it is cancelled.
    pub scrub_slack_px: f32,
    pub trackpad_on_space_long_press: bool,
    pub trackpad_step_px: f32,
}

impl Default for TouchConfig {
    fn default() -> Self {
        TouchConfig {
            long_press_ms: 300,
            repeat_delay_ms: 400,
            repeat_interval_ms: 50,
            double_tap_ms: 350,
            swipe_enabled: true,
            swipe_min_units: 0.5,
            swipe_force_units: 1.5,
            space_cursor: true,
            cursor_trigger_px: 18.0,
            cursor_step_px: 11.0,
            delete_glide: true,
            delete_trigger_px: 18.0,
            delete_word_px: 36.0,
            scrub_slack_px: 25.0,
            trackpad_on_space_long_press: false,
            trackpad_step_px: 12.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TouchEvent {
    /// Visual/haptic press feedback for a key.
    Pressed {
        key: usize,
    },
    Released {
        key: usize,
    },
    /// The key was tapped (x, y is the touch point used by the spatial model).
    Activate {
        key: usize,
        x: f32,
        y: f32,
    },
    /// Shift was double tapped: caps lock.
    CapsLock {
        key: usize,
    },
    /// Shift is released after it was used as a held modifier.
    ShiftChordEnd {
        key: usize,
    },
    LongPress {
        key: usize,
        x: f32,
        y: f32,
    },
    PopupMove {
        x: f32,
        y: f32,
    },
    PopupCommit,
    Repeat {
        key: usize,
        count: u32,
    },
    CursorStart,
    /// Relative cursor movement in characters (negative = left).
    CursorMove {
        dx: i32,
        dy: i32,
    },
    CursorEnd,
    TrackpadStart,
    DeletePreview {
        words: u32,
    },
    DeleteCommit {
        words: u32,
    },
    DeleteCancel,
    SwipeStart {
        key: usize,
    },
    SwipeMove {
        x: f32,
        y: f32,
    },
    SwipeEnd,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TracePoint {
    pub x: f32,
    pub y: f32,
    pub t: u32,
}

#[derive(Debug, Clone, PartialEq)]
enum Mode {
    Tap,
    Popup,
    Repeating { count: u32 },
    Swipe,
    Cursor { sent: i32 },
    Trackpad { sx: i32, sy: i32 },
    Delete { words: u32 },
    ShiftHeld { used: bool },
    Done,
}

#[derive(Debug, Clone)]
struct Pointer {
    id: u32,
    key: Option<usize>,
    start: TracePoint,
    last: TracePoint,
    mode: Mode,
    deadline: Option<u32>,
    /// Anchor for relative scrubs (trackpad resets it).
    anchor: (f32, f32),
}

#[derive(Debug, Default)]
pub struct TouchMachine {
    pub config: TouchConfig,
    kinds: Vec<KeyKind>,
    geom: LayerGeometry,
    pointers: Vec<Pointer>,
    trace: Vec<TracePoint>,
    last_shift_tap: Option<u32>,
}

impl TouchMachine {
    pub fn new(config: TouchConfig) -> TouchMachine {
        TouchMachine { config, ..Default::default() }
    }

    /// Install the current layer. Active touches are cancelled.
    pub fn set_layer(&mut self, geom: LayerGeometry, kinds: Vec<KeyKind>) -> Vec<TouchEvent> {
        let out = self.cancel_all();
        assert_eq!(geom.keys.len(), kinds.len());
        self.geom = geom;
        self.kinds = kinds;
        out
    }

    pub fn geometry(&self) -> &LayerGeometry {
        &self.geom
    }

    /// Points of the glide in progress (or the one that just ended).
    pub fn trace(&self) -> &[TracePoint] {
        &self.trace
    }

    pub fn is_active(&self) -> bool {
        !self.pointers.is_empty()
    }

    pub fn swiping(&self) -> bool {
        self.pointers.iter().any(|p| p.mode == Mode::Swipe)
    }

    pub fn next_deadline(&self) -> Option<u32> {
        self.pointers.iter().filter_map(|p| p.deadline).min()
    }

    pub fn cancel_all(&mut self) -> Vec<TouchEvent> {
        let mut out = Vec::new();
        for p in self.pointers.drain(..) {
            match p.mode {
                Mode::Delete { .. } => out.push(TouchEvent::DeleteCancel),
                Mode::Cursor { .. } | Mode::Trackpad { .. } => out.push(TouchEvent::CursorEnd),
                _ => {}
            }
            if let Some(k) = p.key {
                out.push(TouchEvent::Released { key: k });
            }
        }
        self.trace.clear();
        out
    }

    fn kind(&self, key: usize) -> KeyKind {
        self.kinds.get(key).copied().unwrap_or(KeyKind::Other { popup: false })
    }

    pub fn down(&mut self, id: u32, x: f32, y: f32, t: u32) -> Vec<TouchEvent> {
        let mut out = Vec::new();
        // A glide, popup or scrub owns the keyboard until it ends.
        if self.pointers.iter().any(|p| {
            matches!(
                p.mode,
                Mode::Swipe | Mode::Popup | Mode::Cursor { .. } | Mode::Delete { .. } | Mode::Trackpad { .. }
            )
        }) {
            return out;
        }
        // Rollover: a second finger finishes pending taps on text keys.
        let mut i = 0;
        while i < self.pointers.len() {
            let p = &self.pointers[i];
            let finish = p.mode == Mode::Tap
                && p.key.map(|k| matches!(self.kind(k), KeyKind::Text { .. } | KeyKind::Space)).unwrap_or(false);
            if finish {
                let p = self.pointers.remove(i);
                let k = p.key.unwrap();
                out.push(TouchEvent::Activate { key: k, x: p.last.x, y: p.last.y });
                out.push(TouchEvent::Released { key: k });
            } else {
                i += 1;
            }
        }
        for p in self.pointers.iter_mut() {
            if let Mode::ShiftHeld { used } = &mut p.mode {
                *used = true;
            }
        }
        let key = self.geom.hit(x, y);
        let tp = TracePoint { x, y, t };
        let mut p = Pointer { id, key, start: tp, last: tp, mode: Mode::Tap, deadline: None, anchor: (x, y) };
        if let Some(k) = key {
            out.push(TouchEvent::Pressed { key: k });
            match self.kind(k) {
                KeyKind::Shift => {
                    let dbl =
                        self.last_shift_tap.map(|lt| t.wrapping_sub(lt) <= self.config.double_tap_ms).unwrap_or(false);
                    if dbl {
                        self.last_shift_tap = None;
                        out.push(TouchEvent::CapsLock { key: k });
                        p.mode = Mode::Done;
                    } else {
                        self.last_shift_tap = Some(t);
                        out.push(TouchEvent::Activate { key: k, x, y });
                        p.mode = Mode::ShiftHeld { used: false };
                        p.deadline = Some(t + self.config.long_press_ms);
                    }
                }
                KeyKind::Backspace | KeyKind::Repeat => p.deadline = Some(t + self.config.repeat_delay_ms),
                KeyKind::Text { .. } | KeyKind::Other { .. } | KeyKind::Space => {
                    p.deadline = Some(t + self.config.long_press_ms)
                }
            }
        }
        self.trace.clear();
        self.trace.push(tp);
        self.pointers.push(p);
        out
    }

    pub fn motion(&mut self, id: u32, x: f32, y: f32, t: u32) -> Vec<TouchEvent> {
        let mut out = Vec::new();
        let Some(pi) = self.pointers.iter().position(|p| p.id == id) else { return out };
        let single = self.pointers.len() == 1;
        let unit = self.geom.unit.max(1.0);
        let cfg = self.config.clone();
        let kind = self.pointers[pi].key.map(|k| self.kind(k));
        let key_hit = self.pointers[pi].key.map(|k| self.geom.keys[k].hit);
        let tp = TracePoint { x, y, t };
        let p = &mut self.pointers[pi];
        p.last = tp;
        let dx = x - p.start.x;
        let dy = y - p.start.y;
        let dist = (dx * dx + dy * dy).sqrt();
        match p.mode.clone() {
            Mode::Tap => match kind {
                Some(KeyKind::Text { letter: true, .. }) if cfg.swipe_enabled && single => {
                    let left_key = key_hit.map(|r| !r.contains(x, y)).unwrap_or(true);
                    if (left_key && dist > cfg.swipe_min_units * unit) || dist > cfg.swipe_force_units * unit {
                        let k = p.key.take().unwrap();
                        p.mode = Mode::Swipe;
                        p.deadline = None;
                        out.push(TouchEvent::Released { key: k });
                        out.push(TouchEvent::SwipeStart { key: k });
                        self.trace.push(tp);
                        out.push(TouchEvent::SwipeMove { x, y });
                    } else {
                        self.trace.push(tp);
                    }
                }
                Some(KeyKind::Space) if cfg.space_cursor && dx.abs() > cfg.cursor_trigger_px => {
                    p.mode = Mode::Cursor { sent: 0 };
                    p.deadline = None;
                    out.push(TouchEvent::CursorStart);
                    Self::scrub_cursor(p, &cfg, &mut out);
                }
                Some(KeyKind::Backspace) if cfg.delete_glide && dx < -cfg.delete_trigger_px => {
                    p.mode = Mode::Delete { words: 0 };
                    p.deadline = None;
                    Self::scrub_delete(p, &cfg, &mut out);
                }
                Some(KeyKind::Space) | Some(KeyKind::Backspace) | Some(KeyKind::Shift) => {}
                _ => {
                    // Slide-to-adjust: the key under the finger at release wins.
                    let nk = self.geom.hit(x, y);
                    let p = &mut self.pointers[pi];
                    if nk != p.key {
                        if let Some(k) = p.key {
                            out.push(TouchEvent::Released { key: k });
                        }
                        if let Some(k) = nk {
                            out.push(TouchEvent::Pressed { key: k });
                        }
                        p.key = nk;
                        p.deadline = nk.map(|_| t + cfg.long_press_ms);
                    }
                }
            },
            Mode::Swipe => {
                self.trace.push(tp);
                out.push(TouchEvent::SwipeMove { x, y });
            }
            Mode::Popup => out.push(TouchEvent::PopupMove { x, y }),
            Mode::Cursor { .. } => {
                if dy.abs() > cfg.scrub_slack_px + unit {
                    // Gboard keeps the cursor where it is when the finger leaves the row.
                    p.mode = Mode::Done;
                    out.push(TouchEvent::CursorEnd);
                } else {
                    Self::scrub_cursor(p, &cfg, &mut out);
                }
            }
            Mode::Trackpad { sx, sy } => {
                let nx = ((x - p.anchor.0) / cfg.trackpad_step_px).trunc() as i32;
                let ny = ((y - p.anchor.1) / (cfg.trackpad_step_px * 2.0)).trunc() as i32;
                if nx != sx || ny != sy {
                    out.push(TouchEvent::CursorMove { dx: nx - sx, dy: ny - sy });
                    p.mode = Mode::Trackpad { sx: nx, sy: ny };
                }
            }
            Mode::Delete { .. } => {
                if dy.abs() > cfg.scrub_slack_px + unit {
                    p.mode = Mode::Done;
                    out.push(TouchEvent::DeleteCancel);
                } else {
                    Self::scrub_delete(p, &cfg, &mut out);
                }
            }
            Mode::Repeating { .. } | Mode::ShiftHeld { .. } | Mode::Done => {}
        }
        out
    }

    fn scrub_cursor(p: &mut Pointer, cfg: &TouchConfig, out: &mut Vec<TouchEvent>) {
        let Mode::Cursor { sent } = p.mode else { return };
        let dx = p.last.x - p.start.x;
        let travel = (dx.abs() - cfg.cursor_trigger_px).max(0.0);
        let steps = dx.signum() as i32 * (1 + (travel / cfg.cursor_step_px) as i32);
        if steps != sent {
            out.push(TouchEvent::CursorMove { dx: steps - sent, dy: 0 });
            p.mode = Mode::Cursor { sent: steps };
        }
    }

    fn scrub_delete(p: &mut Pointer, cfg: &TouchConfig, out: &mut Vec<TouchEvent>) {
        let Mode::Delete { words } = p.mode else { return };
        let dx = p.start.x - p.last.x;
        let n =
            if dx < cfg.delete_trigger_px { 0 } else { 1 + ((dx - cfg.delete_trigger_px) / cfg.delete_word_px) as u32 };
        if n != words {
            out.push(TouchEvent::DeletePreview { words: n });
            p.mode = Mode::Delete { words: n };
        }
    }

    pub fn up(&mut self, id: u32, t: u32) -> Vec<TouchEvent> {
        let mut out = Vec::new();
        let Some(pi) = self.pointers.iter().position(|p| p.id == id) else { return out };
        let mut p = self.pointers.remove(pi);
        p.last.t = t;
        match p.mode {
            Mode::Tap => {
                if let Some(k) = p.key {
                    out.push(TouchEvent::Activate { key: k, x: p.last.x, y: p.last.y });
                }
            }
            Mode::Popup => out.push(TouchEvent::PopupCommit),
            Mode::Swipe => {
                self.trace.push(p.last);
                out.push(TouchEvent::SwipeEnd);
            }
            Mode::Cursor { .. } | Mode::Trackpad { .. } => out.push(TouchEvent::CursorEnd),
            Mode::Delete { words } => {
                if words > 0 {
                    out.push(TouchEvent::DeleteCommit { words });
                } else {
                    out.push(TouchEvent::DeleteCancel);
                }
            }
            Mode::ShiftHeld { used } => {
                if used {
                    if let Some(k) = p.key {
                        out.push(TouchEvent::ShiftChordEnd { key: k });
                    }
                }
            }
            Mode::Repeating { .. } | Mode::Done => {}
        }
        if let Some(k) = p.key {
            out.push(TouchEvent::Released { key: k });
        }
        out
    }

    pub fn cancel(&mut self, id: u32) -> Vec<TouchEvent> {
        let mut out = Vec::new();
        if let Some(pi) = self.pointers.iter().position(|p| p.id == id) {
            let p = self.pointers.remove(pi);
            match p.mode {
                Mode::Delete { .. } => out.push(TouchEvent::DeleteCancel),
                Mode::Cursor { .. } | Mode::Trackpad { .. } => out.push(TouchEvent::CursorEnd),
                Mode::Swipe => self.trace.clear(),
                _ => {}
            }
            if let Some(k) = p.key {
                out.push(TouchEvent::Released { key: k });
            }
        }
        out
    }

    pub fn timer(&mut self, now: u32) -> Vec<TouchEvent> {
        let mut out = Vec::new();
        let cfg = self.config.clone();
        for i in 0..self.pointers.len() {
            let Some(dl) = self.pointers[i].deadline else { continue };
            if now.wrapping_sub(dl) > u32::MAX / 2 {
                continue; // not yet due
            }
            let key = self.pointers[i].key;
            let kind = key.map(|k| self.kind(k));
            let p = &mut self.pointers[i];
            p.deadline = None;
            match (&p.mode, kind) {
                (Mode::Tap | Mode::Repeating { .. }, Some(KeyKind::Backspace | KeyKind::Repeat)) => {
                    let count = match p.mode {
                        Mode::Repeating { count } => count + 1,
                        _ => 1,
                    };
                    p.mode = Mode::Repeating { count };
                    if count == 1 {
                        // The press itself counts as the first deletion.
                        out.push(TouchEvent::Activate { key: key.unwrap(), x: p.last.x, y: p.last.y });
                    }
                    out.push(TouchEvent::Repeat { key: key.unwrap(), count });
                    p.deadline = Some(now + cfg.repeat_interval_ms);
                }
                (Mode::Tap, Some(KeyKind::Text { popup: true, .. }))
                | (Mode::Tap, Some(KeyKind::Other { popup: true })) => {
                    p.mode = Mode::Popup;
                    out.push(TouchEvent::LongPress { key: key.unwrap(), x: p.last.x, y: p.last.y });
                }
                (Mode::Tap, Some(KeyKind::Space)) => {
                    if cfg.trackpad_on_space_long_press {
                        p.mode = Mode::Trackpad { sx: 0, sy: 0 };
                        p.anchor = (p.last.x, p.last.y);
                        out.push(TouchEvent::TrackpadStart);
                    } else {
                        out.push(TouchEvent::LongPress { key: key.unwrap(), x: p.last.x, y: p.last.y });
                        p.mode = Mode::Done;
                    }
                }
                (Mode::ShiftHeld { .. }, _) => {
                    // Holding shift past the long-press delay makes it a modifier.
                    p.mode = Mode::ShiftHeld { used: true };
                }
                _ => {}
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{GeometryParams, Rect};
    use crate::layout::{KeyAction, Layer, RowDef, parse_row};

    fn machine(cfg: TouchConfig) -> TouchMachine {
        let layer = Layer {
            rows: [
                "q w e r t y u i o p",
                "a s d f g h j k l .",
                "{shift:1.5} z x c v b n m {backspace:1.5}",
                "{layer=symbols:2} {space:6} {enter:2}",
            ]
            .iter()
            .map(|r| RowDef { keys: parse_row(r).unwrap(), height: 1.0 })
            .collect(),
        };
        let mut p = GeometryParams::phone(400.0, 50.0);
        p.area = Rect::new(0.0, 0.0, 400.0, 0.0);
        p.pad_x = 0.0;
        p.pad_top = 0.0;
        let geom = LayerGeometry::compute(&layer, &p);
        let kinds = layer
            .keys()
            .filter(|k| k.style != crate::layout::KeyStyle::Invisible)
            .map(|k| match &k.action {
                KeyAction::Text(t) => {
                    KeyKind::Text { letter: t.chars().all(|c| c.is_alphabetic()), popup: t == "e" || t == "." }
                }
                KeyAction::Space => KeyKind::Space,
                KeyAction::Backspace => KeyKind::Backspace,
                KeyAction::Shift => KeyKind::Shift,
                _ => KeyKind::Other { popup: false },
            })
            .collect();
        let mut m = TouchMachine::new(cfg);
        m.set_layer(geom, kinds);
        m
    }

    fn center(m: &TouchMachine, row: usize, col: usize) -> (f32, f32) {
        m.geometry().keys[m.geometry().find(row, col).unwrap()].visual.center()
    }

    fn activated(ev: &[TouchEvent]) -> Vec<usize> {
        ev.iter().filter_map(|e| if let TouchEvent::Activate { key, .. } = e { Some(*key) } else { None }).collect()
    }

    #[test]
    fn tap_activates_on_release() {
        let mut m = machine(TouchConfig::default());
        let (x, y) = center(&m, 0, 0);
        let ev = m.down(1, x, y, 0);
        assert!(activated(&ev).is_empty());
        let ev = m.up(1, 60);
        assert_eq!(activated(&ev), vec![0]);
    }

    #[test]
    fn rollover_commits_first_key() {
        let mut m = machine(TouchConfig::default());
        let (x1, y1) = center(&m, 0, 0);
        let (x2, y2) = center(&m, 0, 5);
        m.down(1, x1, y1, 0);
        let ev = m.down(2, x2, y2, 30);
        assert_eq!(activated(&ev), vec![0]);
        let ev = m.up(1, 40);
        assert!(activated(&ev).is_empty());
        let ev = m.up(2, 80);
        assert_eq!(activated(&ev), vec![5]);
    }

    #[test]
    fn swipe_detection() {
        let mut m = machine(TouchConfig::default());
        let (x1, y1) = center(&m, 0, 0);
        let (x2, y2) = center(&m, 0, 4);
        m.down(1, x1, y1, 0);
        let mut started = false;
        for i in 1..=10 {
            let f = i as f32 / 10.0;
            let ev = m.motion(1, x1 + (x2 - x1) * f, y1 + (y2 - y1) * f, i * 10);
            started |= ev.iter().any(|e| matches!(e, TouchEvent::SwipeStart { .. }));
        }
        assert!(started);
        let ev = m.up(1, 120);
        assert!(ev.contains(&TouchEvent::SwipeEnd));
        assert!(activated(&ev).is_empty());
        assert!(m.trace().len() >= 8);
    }

    #[test]
    fn small_jitter_is_a_tap() {
        let mut m = machine(TouchConfig::default());
        let (x, y) = center(&m, 1, 3);
        m.down(1, x, y, 0);
        let ev = m.motion(1, x + 6.0, y + 3.0, 20);
        assert!(!ev.iter().any(|e| matches!(e, TouchEvent::SwipeStart { .. })));
        assert_eq!(activated(&m.up(1, 50)).len(), 1);
    }

    #[test]
    fn long_press_popup() {
        let mut m = machine(TouchConfig::default());
        let (x, y) = center(&m, 0, 2);
        m.down(1, x, y, 0);
        assert_eq!(m.next_deadline(), Some(300));
        let ev = m.timer(300);
        assert!(matches!(ev[0], TouchEvent::LongPress { key: 2, .. }));
        let ev = m.motion(1, x + 20.0, y - 40.0, 350);
        assert!(matches!(ev[0], TouchEvent::PopupMove { .. }));
        let ev = m.up(1, 400);
        assert_eq!(ev[0], TouchEvent::PopupCommit);
    }

    #[test]
    fn space_cursor_scrub() {
        let mut m = machine(TouchConfig::default());
        let (x, y) = center(&m, 3, 1);
        m.down(1, x, y, 0);
        let ev = m.motion(1, x + 10.0, y, 10);
        assert!(ev.is_empty());
        let ev = m.motion(1, x + 20.0, y, 20);
        assert_eq!(ev, vec![TouchEvent::CursorStart, TouchEvent::CursorMove { dx: 1, dy: 0 }]);
        let ev = m.motion(1, x + 20.0 + 2.0 * 11.0 + 1.0, y, 30);
        assert_eq!(ev, vec![TouchEvent::CursorMove { dx: 2, dy: 0 }]);
        let ev = m.motion(1, x - 30.0, y, 40);
        assert_eq!(ev, vec![TouchEvent::CursorMove { dx: -5, dy: 0 }]);
        let ev = m.up(1, 50);
        assert!(ev.contains(&TouchEvent::CursorEnd));
        assert!(activated(&ev).is_empty());
    }

    #[test]
    fn backspace_glide_delete() {
        let mut m = machine(TouchConfig::default());
        let (x, y) = center(&m, 2, 8);
        m.down(1, x, y, 0);
        let ev = m.motion(1, x - 20.0, y, 10);
        assert_eq!(ev, vec![TouchEvent::DeletePreview { words: 1 }]);
        let ev = m.motion(1, x - 20.0 - 36.0 - 1.0, y, 20);
        assert_eq!(ev, vec![TouchEvent::DeletePreview { words: 2 }]);
        let ev = m.up(1, 30);
        assert!(ev.contains(&TouchEvent::DeleteCommit { words: 2 }));
        assert!(activated(&ev).is_empty());
    }

    #[test]
    fn repeat_keys_activate_on_tap_and_repeat_when_held() {
        let mut m = machine(TouchConfig::default());
        let (x, y) = center(&m, 0, 0);
        let k = m.geometry().find(0, 0).unwrap();
        m.kinds[k] = KeyKind::Repeat;
        m.down(1, x, y, 0);
        assert_eq!(activated(&m.up(1, 50)), vec![k]);
        m.down(1, x, y, 100);
        let ev = m.timer(500);
        assert_eq!(activated(&ev), vec![k]);
        assert!(m.timer(550).contains(&TouchEvent::Repeat { key: k, count: 2 }));
        assert!(activated(&m.up(1, 560)).is_empty());
    }

    #[test]
    fn backspace_repeat() {
        let mut m = machine(TouchConfig::default());
        let (x, y) = center(&m, 2, 8);
        let bk = m.geometry().find(2, 8).unwrap();
        m.down(1, x, y, 0);
        let ev = m.timer(400);
        assert_eq!(activated(&ev), vec![bk]);
        assert!(ev.contains(&TouchEvent::Repeat { key: bk, count: 1 }));
        let ev = m.timer(450);
        assert!(ev.contains(&TouchEvent::Repeat { key: bk, count: 2 }));
        let ev = m.up(1, 460);
        assert!(activated(&ev).is_empty());
    }

    #[test]
    fn shift_double_tap_and_chord() {
        let mut m = machine(TouchConfig::default());
        let (sx, sy) = center(&m, 2, 0);
        let sk = m.geometry().find(2, 0).unwrap();
        let ev = m.down(1, sx, sy, 0);
        assert_eq!(activated(&ev), vec![sk]);
        m.up(1, 50);
        let ev = m.down(1, sx, sy, 200);
        assert!(ev.contains(&TouchEvent::CapsLock { key: sk }));
        m.up(1, 250);
        // Chord: hold shift, type a letter, release shift.
        let (ax, ay) = center(&m, 1, 0);
        m.down(1, sx, sy, 2000);
        m.down(2, ax, ay, 2050);
        let ev = m.up(2, 2100);
        assert_eq!(activated(&ev).len(), 1);
        let ev = m.up(1, 2150);
        assert!(ev.contains(&TouchEvent::ShiftChordEnd { key: sk }));
    }

    #[test]
    fn slide_to_adjust_on_non_letters() {
        let mut cfg = TouchConfig::default();
        cfg.swipe_enabled = false;
        let mut m = machine(cfg);
        let (x1, y1) = center(&m, 0, 0);
        let (x2, y2) = center(&m, 0, 1);
        m.down(1, x1, y1, 0);
        m.motion(1, x2, y2, 30);
        assert_eq!(activated(&m.up(1, 60)), vec![1]);
    }
}
