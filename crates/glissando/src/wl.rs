//! Wayland side: globals, the layer-shell surface, input-method-v2,
//! virtual-keyboard-v1, touch/pointer input, shm rendering and clipboard
//! monitoring. Everything runs on the glib main loop.

use crate::clipboard;
use glissando_config::{Config, ThemeMode};
use crate::dbus::{self, Haptics, OskService};
use crate::keyboard::{Action, ChordKey, Haptic, Keyboard, Mods};
use crate::render::Renderer;
use crate::theme::Theme;
use crate::vk::{Keymap, VkKey, MOD_ALT, MOD_CTRL};
use glissando_lang::engine::{FieldInfo, Key, Op, Purpose};
use std::cell::RefCell;
use std::io::Read;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool,
    wl_surface, wl_touch,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols::wp::fractional_scale::v1::client::{wp_fractional_scale_manager_v1, wp_fractional_scale_v1};
use wayland_protocols::wp::text_input::zv3::client::zwp_text_input_v3::{ContentHint, ContentPurpose};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols_misc::zwp_input_method_v2::client::{zwp_input_method_manager_v2, zwp_input_method_v2};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{zwp_virtual_keyboard_manager_v1, zwp_virtual_keyboard_v1};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1, zwlr_data_control_manager_v1, zwlr_data_control_offer_v1,
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{zwlr_foreign_toplevel_handle_v1, zwlr_foreign_toplevel_manager_v1};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

fn debug() -> bool {
    std::env::var_os("GLISSANDO_DEBUG").is_some()
}

macro_rules! dlog {
    ($($t:tt)*) => { if debug() { eprintln!("glissando: {}", format!($($t)*)); } };
}

struct Buffer {
    wl: wl_buffer::WlBuffer,
    map: memmap2::MmapMut,
    w: i32,
    h: i32,
    busy: Arc<AtomicBool>,
}

struct ImState {
    active: bool,
    surrounding: Option<(String, u32, u32)>,
    hint: ContentHint,
    purpose: Option<ContentPurpose>,
}

impl Default for ImState {
    fn default() -> Self {
        ImState { active: false, surrounding: None, hint: ContentHint::empty(), purpose: None }
    }
}

/// A window as seen through wlr-foreign-toplevel (pending values until `done`).
struct Toplevel {
    handle: zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1,
    app_id: String,
    activated: bool,
    pending_app_id: Option<String>,
    pending_activated: Option<bool>,
}

struct SurfaceState {
    surface: wl_surface::WlSurface,
    layer: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    viewport: Option<wp_viewport::WpViewport>,
    _fractional: Option<wp_fractional_scale_v1::WpFractionalScaleV1>,
    configured: bool,
    width: u32,
    height: u32,
    /// Fractional scale in 1/120 units, 0 when unknown.
    scale120: u32,
    int_scale: i32,
    frame_pending: bool,
    buffers: Vec<Buffer>,
}

pub struct App {
    conn: Connection,
    qh: QueueHandle<App>,
    me: Weak<RefCell<App>>,
    compositor: wl_compositor::WlCompositor,
    shm: wl_shm::WlShm,
    layer_shell: zwlr_layer_shell_v1::ZwlrLayerShellV1,
    viewporter: Option<wp_viewporter::WpViewporter>,
    fractional: Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>,
    _seat: Option<wl_seat::WlSeat>,
    touch: Option<wl_touch::WlTouch>,
    pointer: Option<wl_pointer::WlPointer>,
    pointer_pos: (f32, f32),
    pointer_down: bool,
    touch_pos: Vec<(u32, f32, f32)>,
    im: Option<zwp_input_method_v2::ZwpInputMethodV2>,
    im_pending: ImState,
    im_current: ImState,
    im_serial: u32,
    last_preedit: String,
    vk: Option<zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1>,
    keymap: Keymap,
    keymap_sent: u64,
    data_device: Option<zwlr_data_control_device_v1::ZwlrDataControlDeviceV1>,
    _data_control: Option<zwlr_data_control_manager_v1::ZwlrDataControlManagerV1>,
    outputs: Vec<(wl_output::WlOutput, i32)>,
    _toplevel_manager: Option<zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1>,
    toplevels: Vec<Toplevel>,
    /// app_id of the focused window, when the compositor tells us.
    active_app: Option<String>,
    surface: Option<SurfaceState>,
    pub kb: Keyboard,
    renderer: Renderer,
    theme: Theme,
    start: Instant,
    dirty: bool,
    timer: Option<glib::SourceId>,
    anim: Option<glib::SourceId>,
    prewarm: Option<glib::SourceId>,
    user_hidden: bool,
    forced_visible: bool,
    osk: Option<Rc<OskService>>,
    haptics: Haptics,
    auto_show: bool,
    dump_geometry: Option<std::path::PathBuf>,
    last_geometry: String,
    save_timer: Option<glib::SourceId>,
    pub quit: bool,
    frames: u64,
    voice: Option<(std::process::Child, std::process::ChildStdin)>,
}

/// Path of a helper binary installed next to the keyboard (or in PATH).
fn helper(name: &str) -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(name)))
        .filter(|p| p.exists())
        .unwrap_or_else(|| name.into())
}

pub fn voice_available() -> bool {
    std::process::Command::new(helper("glissando-voice"))
        .arg("--check")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub struct Options {
    pub replace: bool,
    pub show: bool,
    pub dump_geometry: Option<std::path::PathBuf>,
    pub no_dbus: bool,
}

impl App {
    pub fn now(&self) -> u32 {
        self.start.elapsed().as_millis() as u32
    }

    fn theme_for(config: &Config) -> Theme {
        let dark = match config.theme {
            ThemeMode::Dark => true,
            ThemeMode::Light => false,
            ThemeMode::System => dbus::prefers_dark(),
        };
        let t = if dark { Theme::dark() } else { Theme::light() };
        if config.key_borders { t.with_borders() } else { t }
    }

    pub fn apply_config(&mut self, config: Config, layouts: Vec<glissando_core::layout::Layout>) {
        self.theme = Self::theme_for(&config);
        self.renderer.set_font(&config.font);
        self.haptics.enabled = config.haptics;
        self.haptics.strength = config.haptic_strength as f64;
        self.haptics.ms = config.haptic_ms;
        clipboard::HISTORY.with(|h| {
            h.borrow_mut().configure(config.clipboard_history, config.clipboard_hours, Some(glissando_config::state_dir().join("pinned-clips")))
        });
        self.kb.engine.settings = crate::engine_settings(&config);
        self.kb.set_config(config, layouts);
        self.after_input();
    }

    // ---- visibility ----------------------------------------------------------

    fn want_visible(&self) -> bool {
        self.forced_visible || (self.im_current.active && self.auto_show && !self.user_hidden && !self.app_blocked())
    }

    /// The focused app is on the user's list of apps where the keyboard
    /// opens only on request (e.g. a calculator with its own keypad).
    fn app_blocked(&self) -> bool {
        self.active_app.as_deref().map(|a| self.kb.config.auto_show_blocked(a)).unwrap_or(false)
    }

    fn toplevels_changed(&mut self) {
        let active = self.toplevels.iter().find(|t| t.activated).map(|t| t.app_id.clone());
        if active != self.active_app {
            dlog!("active app {active:?}");
            self.active_app = active;
            self.update_visibility();
            self.flush();
        }
    }

    fn update_visibility(&mut self) {
        let want = self.want_visible();
        if want && self.surface.is_none() {
            self.create_surface();
        } else if !want && self.surface.is_some() {
            self.destroy_surface();
        }
        if let Some(osk) = &self.osk {
            osk.set_visible(want);
        }
    }

    pub fn set_visible_request(&mut self, v: bool) {
        dlog!("SetVisible({v})");
        if v {
            self.forced_visible = true;
            self.user_hidden = false;
            if !self.im_current.active {
                self.kb.set_fallback_mode();
            }
        } else {
            self.forced_visible = false;
            self.user_hidden = true;
        }
        self.update_visibility();
        self.flush();
    }

    fn create_surface(&mut self) {
        let surface = self.compositor.create_surface(&self.qh, ());
        let layer = self.layer_shell.get_layer_surface(
            &surface,
            self.outputs.first().map(|o| &o.0),
            zwlr_layer_shell_v1::Layer::Top,
            "osk".into(),
            &self.qh,
            (),
        );
        use zwlr_layer_surface_v1::Anchor;
        layer.set_anchor(Anchor::Bottom | Anchor::Left | Anchor::Right);
        let h = self.kb.desired_height() as u32;
        layer.set_size(0, h);
        layer.set_exclusive_zone(h as i32);
        layer.set_keyboard_interactivity(zwlr_layer_surface_v1::KeyboardInteractivity::None);
        let viewport = self.viewporter.as_ref().map(|v| v.get_viewport(&surface, &self.qh, ()));
        let fractional = self.fractional.as_ref().map(|f| f.get_fractional_scale(&surface, &self.qh, ()));
        surface.commit();
        self.surface = Some(SurfaceState {
            surface,
            layer,
            viewport,
            _fractional: fractional,
            configured: false,
            width: 0,
            height: h,
            scale120: 0,
            int_scale: self.outputs.iter().map(|o| o.1).max().unwrap_or(1),
            frame_pending: false,
            buffers: Vec::new(),
        });
        dlog!("surface created, height {h}");
    }

    fn destroy_surface(&mut self) {
        if let Some(s) = self.surface.take() {
            for b in s.buffers {
                b.wl.destroy();
            }
            if let Some(v) = s.viewport {
                v.destroy();
            }
            s.layer.destroy();
            s.surface.destroy();
        }
        self.kb.cancel_touches();
        dlog!("surface destroyed");
    }

    fn update_height(&mut self) {
        let h = self.kb.desired_height() as u32;
        if let Some(s) = &mut self.surface {
            if s.height != h {
                s.layer.set_size(0, h);
                s.layer.set_exclusive_zone(h as i32);
                s.surface.commit();
                s.height = h;
            }
        }
        self.write_geometry();
    }

    fn write_geometry(&mut self) {
        if let Some(p) = &self.dump_geometry {
            let dump = self.kb.geometry_dump();
            if dump != self.last_geometry {
                let tmp = p.with_extension("tmp");
                if std::fs::write(&tmp, &dump).is_ok() {
                    let _ = std::fs::rename(&tmp, p);
                }
                self.last_geometry = dump;
            }
        }
    }

    // ---- rendering -----------------------------------------------------------

    pub fn request_redraw(&mut self) {
        self.dirty = true;
        let can = self.surface.as_ref().map(|s| s.configured && !s.frame_pending).unwrap_or(false);
        if can {
            self.draw();
        }
    }

    fn draw(&mut self) {
        let now = self.now();
        let view = self.kb.view(now);
        let Some(s) = &mut self.surface else { return };
        if !s.configured || s.width == 0 {
            return;
        }
        let (bw, bh, scale) = if s.scale120 > 0 && s.viewport.is_some() {
            let f = s.scale120 as f64 / 120.0;
            ((s.width as f64 * f).round() as i32, (s.height as f64 * f).round() as i32, f)
        } else {
            let f = s.int_scale.max(1);
            (s.width as i32 * f, s.height as i32 * f, f as f64)
        };
        // Drop buffers of the wrong size that the compositor no longer uses.
        s.buffers.retain(|b| {
            let keep = (b.w == bw && b.h == bh) || b.busy.load(Ordering::Acquire);
            if !keep {
                b.wl.destroy();
            }
            keep
        });
        let idx = match s.buffers.iter().position(|b| b.w == bw && b.h == bh && !b.busy.load(Ordering::Acquire)) {
            Some(i) => i,
            None => {
                if s.buffers.len() >= 4 {
                    // Everything is busy; try again on the next frame.
                    return;
                }
                let stride = bw * 4;
                let size = (stride * bh) as usize;
                let fd = match rustix::fs::memfd_create("glissando-buffer", rustix::fs::MemfdFlags::CLOEXEC) {
                    Ok(fd) => fd,
                    Err(e) => {
                        eprintln!("glissando: memfd_create: {e}");
                        return;
                    }
                };
                let file = std::fs::File::from(fd);
                if file.set_len(size as u64).is_err() {
                    return;
                }
                let map = match unsafe { memmap2::MmapMut::map_mut(&file) } {
                    Ok(m) => m,
                    Err(_) => return,
                };
                let pool = self.shm.create_pool(file.as_fd(), size as i32, &self.qh, ());
                let busy = Arc::new(AtomicBool::new(false));
                let wl = pool.create_buffer(0, bw, bh, stride, wl_shm::Format::Argb8888, &self.qh, busy.clone());
                pool.destroy();
                s.buffers.push(Buffer { wl, map, w: bw, h: bh, busy });
                s.buffers.len() - 1
            }
        };
        let buf = &mut s.buffers[idx];
        {
            let stride = buf.w * 4;
            // SAFETY: the mapping outlives the cairo surface, which is dropped
            // before the buffer is handed to the compositor.
            let surf = unsafe {
                cairo::ImageSurface::create_for_data_unsafe(buf.map.as_mut_ptr(), cairo::Format::ARgb32, buf.w, buf.h, stride)
            };
            if let Ok(surf) = surf {
                let t0 = Instant::now();
                self.renderer.render(&surf, scale, &view, &self.theme);
                surf.finish();
                let ms = t0.elapsed().as_secs_f64() * 1000.0;
                self.frames += 1;
                if debug() && (self.frames <= 3 || ms > 12.0) {
                    eprintln!("glissando: frame {} {bw}x{bh} scale {scale:.2}: {ms:.1} ms", self.frames);
                }
            }
        }
        buf.busy.store(true, Ordering::Release);
        if let Some(vp) = &s.viewport {
            if s.scale120 > 0 {
                vp.set_destination(s.width as i32, s.height as i32);
                s.surface.set_buffer_scale(1);
            } else {
                vp.set_destination(-1, -1);
                s.surface.set_buffer_scale(s.int_scale.max(1));
            }
        } else {
            s.surface.set_buffer_scale(s.int_scale.max(1));
        }
        s.surface.attach(Some(&buf.wl), 0, 0);
        s.surface.damage_buffer(0, 0, bw, bh);
        s.surface.frame(&self.qh, ());
        s.frame_pending = true;
        s.surface.commit();
        self.dirty = false;
        if self.kb.animating(now) {
            self.dirty = true;
        }
        self.schedule_prewarm(bw, bh, scale);
    }

    /// While idle, render the key layer for the other shift state so that
    /// auto-capitalisation and shift taps never wait for a full redraw.
    fn schedule_prewarm(&mut self, bw: i32, bh: i32, scale: f64) {
        if self.prewarm.is_some() {
            return;
        }
        let me = self.me.clone();
        self.prewarm = Some(glib::idle_add_local_full(glib::Priority::LOW, move || {
            if let Some(app) = me.upgrade() {
                if let Ok(mut app) = app.try_borrow_mut() {
                    app.prewarm = None;
                    if app.kb_busy() {
                        return glib::ControlFlow::Break;
                    }
                    let now = app.now();
                    if let Some(v) = app.kb.alternate_view(now) {
                        let theme = app.theme.clone();
                        app.renderer.prewarm(bw, bh, scale, &v, &theme);
                    }
                }
            }
            glib::ControlFlow::Break
        }));
    }

    fn kb_busy(&self) -> bool {
        self.dirty || self.kb.animating(self.now()) || self.kb.next_deadline().is_some()
    }

    fn frame_done(&mut self) {
        if let Some(s) = &mut self.surface {
            s.frame_pending = false;
        }
        if self.dirty {
            self.draw();
        }
        self.flush();
    }

    // ---- input method --------------------------------------------------------

    fn field_info(&self) -> FieldInfo {
        let h = self.im_current.hint;
        let purpose = match self.im_current.purpose {
            Some(ContentPurpose::Alpha) => Purpose::Alpha,
            Some(ContentPurpose::Digits) => Purpose::Digits,
            Some(ContentPurpose::Number) => Purpose::Number,
            Some(ContentPurpose::Phone) => Purpose::Phone,
            Some(ContentPurpose::Url) => Purpose::Url,
            Some(ContentPurpose::Email) => Purpose::Email,
            Some(ContentPurpose::Name) => Purpose::Name,
            Some(ContentPurpose::Password) => Purpose::Password,
            Some(ContentPurpose::Pin) => Purpose::Pin,
            Some(ContentPurpose::Date) => Purpose::Date,
            Some(ContentPurpose::Time) => Purpose::Time,
            Some(ContentPurpose::Datetime) => Purpose::Datetime,
            Some(ContentPurpose::Terminal) => Purpose::Terminal,
            _ => Purpose::Normal,
        };
        FieldInfo {
            purpose,
            sensitive: h.contains(ContentHint::SensitiveData) || h.contains(ContentHint::HiddenText),
            multiline: h.contains(ContentHint::Multiline),
            lowercase: h.contains(ContentHint::Lowercase),
            uppercase: h.contains(ContentHint::Uppercase),
            titlecase: h.contains(ContentHint::Titlecase),
            no_completion: false,
        }
    }

    fn im_done(&mut self) {
        self.im_serial = self.im_serial.wrapping_add(1);
        dlog!("im done -> serial {}", self.im_serial);
        let was_active = self.im_current.active;
        let p = std::mem::take(&mut self.im_pending);
        let activated = p.active && !was_active;
        let content_changed = p.hint != self.im_current.hint || p.purpose != self.im_current.purpose;
        self.im_current.active = p.active;
        self.im_current.hint = p.hint;
        self.im_current.purpose = p.purpose;
        // Surrounding text and content type persist until the next activate.
        self.im_pending.active = p.active;
        self.im_pending.hint = p.hint;
        self.im_pending.purpose = p.purpose;
        if activated {
            dlog!("activate");
            self.user_hidden = false;
            self.last_preedit.clear();
            // A field that sends no surrounding text must not inherit the
            // previous field's (deletes would go by stale byte counts).
            self.im_current.surrounding = None;
            self.kb.activate(self.field_info());
        } else if !p.active && was_active {
            dlog!("deactivate");
            self.kb.deactivate();
            self.forced_visible = false;
        } else if p.active && content_changed {
            self.kb.activate(self.field_info());
        }
        if let Some((text, cursor, anchor)) = p.surrounding {
            let c = (cursor as usize).min(text.len());
            if text.is_char_boundary(c) {
                self.kb.surrounding(&text[..c], &text[c..], anchor != cursor);
            }
            self.im_current.surrounding = Some((text, cursor, anchor));
        }
        self.update_visibility();
        self.after_input();
    }

    fn send_vk_key(&mut self, key: VkKey, mods: u32) {
        let code = self.keymap.code(key);
        self.send_vk_code(code, mods);
    }

    fn send_chord(&mut self, key: ChordKey, mods: Mods) {
        let mask = if mods.ctrl { MOD_CTRL } else { 0 } | if mods.alt { MOD_ALT } else { 0 };
        let code = match key {
            ChordKey::Char(c) => self.keymap.char_code(c).0,
            ChordKey::Nav(k) => self.keymap.code(k.into()),
            ChordKey::Backspace => self.keymap.code(VkKey::BackSpace),
            ChordKey::Enter => self.keymap.code(VkKey::Return),
        };
        self.send_vk_code(code, mask);
    }

    fn send_vk_code(&mut self, code: u32, mods: u32) {
        if self.vk.is_none() {
            return;
        }
        self.ensure_keymap();
        let Some(vk) = self.vk.as_ref() else { return };
        let t = self.now();
        if mods != 0 {
            vk.modifiers(mods, 0, 0, 0);
        }
        vk.key(t, code, 1);
        vk.key(t, code, 0);
        if mods != 0 {
            vk.modifiers(0, 0, 0, 0);
        }
    }

    fn ensure_keymap(&mut self) {
        if self.keymap_sent == self.keymap.generation + 1 {
            return;
        }
        let Some(vk) = &self.vk else { return };
        let text = self.keymap.text();
        let mut bytes = text.into_bytes();
        bytes.push(0);
        if let Ok(fd) = rustix::fs::memfd_create("glissando-keymap", rustix::fs::MemfdFlags::CLOEXEC) {
            let mut f = std::fs::File::from(fd);
            use std::io::Write;
            if f.write_all(&bytes).is_ok() {
                vk.keymap(1, f.as_fd(), bytes.len() as u32);
                self.keymap_sent = self.keymap.generation + 1;
            }
        }
    }

    fn type_via_vk(&mut self, text: &str) {
        for c in text.chars() {
            match c {
                '\n' => self.send_vk_key(VkKey::Return, 0),
                '\t' => self.send_vk_key(VkKey::Tab, 0),
                c => {
                    let (code, changed) = self.keymap.char_code(c);
                    if changed {
                        self.ensure_keymap();
                    }
                    if let Some(vk) = &self.vk {
                        let t = self.now();
                        vk.key(t, code, 1);
                        vk.key(t, code, 0);
                    }
                }
            }
        }
    }

    fn apply_ops(&mut self, ops: Vec<Op>) {
        let use_im = self.im_current.active && self.im.is_some();
        if !use_im {
            for op in ops {
                match op {
                    Op::Commit(s) => self.type_via_vk(&s),
                    Op::DeleteBefore { chars, .. } => {
                        for _ in 0..chars {
                            self.send_vk_key(VkKey::BackSpace, 0);
                        }
                    }
                    Op::Preedit(_) => {}
                    Op::Key(Key::Backspace) => self.send_vk_key(VkKey::BackSpace, 0),
                    Op::Key(Key::Enter) => self.send_vk_key(VkKey::Return, 0),
                }
            }
            return;
        }
        let mut commit = String::new();
        let mut delete: Option<usize> = None;
        let mut preedit: Option<String> = None;
        let flush = |app: &mut App, commit: &mut String, delete: &mut Option<usize>, preedit: &mut Option<String>| {
            let Some(im) = &app.im else { return };
            if commit.is_empty() && delete.is_none() && preedit.as_ref().map(|p| *p == app.last_preedit).unwrap_or(true) {
                *preedit = None;
                return;
            }
            if let Some(d) = delete.take() {
                im.delete_surrounding_text(d as u32, 0);
            }
            if !commit.is_empty() {
                im.commit_string(std::mem::take(commit));
            }
            let p = preedit.take().unwrap_or_default();
            if !p.is_empty() {
                let len = p.len() as i32;
                im.set_preedit_string(p.clone(), len, len);
            }
            dlog!("im commit serial {} (preedit {} bytes)", app.im_serial, p.len());
            app.last_preedit = p;
            im.commit(app.im_serial);
        };
        for op in ops {
            match op {
                Op::Commit(s) => {
                    if preedit.is_some() {
                        flush(self, &mut commit, &mut delete, &mut preedit);
                    }
                    commit.push_str(&s);
                }
                Op::DeleteBefore { chars, bytes } => {
                    if !commit.is_empty() || delete.is_some() || preedit.is_some() {
                        flush(self, &mut commit, &mut delete, &mut preedit);
                    }
                    if self.im_current.surrounding.is_some() {
                        delete = Some(bytes);
                    } else {
                        for _ in 0..chars {
                            self.send_vk_key(VkKey::BackSpace, 0);
                        }
                    }
                }
                Op::Preedit(p) => preedit = Some(p),
                Op::Key(k) => {
                    // Clear any visible preedit before raw key events.
                    if !self.last_preedit.is_empty() && preedit.is_none() {
                        preedit = Some(String::new());
                    }
                    flush(self, &mut commit, &mut delete, &mut preedit);
                    match k {
                        Key::Backspace => self.send_vk_key(VkKey::BackSpace, 0),
                        Key::Enter => self.send_vk_key(VkKey::Return, 0),
                    }
                }
            }
        }
        flush(self, &mut commit, &mut delete, &mut preedit);
    }

    /// Process keyboard actions and schedule timers/redraws after any input.
    pub fn after_input(&mut self) {
        for a in self.kb.take_actions() {
            match a {
                Action::Ops(ops) => self.apply_ops(ops),
                Action::Nav(k) => {
                    dlog!("nav {k:?}");
                    self.send_vk_key(k.into(), 0)
                }
                Action::Chord(k, m) => {
                    dlog!("chord {k:?} {m:?}");
                    self.send_chord(k, m)
                }
                Action::Haptic(h) => self.haptics.vibrate(match h {
                    Haptic::Key => 1.0,
                    Haptic::LongPress => 1.4,
                    Haptic::Tick => 0.5,
                }),
                Action::Hide => {
                    self.user_hidden = true;
                    self.forced_visible = false;
                    self.update_visibility();
                }
                Action::OpenSettings => {
                    let exe = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("glissando-settings")));
                    let cmd = exe.filter(|p| p.exists()).unwrap_or_else(|| "glissando-settings".into());
                    if let Err(e) = std::process::Command::new(cmd).env_remove("DESKTOP_AUTOSTART_ID").spawn() {
                        eprintln!("glissando: cannot start settings: {e}");
                    }
                    self.user_hidden = true;
                    self.update_visibility();
                }
                Action::Resize => self.update_height(),
                Action::SaveConfig => {
                    if let Err(e) = self.kb.config.save() {
                        eprintln!("glissando: saving config failed: {e}");
                    }
                    self.theme = Self::theme_for(&self.kb.config);
                    self.update_height();
                }
                Action::Paste(text) => {
                    let ops = vec![Op::Commit(text)];
                    self.apply_ops(ops);
                }
                Action::VoiceStart(lang) => self.voice_start(&lang),
                Action::VoiceStop => self.voice_send("stop"),
                Action::VoiceCancel => {
                    self.voice_send("cancel");
                    if let Some((mut c, _)) = self.voice.take() {
                        let _ = c.kill();
                        let _ = c.wait();
                    }
                }
            }
        }
        // Text reaches the application before we spend time drawing; a late
        // commit can carry a stale serial and be dropped by the compositor.
        self.flush();
        self.write_geometry();
        self.schedule_timer();
        self.schedule_save();
        self.request_redraw();
        if self.kb.animating(self.now()) {
            self.schedule_animation();
        }
        self.flush();
    }

    fn voice_send(&mut self, msg: &str) {
        if let Some((_, stdin)) = &mut self.voice {
            use std::io::Write;
            let _ = writeln!(stdin, "{msg}");
        }
    }

    fn voice_start(&mut self, lang: &str) {
        if let Some((mut c, _)) = self.voice.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        let child = std::process::Command::new(helper("glissando-voice"))
            .env_remove("DESKTOP_AUTOSTART_ID")
            .args(["--lang", lang])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                self.kb.voice_event(&format!("error {e}"));
                return;
            }
        };
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let fd = stdout.as_raw_fd();
        let _ = rustix::fs::fcntl_setfl(&stdout, rustix::fs::OFlags::NONBLOCK);
        let mut file = std::fs::File::from(OwnedFd::from(stdout));
        let mut buf = String::new();
        let me = self.me.clone();
        glib_unix::unix_fd_add_local(fd, glib::IOCondition::IN | glib::IOCondition::HUP, move |_, _| {
            let mut chunk = [0u8; 4096];
            let mut eof = false;
            loop {
                match file.read(&mut chunk) {
                    Ok(0) => {
                        eof = true;
                        break;
                    }
                    Ok(n) => buf.push_str(&String::from_utf8_lossy(&chunk[..n])),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(_) => {
                        eof = true;
                        break;
                    }
                }
            }
            let Some(app) = me.upgrade() else { return glib::ControlFlow::Break };
            let Ok(mut app) = app.try_borrow_mut() else { return glib::ControlFlow::Continue };
            while let Some(i) = buf.find('\n') {
                let line: String = buf.drain(..=i).collect();
                app.kb.voice_event(line.trim_end());
            }
            if eof {
                app.kb.voice_event("end");
                if let Some((mut c, _)) = app.voice.take() {
                    let _ = c.wait();
                }
            }
            app.after_input();
            if eof { glib::ControlFlow::Break } else { glib::ControlFlow::Continue }
        });
        self.voice = Some((child, stdin));
    }

    fn schedule_animation(&mut self) {
        if self.anim.is_some() {
            return;
        }
        let me = self.me.clone();
        self.anim = Some(glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
            let Some(app) = me.upgrade() else { return glib::ControlFlow::Break };
            let mut app = app.borrow_mut();
            let now = app.now();
            app.kb.timer(now);
            app.request_redraw();
            app.flush();
            if app.kb.animating(now) {
                glib::ControlFlow::Continue
            } else {
                app.anim = None;
                glib::ControlFlow::Break
            }
        }));
    }

    fn schedule_timer(&mut self) {
        if let Some(t) = self.timer.take() {
            t.remove();
        }
        let Some(deadline) = self.kb.next_deadline() else { return };
        let now = self.now();
        let delay = deadline.saturating_sub(now) as u64;
        let me = self.me.clone();
        self.timer = Some(glib::timeout_add_local_once(std::time::Duration::from_millis(delay), move || {
            if let Some(app) = me.upgrade() {
                let mut app = app.borrow_mut();
                app.timer = None;
                let now = app.now();
                app.kb.timer(now);
                app.after_input();
            }
        }));
    }

    fn schedule_save(&mut self) {
        if self.save_timer.is_some() || !self.kb.engine.user_data_dirty() {
            return;
        }
        let me = self.me.clone();
        self.save_timer = Some(glib::timeout_add_local_once(std::time::Duration::from_secs(20), move || {
            if let Some(app) = me.upgrade() {
                let mut app = app.borrow_mut();
                app.save_timer = None;
                app.kb.engine.save_user_data();
            }
        }));
    }

    pub fn flush(&self) {
        if let Err(e) = self.conn.flush() {
            dlog!("flush: {e}");
        }
    }

    pub fn shutdown(&mut self) {
        self.kb.engine.save_user_data();
        self.destroy_surface();
        self.flush();
    }

    fn read_clipboard(&mut self, offer: zwlr_data_control_offer_v1::ZwlrDataControlOfferV1, mimes: Vec<String>) {
        let enabled = clipboard::HISTORY.with(|h| h.borrow().enabled);
        if !enabled {
            offer.destroy();
            return;
        }
        let sensitive = mimes.iter().any(|m| m.contains("passwordManagerHint") || m.contains("x-kde-passwordManagerHint"));
        let mime = ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"]
            .iter()
            .find(|m| mimes.iter().any(|x| x == *m))
            .map(|m| m.to_string());
        let Some(mime) = mime else {
            offer.destroy();
            return;
        };
        if sensitive {
            offer.destroy();
            return;
        }
        let Ok((r, w)) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC) else {
            offer.destroy();
            return;
        };
        offer.receive(mime, w.as_fd());
        drop(w);
        self.flush();
        offer.destroy();
        let _ = rustix::fs::fcntl_setfl(&r, rustix::fs::OFlags::NONBLOCK);
        let mut buf: Vec<u8> = Vec::new();
        let raw = r.as_raw_fd();
        let mut file = std::fs::File::from(r);
        glib_unix::unix_fd_add_local(raw, glib::IOCondition::IN | glib::IOCondition::HUP, move |_, cond| {
            let mut chunk = [0u8; 8192];
            loop {
                match file.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&chunk[..n]);
                        if buf.len() > 64 * 1024 {
                            return glib::ControlFlow::Break;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if cond.contains(glib::IOCondition::HUP) {
                            break;
                        }
                        return glib::ControlFlow::Continue;
                    }
                    Err(_) => return glib::ControlFlow::Break,
                }
            }
            if let Ok(s) = String::from_utf8(std::mem::take(&mut buf)) {
                clipboard::HISTORY.with(|h| h.borrow_mut().add(&s, false));
            }
            glib::ControlFlow::Break
        });
    }
}

/// Connect, bind globals and start dispatching on the glib main loop.
pub fn run(opts: Options, config: Config, kb: Keyboard) -> Result<Rc<RefCell<App>>, String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("cannot connect to Wayland: {e}"))?;
    let (globals, mut queue) = registry_queue_init::<App>(&conn).map_err(|e| e.to_string())?;
    let qh = queue.handle();
    let compositor: wl_compositor::WlCompositor =
        globals.bind(&qh, 4..=6, ()).map_err(|e| format!("wl_compositor: {e}"))?;
    let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).map_err(|e| format!("wl_shm: {e}"))?;
    let layer_shell: zwlr_layer_shell_v1::ZwlrLayerShellV1 =
        globals.bind(&qh, 1..=4, ()).map_err(|_| "compositor lacks wlr-layer-shell".to_string())?;
    let seat: Option<wl_seat::WlSeat> = globals.bind(&qh, 1..=7, ()).ok();
    let im_mgr: Option<zwp_input_method_manager_v2::ZwpInputMethodManagerV2> = globals.bind(&qh, 1..=1, ()).ok();
    let vk_mgr: Option<zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1> = globals.bind(&qh, 1..=1, ()).ok();
    let viewporter: Option<wp_viewporter::WpViewporter> = globals.bind(&qh, 1..=1, ()).ok();
    let fractional: Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1> = globals.bind(&qh, 1..=1, ()).ok();
    let data_control: Option<zwlr_data_control_manager_v1::ZwlrDataControlManagerV1> = globals.bind(&qh, 1..=2, ()).ok();
    let toplevel_manager: Option<zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1> =
        globals.bind(&qh, 1..=3, ()).ok();
    if toplevel_manager.is_none() {
        dlog!("no wlr-foreign-toplevel: apps are not told apart for automatic showing");
    }
    let mut outputs = Vec::new();
    for g in globals.contents().clone_list() {
        if g.interface == wl_output::WlOutput::interface().name {
            let o: wl_output::WlOutput = globals.registry().bind(g.name, g.version.min(4), &qh, ());
            outputs.push((o, 1));
        }
    }
    let Some(seat) = seat else { return Err("no wl_seat".into()) };
    let im = im_mgr.as_ref().map(|m| m.get_input_method(&seat, &qh, ()));
    if im.is_none() {
        eprintln!("glissando: compositor lacks input-method-v2; typing falls back to the virtual keyboard");
    }
    let vk = vk_mgr.as_ref().map(|m| m.create_virtual_keyboard(&seat, &qh, ()));
    let data_device = data_control.as_ref().map(|m| m.get_data_device(&seat, &qh, ()));
    let theme = App::theme_for(&config);
    let mut haptics = Haptics::new();
    haptics.enabled = config.haptics;
    haptics.strength = config.haptic_strength as f64;
    haptics.ms = config.haptic_ms;
    clipboard::HISTORY.with(|h| {
        h.borrow_mut().configure(config.clipboard_history, config.clipboard_hours, Some(glissando_config::state_dir().join("pinned-clips")))
    });
    let app = Rc::new(RefCell::new(App {
        conn: conn.clone(),
        qh: qh.clone(),
        me: Weak::new(),
        compositor,
        shm,
        layer_shell,
        viewporter,
        fractional,
        _seat: Some(seat),
        touch: None,
        pointer: None,
        pointer_pos: (0.0, 0.0),
        pointer_down: false,
        touch_pos: Vec::new(),
        im,
        im_pending: ImState::default(),
        im_current: ImState::default(),
        im_serial: 0,
        last_preedit: String::new(),
        vk,
        keymap: Keymap::default(),
        keymap_sent: 0,
        data_device,
        _data_control: data_control,
        outputs,
        _toplevel_manager: toplevel_manager,
        toplevels: Vec::new(),
        active_app: None,
        surface: None,
        renderer: Renderer::new(&config.font),
        kb,
        theme,
        start: Instant::now(),
        dirty: true,
        timer: None,
        anim: None,
        prewarm: None,
        user_hidden: false,
        forced_visible: opts.show,
        osk: None,
        haptics,
        auto_show: std::env::var_os("GLISSANDO_AUTOSHOW").is_some() || dbus::osk_enabled(),
        dump_geometry: opts.dump_geometry.clone(),
        last_geometry: String::new(),
        save_timer: None,
        quit: false,
        frames: 0,
        voice: None,
    }));
    app.borrow_mut().me = Rc::downgrade(&app);
    queue.roundtrip(&mut app.borrow_mut()).map_err(|e| e.to_string())?;
    if !opts.no_dbus {
        let w1 = Rc::downgrade(&app);
        let w2 = Rc::downgrade(&app);
        let w3 = Rc::downgrade(&app);
        let osk = OskService::start(
            opts.replace,
            move |v| {
                if let Some(a) = w1.upgrade() {
                    if let Ok(mut a) = a.try_borrow_mut() {
                        a.set_visible_request(v);
                    }
                }
            },
            move || {
                eprintln!("glissando: lost sm.puri.OSK0 (replaced by another keyboard or no session bus)");
                if let Some(a) = w2.upgrade() {
                    if let Ok(mut a) = a.try_borrow_mut() {
                        a.quit = true;
                    }
                }
            },
            // gnome-session's Stop goes through the same exit as a signal, so
            // the words learned since the last timed save are kept.
            move || {
                if let Some(a) = w3.upgrade() {
                    if let Ok(mut a) = a.try_borrow_mut() {
                        a.quit = true;
                    }
                }
            },
        );
        app.borrow_mut().osk = Some(osk);
    }
    {
        let mut a = app.borrow_mut();
        if a.forced_visible {
            a.kb.set_fallback_mode();
        }
        a.update_visibility();
        a.flush();
    }
    {
        let w = Rc::downgrade(&app);
        glib::idle_add_local_full(glib::Priority::LOW, move || {
            if let Some(a) = w.upgrade() {
                if let Ok(a) = a.try_borrow() {
                    a.renderer.warm_fonts();
                }
            }
            glib::ControlFlow::Break
        });
    }
    let queue = Rc::new(RefCell::new(queue));
    let fd = conn.backend().poll_fd().as_raw_fd();
    let app2 = app.clone();
    let conn2 = conn.clone();
    glib_unix::unix_fd_add_local(fd, glib::IOCondition::IN | glib::IOCondition::ERR | glib::IOCondition::HUP, move |_, cond| {
        let mut q = queue.borrow_mut();
        if let Some(guard) = q.prepare_read() {
            if let Err(e) = guard.read() {
                if !matches!(&e, wayland_client::backend::WaylandError::Io(io) if io.kind() == std::io::ErrorKind::WouldBlock) {
                    eprintln!("glissando: Wayland connection error: {e}");
                    app2.borrow_mut().quit = true;
                    return glib::ControlFlow::Break;
                }
            }
        }
        let mut a = app2.borrow_mut();
        if let Err(e) = q.dispatch_pending(&mut a) {
            eprintln!("glissando: protocol error: {e}");
            a.quit = true;
            return glib::ControlFlow::Break;
        }
        let _ = conn2.flush();
        if cond.contains(glib::IOCondition::HUP) || cond.contains(glib::IOCondition::ERR) {
            a.quit = true;
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
    Ok(app)
}

// ---- dispatch implementations ------------------------------------------------

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for App {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

macro_rules! ignore_events {
    ($($t:ty),*) => {$(
        impl Dispatch<$t, ()> for App {
            fn event(_: &mut Self, _: &$t, _: <$t as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
        }
    )*};
}

ignore_events!(
    wl_compositor::WlCompositor,
    wl_shm::WlShm,
    wl_shm_pool::WlShmPool,
    zwlr_layer_shell_v1::ZwlrLayerShellV1,
    wp_viewporter::WpViewporter,
    wp_viewport::WpViewport,
    wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
    zwp_input_method_manager_v2::ZwpInputMethodManagerV2,
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
    zwlr_data_control_manager_v1::ZwlrDataControlManagerV1
);

impl Dispatch<wl_buffer::WlBuffer, Arc<AtomicBool>> for App {
    fn event(app: &mut Self, _: &wl_buffer::WlBuffer, e: wl_buffer::Event, busy: &Arc<AtomicBool>, _: &Connection, _: &QueueHandle<Self>) {
        if let wl_buffer::Event::Release = e {
            busy.store(false, Ordering::Release);
            if app.dirty {
                app.request_redraw();
            }
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for App {
    fn event(app: &mut Self, _: &wl_callback::WlCallback, e: wl_callback::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_callback::Event::Done { .. } = e {
            app.frame_done();
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for App {
    fn event(app: &mut Self, o: &wl_output::WlOutput, e: wl_output::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_output::Event::Scale { factor } = e {
            if let Some(entry) = app.outputs.iter_mut().find(|x| &x.0 == o) {
                entry.1 = factor;
            }
        }
    }
}

impl Dispatch<wl_surface::WlSurface, ()> for App {
    fn event(app: &mut Self, _: &wl_surface::WlSurface, e: wl_surface::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        let mut changed = false;
        match e {
            wl_surface::Event::PreferredBufferScale { factor } => {
                if let Some(s) = &mut app.surface {
                    changed = s.int_scale != factor;
                    s.int_scale = factor;
                }
            }
            wl_surface::Event::Enter { output } => {
                let f = app.outputs.iter().find(|x| x.0 == output).map(|x| x.1).unwrap_or(1);
                if let Some(s) = &mut app.surface {
                    if f > s.int_scale {
                        s.int_scale = f;
                        changed = true;
                    }
                }
            }
            _ => {}
        }
        if changed {
            app.request_redraw();
        }
    }
}

impl Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, ()> for App {
    fn event(app: &mut Self, _: &wp_fractional_scale_v1::WpFractionalScaleV1, e: wp_fractional_scale_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = e {
            if let Some(s) = &mut app.surface {
                s.scale120 = scale;
            }
            app.request_redraw();
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for App {
    fn event(app: &mut Self, l: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, e: zwlr_layer_surface_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match e {
            zwlr_layer_surface_v1::Event::Configure { serial, width, height } => {
                l.ack_configure(serial);
                let landscape = width > 540;
                app.kb.resize(width as f32, landscape);
                let want_h = app.kb.desired_height() as u32;
                if let Some(s) = &mut app.surface {
                    s.configured = true;
                    s.width = width;
                    s.height = if height > 0 { height } else { want_h };
                }
                dlog!("configure {width}x{height}");
                app.update_height();
                app.dirty = true;
                if let Some(s) = &mut app.surface {
                    s.frame_pending = false;
                }
                app.request_redraw();
            }
            zwlr_layer_surface_v1::Event::Closed => {
                app.surface = None;
                app.user_hidden = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for App {
    fn event(app: &mut Self, seat: &wl_seat::WlSeat, e: wl_seat::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_seat::Event::Capabilities { capabilities: WEnum::Value(caps) } = e {
            dlog!("seat capabilities {caps:?}");
            if caps.contains(wl_seat::Capability::Touch) {
                if app.touch.is_none() {
                    app.touch = Some(seat.get_touch(qh, ()));
                }
            } else if let Some(t) = app.touch.take() {
                if t.version() >= 3 {
                    t.release();
                }
                app.touch_pos.clear();
                app.kb.cancel_touches();
            }
            if caps.contains(wl_seat::Capability::Pointer) {
                if app.pointer.is_none() {
                    app.pointer = Some(seat.get_pointer(qh, ()));
                }
            } else if let Some(p) = app.pointer.take() {
                if p.version() >= 3 {
                    p.release();
                }
                if app.pointer_down {
                    app.pointer_down = false;
                    app.kb.touch_cancel(POINTER_ID);
                }
            }
        }
    }
}

const POINTER_ID: u32 = 0x7fff_0000;
const BTN_LEFT: u32 = 0x110;

impl Dispatch<wl_touch::WlTouch, ()> for App {
    fn event(app: &mut Self, _: &wl_touch::WlTouch, e: wl_touch::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        let now = app.now();
        match e {
            wl_touch::Event::Down { surface, id, x, y, .. } => {
                if app.surface.as_ref().map(|s| s.surface == surface).unwrap_or(false) {
                    app.touch_pos.retain(|t| t.0 != id as u32);
                    app.touch_pos.push((id as u32, x as f32, y as f32));
                    app.kb.touch_down(id as u32, x as f32, y as f32, now);
                }
            }
            wl_touch::Event::Motion { id, x, y, .. } => {
                if let Some(t) = app.touch_pos.iter_mut().find(|t| t.0 == id as u32) {
                    t.1 = x as f32;
                    t.2 = y as f32;
                }
                app.kb.touch_motion(id as u32, x as f32, y as f32, now)
            }
            wl_touch::Event::Up { id, .. } => {
                let (x, y) = app.touch_pos.iter().find(|t| t.0 == id as u32).map(|t| (t.1, t.2)).unwrap_or((-1.0, -1.0));
                app.touch_pos.retain(|t| t.0 != id as u32);
                app.kb.touch_up(id as u32, x, y, now);
            }
            wl_touch::Event::Cancel => {
                app.touch_pos.clear();
                app.kb.cancel_touches()
            }
            wl_touch::Event::Frame => {}
            _ => return,
        }
        app.after_input();
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for App {
    fn event(app: &mut Self, _: &wl_pointer::WlPointer, e: wl_pointer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        let now = app.now();
        match e {
            wl_pointer::Event::Enter { surface_x, surface_y, .. } => {
                dlog!("pointer enter");
                app.pointer_pos = (surface_x as f32, surface_y as f32);
                return;
            }
            wl_pointer::Event::Leave { .. } => {
                if app.pointer_down {
                    app.pointer_down = false;
                    app.kb.touch_cancel(POINTER_ID);
                }
            }
            wl_pointer::Event::Motion { surface_x, surface_y, .. } => {
                if std::env::var_os("GLISSANDO_DEBUG_INPUT").is_some() {
                    eprintln!("motion down={} {surface_x:.0},{surface_y:.0}", app.pointer_down);
                }
                app.pointer_pos = (surface_x as f32, surface_y as f32);
                if !app.pointer_down {
                    return;
                }
                app.kb.touch_motion(POINTER_ID, surface_x as f32, surface_y as f32, now);
            }
            wl_pointer::Event::Button { button, state: WEnum::Value(st), .. } if button == BTN_LEFT => {
                if std::env::var_os("GLISSANDO_DEBUG_INPUT").is_some() {
                    eprintln!("button {st:?} at {:?}", app.pointer_pos);
                }
                let (x, y) = app.pointer_pos;
                match st {
                    wl_pointer::ButtonState::Pressed => {
                        app.pointer_down = true;
                        app.kb.touch_down(POINTER_ID, x, y, now);
                    }
                    _ => {
                        if app.pointer_down {
                            app.pointer_down = false;
                            app.kb.touch_up(POINTER_ID, x, y, now);
                        }
                    }
                }
            }
            _ => return,
        }
        app.after_input();
    }
}

impl Dispatch<zwp_input_method_v2::ZwpInputMethodV2, ()> for App {
    fn event(app: &mut Self, _: &zwp_input_method_v2::ZwpInputMethodV2, e: zwp_input_method_v2::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        use zwp_input_method_v2::Event;
        match e {
            Event::Activate => {
                // Activation resets the double-buffered text input state.
                app.im_pending = ImState { active: true, ..Default::default() };
            }
            Event::Deactivate => app.im_pending.active = false,
            Event::SurroundingText { text, cursor, anchor } => app.im_pending.surrounding = Some((text, cursor, anchor)),
            // Not reliable across toolkits (Qt never sets it); the engine
            // compares the text instead.
            Event::TextChangeCause { .. } => {}
            Event::ContentType { hint, purpose } => {
                app.im_pending.hint = hint.into_result().unwrap_or(ContentHint::empty());
                app.im_pending.purpose = purpose.into_result().ok();
            }
            Event::Done => app.im_done(),
            Event::Unavailable => {
                eprintln!("glissando: another input method is already running on this seat");
                app.im = None;
            }
            _ => {}
        }
    }
}

impl Dispatch<zwlr_data_control_device_v1::ZwlrDataControlDeviceV1, ()> for App {
    fn event(app: &mut Self, _: &zwlr_data_control_device_v1::ZwlrDataControlDeviceV1, e: zwlr_data_control_device_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match e {
            zwlr_data_control_device_v1::Event::Selection { id: Some(offer) } => {
                let mimes = offer.data::<Mutex<Vec<String>>>().map(|m| m.lock().unwrap().clone()).unwrap_or_default();
                app.read_clipboard(offer, mimes);
            }
            zwlr_data_control_device_v1::Event::PrimarySelection { id: Some(offer) } => offer.destroy(),
            zwlr_data_control_device_v1::Event::Finished => {
                app.data_device = None;
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(App, zwlr_data_control_device_v1::ZwlrDataControlDeviceV1, [
        zwlr_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (zwlr_data_control_offer_v1::ZwlrDataControlOfferV1, Mutex::new(Vec::new())),
    ]);
}

impl Dispatch<zwlr_data_control_offer_v1::ZwlrDataControlOfferV1, Mutex<Vec<String>>> for App {
    fn event(_: &mut Self, _: &zwlr_data_control_offer_v1::ZwlrDataControlOfferV1, e: zwlr_data_control_offer_v1::Event, mimes: &Mutex<Vec<String>>, _: &Connection, _: &QueueHandle<Self>) {
        if let zwlr_data_control_offer_v1::Event::Offer { mime_type } = e {
            mimes.lock().unwrap().push(mime_type);
        }
    }
}

impl Dispatch<zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1, ()> for App {
    fn event(app: &mut Self, _: &zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1, e: zwlr_foreign_toplevel_manager_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match e {
            zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } => app.toplevels.push(Toplevel {
                handle: toplevel,
                app_id: String::new(),
                activated: false,
                pending_app_id: None,
                pending_activated: None,
            }),
            zwlr_foreign_toplevel_manager_v1::Event::Finished => {
                app.toplevels.clear();
                app.toplevels_changed();
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(App, zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1, [
        zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE => (zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1, ()> for App {
    fn event(app: &mut Self, h: &zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1, e: zwlr_foreign_toplevel_handle_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        use zwlr_foreign_toplevel_handle_v1::{Event, State};
        let Some(i) = app.toplevels.iter().position(|t| &t.handle == h) else { return };
        match e {
            Event::AppId { app_id } => app.toplevels[i].pending_app_id = Some(app_id),
            Event::State { state } => {
                let activated = state
                    .chunks_exact(4)
                    .any(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]]) == State::Activated as u32);
                app.toplevels[i].pending_activated = Some(activated);
            }
            Event::Done => {
                let t = &mut app.toplevels[i];
                if let Some(a) = t.pending_app_id.take() {
                    t.app_id = a;
                }
                if let Some(a) = t.pending_activated.take() {
                    t.activated = a;
                }
                app.toplevels_changed();
            }
            Event::Closed => {
                app.toplevels.remove(i).handle.destroy();
                app.toplevels_changed();
            }
            _ => {}
        }
    }
}

#[allow(dead_code)]
fn owned_fd_from_raw(fd: i32) -> OwnedFd {
    // SAFETY: only used with descriptors we just created.
    unsafe { OwnedFd::from_raw_fd(fd) }
}
