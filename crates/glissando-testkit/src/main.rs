//! Test helpers for end-to-end runs in a nested/headless compositor.
//!
//! * `textclient [--purpose P] [--hint H] [--state FILE] [--toolkit qt] [--app-id ID] [--text T]` – a
//!   window with a text-input-v3 field that behaves like a well-behaved
//!   toolkit: it applies preedit/commit/delete events, reports surrounding
//!   text and also accepts virtual keyboard keys. A click commits the
//!   preedit where it is and puts the cursor at the start. With `--toolkit
//!   qt` it never sets the text change cause, like Qt. Keys pressed with
//!   Ctrl/Alt are written into the text as `<C-c>`, `<A-Left>`. With
//!   `--text` the field starts with that text selected the way GTK selects a
//!   focused search field: the selection is not reported and is deleted when
//!   the preedit starts. Its text state is written to FILE as JSON after
//!   every change.
//! * `pointer <script>` – drives a virtual pointer. Script commands (`;`
//!   separated): `tap X Y`, `down X Y`, `move X Y`, `up`, `swipe MS X1 Y1 X2 Y2 ...`,
//!   `sleep MS`. Coordinates are logical, output-global.

use std::collections::HashMap;
use std::os::fd::AsFd;
use std::time::{Duration, Instant};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols::wp::text_input::zv3::client::{zwp_text_input_manager_v3, zwp_text_input_v3};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols_wlr::virtual_pointer::v1::client::{zwlr_virtual_pointer_manager_v1, zwlr_virtual_pointer_v1};

#[derive(Default)]
struct Pending {
    preedit: Option<(String, i32, i32)>,
    commit: Option<String>,
    delete: Option<(u32, u32)>,
}

struct Client {
    ti: Option<zwp_text_input_v3::ZwpTextInputV3>,
    text: String,
    cursor: usize,
    preedit: String,
    pending: Pending,
    serial: u32,
    purpose: zwp_text_input_v3::ContentPurpose,
    hint: zwp_text_input_v3::ContentHint,
    /// Report the text change cause (GTK does, Qt does not).
    send_cause: bool,
    /// The whole text is selected without telling the input method.
    hidden_selection: bool,
    state_file: Option<String>,
    keymap: HashMap<u32, char>,
    /// Depressed modifier mask from the virtual keyboard.
    mods: u32,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    configured: bool,
    surface: Option<wl_surface::WlSurface>,
    shm: Option<wl_shm::WlShm>,
    xdg_surface: Option<xdg_surface::XdgSurface>,
    width: i32,
    height: i32,
    events: u64,
}

impl Client {
    fn write_state(&mut self) {
        self.events += 1;
        let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
        let json = format!(
            "{{\"text\": \"{}\", \"preedit\": \"{}\", \"cursor\": {}, \"events\": {}}}\n",
            esc(&self.text),
            esc(&self.preedit),
            self.text[..self.cursor].chars().count(),
            self.events
        );
        if let Some(f) = &self.state_file {
            let tmp = format!("{f}.tmp");
            let _ = std::fs::write(&tmp, &json);
            let _ = std::fs::rename(&tmp, f);
        } else {
            print!("{json}");
        }
    }

    fn send_state(&self) {
        let Some(ti) = &self.ti else { return };
        let start = self.text[..self.cursor].char_indices().rev().nth(200).map(|(i, _)| i).unwrap_or(0);
        let window = &self.text[start..];
        let c = (self.cursor - start) as i32;
        ti.set_surrounding_text(window.to_string(), c, c);
        ti.set_content_type(self.hint, self.purpose);
        ti.commit();
    }

    fn apply_done(&mut self) {
        let p = std::mem::take(&mut self.pending);
        // 1. Remove preedit. 2. Delete surrounding. 3. Insert commit. 4. New preedit.
        let starts = self.preedit.is_empty() && p.preedit.as_ref().map(|(t, _, _)| !t.is_empty()).unwrap_or(false);
        if std::mem::take(&mut self.hidden_selection) && (starts || p.commit.is_some()) {
            // GTK deletes the selection as the preedit starts.
            self.text.clear();
            self.cursor = 0;
        }
        self.preedit.clear();
        if let Some((before, after)) = p.delete {
            let b = (before as usize).min(self.cursor);
            let a = (after as usize).min(self.text.len() - self.cursor);
            let start = self.cursor - b;
            if self.text.is_char_boundary(start) && self.text.is_char_boundary(self.cursor + a) {
                self.text.replace_range(start..self.cursor + a, "");
                self.cursor = start;
            }
        }
        if let Some(c) = p.commit {
            self.text.insert_str(self.cursor, &c);
            self.cursor += c.len();
        }
        if let Some((pre, _, _)) = p.preedit {
            self.preedit = pre;
        }
        self.write_state();
        self.send_state();
    }

    fn key(&mut self, code: u32) {
        if self.mods & (4 | 8) != 0 {
            let name = match code {
                1 => "Esc".to_string(),
                14 => "BackSpace".into(),
                15 => "Tab".into(),
                28 => "Return".into(),
                103 => "Up".into(),
                105 => "Left".into(),
                106 => "Right".into(),
                108 => "Down".into(),
                other => self.keymap.get(&other).map(|c| c.to_string()).unwrap_or_else(|| format!("#{other}")),
            };
            let tag = format!(
                "<{}{}{name}>",
                if self.mods & 4 != 0 { "C-" } else { "" },
                if self.mods & 8 != 0 { "A-" } else { "" }
            );
            self.text.insert_str(self.cursor, &tag);
            self.cursor += tag.len();
            self.write_state();
            self.external_change();
            return;
        }
        match code {
            1 => {
                self.text.insert_str(self.cursor, "<Esc>");
                self.cursor += 5;
            }
            15 => {
                self.text.insert(self.cursor, '\t');
                self.cursor += 1;
            }
            14 => {
                if self.cursor > 0 {
                    let prev = self.text[..self.cursor].chars().last().unwrap();
                    self.cursor -= prev.len_utf8();
                    self.text.remove(self.cursor);
                }
            }
            28 => {
                self.text.insert(self.cursor, '\n');
                self.cursor += 1;
            }
            105 => {
                if let Some(c) = self.text[..self.cursor].chars().last() {
                    self.cursor -= c.len_utf8();
                }
            }
            106 => {
                if let Some(c) = self.text[self.cursor..].chars().next() {
                    self.cursor += c.len_utf8();
                }
            }
            102 => self.cursor = 0,
            107 => self.cursor = self.text.len(),
            other => {
                if let Some(&c) = self.keymap.get(&other) {
                    self.text.insert(self.cursor, c);
                    self.cursor += c.len_utf8();
                } else {
                    return;
                }
            }
        }
        self.preedit.clear();
        self.write_state();
        self.external_change();
    }

    /// A click in the field: the toolkit commits the preedit in place, then
    /// the cursor moves to the start.
    fn click(&mut self) {
        let p = std::mem::take(&mut self.preedit);
        self.text.insert_str(self.cursor, &p);
        self.cursor = 0;
        self.write_state();
        self.external_change();
    }

    fn external_change(&mut self) {
        // Cursor moves are external changes for the input method.
        if let Some(ti) = &self.ti {
            if self.send_cause {
                ti.set_text_change_cause(zwp_text_input_v3::ChangeCause::Other);
            }
        }
        self.send_state();
    }

    fn draw(&mut self, qh: &QueueHandle<Client>) {
        let (Some(shm), Some(surface)) = (&self.shm, &self.surface) else { return };
        let (w, h) = (self.width.max(1), self.height.max(1));
        let size = (w * h * 4) as usize;
        let fd = rustix::fs::memfd_create("tk", rustix::fs::MemfdFlags::CLOEXEC).unwrap();
        let file = std::fs::File::from(fd);
        file.set_len(size as u64).unwrap();
        let mut map = unsafe { memmap2::MmapMut::map_mut(&file).unwrap() };
        for px in map.chunks_mut(4) {
            px.copy_from_slice(&[0xf0, 0xf0, 0xf0, 0xff]);
        }
        let pool = shm.create_pool(file.as_fd(), size as i32, qh, ());
        let buf = pool.create_buffer(0, w, h, w * 4, wl_shm::Format::Argb8888, qh, ());
        pool.destroy();
        surface.attach(Some(&buf), 0, 0);
        surface.damage_buffer(0, 0, w, h);
        surface.commit();
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Client {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

macro_rules! noop {
    ($s:ty; $($t:ty),*) => {$(
        impl Dispatch<$t, ()> for $s {
            fn event(_: &mut Self, _: &$t, _: <$t as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
        }
    )*};
}

noop!(Client; wl_compositor::WlCompositor, wl_shm::WlShm, wl_shm_pool::WlShmPool, wl_buffer::WlBuffer, wl_surface::WlSurface,
    zwp_text_input_manager_v3::ZwpTextInputManagerV3, xdg_toplevel::XdgToplevel);

impl Dispatch<wl_pointer::WlPointer, ()> for Client {
    fn event(
        c: &mut Self,
        _: &wl_pointer::WlPointer,
        e: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_pointer::Event::Button { state: WEnum::Value(wl_pointer::ButtonState::Pressed), .. } = e {
            c.click();
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for Client {
    fn event(
        _: &mut Self,
        b: &xdg_wm_base::XdgWmBase,
        e: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = e {
            b.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for Client {
    fn event(
        c: &mut Self,
        s: &xdg_surface::XdgSurface,
        e: xdg_surface::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = e {
            s.ack_configure(serial);
            c.configured = true;
            c.draw(qh);
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for Client {
    fn event(cl: &mut Self, seat: &wl_seat::WlSeat, e: wl_seat::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_seat::Event::Capabilities { capabilities: WEnum::Value(c) } = e {
            if c.contains(wl_seat::Capability::Keyboard) && cl.keyboard.is_none() {
                cl.keyboard = Some(seat.get_keyboard(qh, ()));
            }
            if c.contains(wl_seat::Capability::Pointer) && cl.pointer.is_none() {
                cl.pointer = Some(seat.get_pointer(qh, ()));
            }
        }
    }
}

fn parse_keymap(text: &str) -> HashMap<u32, char> {
    // Understands the keymaps glissando uploads as the compositor passes
    // them on: `key <I200> { [ U0436 ] };`, `[ c ]` or `[ 0x63 ]`.
    let mut m = HashMap::new();
    for line in text.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("key <I") {
            let code: Option<u32> = rest.split('>').next().and_then(|n| n.parse().ok());
            let sym = rest.split('[').nth(1).and_then(|s| s.split(']').next()).map(|s| s.trim());
            if let (Some(code), Some(sym)) = (code, sym) {
                if let Some(ch) = keysym_char(sym) {
                    m.insert(code - 8, ch);
                }
            }
        }
    }
    m
}

fn keysym_char(sym: &str) -> Option<char> {
    let mut cs = sym.chars();
    if let (Some(ch), None) = (cs.next(), cs.next()) {
        return Some(ch);
    }
    let cp = if let Some(hex) = sym.strip_prefix("0x") {
        let v = u32::from_str_radix(hex, 16).ok()?;
        if v < 0x100 { v } else { v.checked_sub(0x0100_0000)? }
    } else {
        u32::from_str_radix(sym.strip_prefix('U')?, 16).ok()?
    };
    char::from_u32(cp)
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for Client {
    fn event(
        c: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        e: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match e {
            wl_keyboard::Event::Keymap { fd, size, .. } => {
                // The descriptor may share its file offset with earlier ones.
                use std::os::unix::fs::FileExt;
                let f = std::fs::File::from(fd);
                let mut buf = vec![0u8; size as usize];
                if f.read_exact_at(&mut buf, 0).is_ok() {
                    let s = String::from_utf8_lossy(&buf).to_string();
                    c.keymap = parse_keymap(&s);
                }
            }
            wl_keyboard::Event::Enter { .. } => {
                if let Some(ti) = &c.ti {
                    ti.enable();
                    c.send_state();
                }
            }
            wl_keyboard::Event::Modifiers { mods_depressed, .. } => c.mods = mods_depressed,
            wl_keyboard::Event::Key { key, state: WEnum::Value(wl_keyboard::KeyState::Pressed), .. } => c.key(key),
            _ => {}
        }
    }
}

impl Dispatch<zwp_text_input_v3::ZwpTextInputV3, ()> for Client {
    fn event(
        c: &mut Self,
        _: &zwp_text_input_v3::ZwpTextInputV3,
        e: zwp_text_input_v3::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwp_text_input_v3::Event;
        match e {
            Event::PreeditString { text, cursor_begin, cursor_end } => {
                c.pending.preedit = Some((text.unwrap_or_default(), cursor_begin, cursor_end))
            }
            Event::CommitString { text } => c.pending.commit = Some(text.unwrap_or_default()),
            Event::DeleteSurroundingText { before_length, after_length } => {
                c.pending.delete = Some((before_length, after_length))
            }
            Event::Done { serial } => {
                eprintln!(
                    "tc: done {serial} preedit={:?} commit={:?}",
                    c.pending.preedit.as_ref().map(|p| p.0.len()),
                    c.pending.commit.as_ref().map(|s| s.len())
                );
                c.serial = serial;
                if let Some(ti) = &c.ti {
                    if c.send_cause {
                        ti.set_text_change_cause(zwp_text_input_v3::ChangeCause::InputMethod);
                    }
                }
                c.apply_done();
            }
            _ => {}
        }
    }
}

fn textclient(args: &[String]) {
    let mut purpose = zwp_text_input_v3::ContentPurpose::Normal;
    let mut hint = zwp_text_input_v3::ContentHint::empty();
    let mut state_file = None;
    let mut send_cause = true;
    let mut app_id = "glissando-textclient".to_string();
    let mut text = String::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--purpose" => {
                i += 1;
                purpose = match args[i].as_str() {
                    "password" => zwp_text_input_v3::ContentPurpose::Password,
                    "email" => zwp_text_input_v3::ContentPurpose::Email,
                    "url" => zwp_text_input_v3::ContentPurpose::Url,
                    "number" => zwp_text_input_v3::ContentPurpose::Number,
                    "digits" => zwp_text_input_v3::ContentPurpose::Digits,
                    "phone" => zwp_text_input_v3::ContentPurpose::Phone,
                    "terminal" => zwp_text_input_v3::ContentPurpose::Terminal,
                    _ => zwp_text_input_v3::ContentPurpose::Normal,
                };
            }
            "--hint" => {
                i += 1;
                for h in args[i].split(',') {
                    hint |= match h {
                        "multiline" => zwp_text_input_v3::ContentHint::Multiline,
                        "sensitive" => zwp_text_input_v3::ContentHint::SensitiveData,
                        "hidden" => zwp_text_input_v3::ContentHint::HiddenText,
                        _ => zwp_text_input_v3::ContentHint::empty(),
                    };
                }
            }
            "--state" => {
                i += 1;
                state_file = Some(args[i].clone());
            }
            "--toolkit" => {
                i += 1;
                send_cause = args[i] != "qt";
            }
            "--app-id" => {
                i += 1;
                app_id = args[i].clone();
            }
            "--text" => {
                i += 1;
                text = args[i].clone();
            }
            _ => {}
        }
        i += 1;
    }
    let conn = Connection::connect_to_env().expect("wayland");
    let (globals, mut q) = registry_queue_init::<Client>(&conn).unwrap();
    let qh = q.handle();
    let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 1..=4, ()).unwrap();
    let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).unwrap();
    let wm: xdg_wm_base::XdgWmBase = globals.bind(&qh, 1..=3, ()).unwrap();
    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=5, ()).unwrap();
    let tim: zwp_text_input_manager_v3::ZwpTextInputManagerV3 = globals.bind(&qh, 1..=1, ()).expect("text-input-v3");
    let ti = tim.get_text_input(&seat, &qh, ());
    let surface = compositor.create_surface(&qh, ());
    let xs = wm.get_xdg_surface(&surface, &qh, ());
    let top = xs.get_toplevel(&qh, ());
    top.set_title("glissando-textclient".into());
    top.set_app_id(app_id);
    surface.commit();
    let mut c = Client {
        ti: Some(ti),
        cursor: text.len(),
        hidden_selection: !text.is_empty(),
        text,
        preedit: String::new(),
        pending: Pending::default(),
        serial: 0,
        purpose,
        hint,
        send_cause,
        state_file,
        keymap: HashMap::new(),
        mods: 0,
        keyboard: None,
        pointer: None,
        configured: false,
        surface: Some(surface),
        shm: Some(shm),
        xdg_surface: Some(xs),
        width: 300,
        height: 200,
        events: 0,
    };
    let _ = &c.xdg_surface;
    c.write_state();
    loop {
        if q.blocking_dispatch(&mut c).is_err() {
            break;
        }
    }
}

// ---- virtual pointer -------------------------------------------------------

struct Ptr {
    outputs: Vec<(wl_output::WlOutput, i32, i32)>,
}

noop!(Ptr; zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1, zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1);

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Ptr {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for Ptr {
    fn event(_: &mut Self, _: &wl_seat::WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_output::WlOutput, ()> for Ptr {
    fn event(
        p: &mut Self,
        o: &wl_output::WlOutput,
        e: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Mode { width, height, .. } = e {
            if let Some(x) = p.outputs.iter_mut().find(|x| &x.0 == o) {
                x.1 = width;
                x.2 = height;
            }
        }
    }
}

/// Persistent pointer: reads scripts from stdin, answers "ok" after each.
fn pointer_server(extent: (u32, u32)) {
    use std::io::{BufRead, Write};
    let conn = Connection::connect_to_env().expect("wayland");
    let (globals, mut q) = registry_queue_init::<Ptr>(&conn).unwrap();
    let qh = q.handle();
    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=5, ()).unwrap();
    let mgr: zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1 =
        globals.bind(&qh, 1..=2, ()).expect("virtual pointer");
    let vp = mgr.create_virtual_pointer(Some(&seat), &qh, ());
    let mut st = Ptr { outputs: Vec::new() };
    q.roundtrip(&mut st).unwrap();
    let t0 = Instant::now();
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        run_script(&conn, &vp, &t0, &line, extent);
        q.roundtrip(&mut st).unwrap();
        println!("ok");
        std::io::stdout().flush().unwrap();
    }
}

fn pointer(script: &str, extent: (u32, u32)) {
    let conn = Connection::connect_to_env().expect("wayland");
    let (globals, mut q) = registry_queue_init::<Ptr>(&conn).unwrap();
    let qh = q.handle();
    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=5, ()).unwrap();
    let mgr: zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1 =
        globals.bind(&qh, 1..=2, ()).expect("virtual pointer");
    let vp = mgr.create_virtual_pointer(Some(&seat), &qh, ());
    let mut st = Ptr { outputs: Vec::new() };
    q.roundtrip(&mut st).unwrap();
    let t0 = Instant::now();
    run_script(&conn, &vp, &t0, script, extent);
    q.roundtrip(&mut st).unwrap();
}

fn run_script(
    conn: &Connection,
    vp: &zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
    t0: &Instant,
    script: &str,
    extent: (u32, u32),
) {
    let now = || t0.elapsed().as_millis() as u32;
    let (ew, eh) = extent;
    let move_to = |x: f64, y: f64| {
        vp.motion_absolute(now(), (x.max(0.0) as u32).min(ew - 1), (y.max(0.0) as u32).min(eh - 1), ew, eh);
        vp.frame();
    };
    let button = |down: bool| {
        let st = if down { wl_pointer::ButtonState::Pressed } else { wl_pointer::ButtonState::Released };
        vp.button(now(), 0x110, st);
        vp.frame();
    };
    for cmd in script.split(';').map(|s| s.trim()).filter(|s| !s.is_empty()) {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        let nums: Vec<f64> = parts[1..].iter().filter_map(|s| s.parse().ok()).collect();
        match parts[0] {
            "move" => move_to(nums[0], nums[1]),
            "down" => {
                move_to(nums[0], nums[1]);
                conn.flush().unwrap();
                std::thread::sleep(Duration::from_millis(15));
                button(true);
            }
            "up" => button(false),
            "tap" => {
                move_to(nums[0], nums[1]);
                conn.flush().unwrap();
                std::thread::sleep(Duration::from_millis(15));
                button(true);
                conn.flush().unwrap();
                std::thread::sleep(Duration::from_millis(40));
                button(false);
            }
            "swipe" => {
                // swipe MS x1 y1 x2 y2 ...: equal time per segment, 120 Hz.
                let ms = nums[0];
                let pts: Vec<(f64, f64)> = nums[1..].chunks(2).map(|c| (c[0], c[1])).collect();
                if pts.len() >= 2 {
                    move_to(pts[0].0, pts[0].1);
                    conn.flush().unwrap();
                    std::thread::sleep(Duration::from_millis(15));
                    button(true);
                    conn.flush().unwrap();
                    let seg_ms = ms / (pts.len() - 1) as f64;
                    for w in pts.windows(2) {
                        let steps = ((seg_ms / 8.0).ceil() as usize).max(2);
                        for s in 1..=steps {
                            let t = s as f64 / steps as f64;
                            move_to(w[0].0 + (w[1].0 - w[0].0) * t, w[0].1 + (w[1].1 - w[0].1) * t);
                            conn.flush().unwrap();
                            std::thread::sleep(Duration::from_millis(8));
                        }
                    }
                    button(false);
                }
            }
            "sleep" => {
                conn.flush().unwrap();
                std::thread::sleep(Duration::from_millis(nums[0] as u64));
            }
            other => eprintln!("unknown command {other}"),
        }
        conn.flush().unwrap();
        std::thread::sleep(Duration::from_millis(30));
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("textclient") => textclient(&args[2..]),
        Some("pointer-server") => {
            let extent = std::env::var("GLISSANDO_TK_EXTENT")
                .ok()
                .and_then(|s| {
                    let (a, b) = s.split_once('x')?;
                    Some((a.parse().ok()?, b.parse().ok()?))
                })
                .unwrap_or((360, 780));
            pointer_server(extent)
        }
        Some("pointer") => {
            let extent = std::env::var("GLISSANDO_TK_EXTENT")
                .ok()
                .and_then(|s| {
                    let (a, b) = s.split_once('x')?;
                    Some((a.parse().ok()?, b.parse().ok()?))
                })
                .unwrap_or((360, 780));
            pointer(&args[2..].join(" "), extent)
        }
        _ => {
            eprintln!(
                "usage: glissando-testkit textclient [--purpose P] [--hint H] [--state FILE] [--toolkit qt] [--app-id ID] | pointer SCRIPT"
            );
            std::process::exit(2);
        }
    }
}
