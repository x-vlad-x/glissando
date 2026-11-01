//! glissando: an on-screen keyboard for Phosh.

mod clipboard;
use glissando_config as config;
mod dbus;
mod emoji;
mod i18n;
mod icons;
mod keyboard;
mod panels;
mod render;
mod theme;
mod vk;
mod wl;

use config::Config;
use gio::prelude::*;
use glissando_core::layout::{Layout, SymbolSet};
use glissando_lang::dict::Dict;
use glissando_lang::engine::{Engine, Settings};
use glissando_lang::model::Language;
use glissando_lang::user::UserModel;
use std::path::PathBuf;

const USAGE: &str = "Usage: glissando [OPTIONS]

  --replace            take over sm.puri.OSK0 from the running keyboard
  --show               show immediately (virtual keyboard typing until a text field is focused)
  --no-dbus            do not register on the session bus (testing)
  --dump-geometry F    write key positions as JSON to F whenever they change (testing)
  --version            print version
  -h, --help           this help

Environment: GLISSANDO_DATA_DIR (extra data directory), GLISSANDO_CONFIG (config file),
GLISSANDO_DEBUG=1 (state logging; never logs typed text)";

pub fn engine_settings(c: &Config) -> Settings {
    Settings {
        suggestions: c.suggestions,
        autocorrect: c.autocorrect,
        autocap: c.autocap,
        autospace: c.autospace,
        double_space_period: c.double_space_period,
        next_word: c.next_word,
        learn: c.personalization,
        preedit: c.composing_underline,
        autospace_after_punct: false,
    }
}

fn load_layouts(config: &Config) -> Vec<Layout> {
    let sym_src =
        config::find_resource("layouts/symbols.toml").and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    let symbols = SymbolSet::parse(&sym_src).unwrap_or_else(|e| {
        eprintln!("glissando: symbols.toml: {e}");
        SymbolSet::default()
    });
    let mut out = Vec::new();
    for id in &config.layouts {
        let Some(p) = config::find_resource(&format!("layouts/{id}.toml")) else {
            eprintln!("glissando: layout {id} not found");
            continue;
        };
        match std::fs::read_to_string(&p)
            .map_err(|e| e.to_string())
            .and_then(|s| Layout::parse(&s, &symbols).map_err(|e| e.to_string()))
        {
            Ok(l) => out.push(l),
            Err(e) => eprintln!("glissando: {}: {e}", p.display()),
        }
    }
    if out.is_empty() {
        // Built-in minimal fallback so the keyboard always works.
        let src = r#"id = "en"
name = "English"
short = "EN"
languages = ["en"]
[layers.letters]
rows = [{ keys = "q w e r t y u i o p" }, { keys = "{gap:0.5} a s d f g h j k l {gap:0.5}" }, { keys = "{shift:1.5} z x c v b n m {backspace:1.5}" }, { keys = "{layer=symbols:1.5} {comma} {space:5} {period} {enter:1.5}" }]
[layers.symbols]
rows = [{ keys = "1 2 3 4 5 6 7 8 9 0" }, { keys = "@ # $ _ & - + ( ) /" }, { keys = "* \" ' : ; ! ? {backspace:3}" }, { keys = "{layer=letters:1.5} {comma} {space:5} {period} {enter:1.5}" }]
"#;
        out.push(Layout::parse(src, &symbols).expect("built-in layout"));
    }
    out
}

fn load_languages(layouts: &[Layout]) -> Vec<Language> {
    let mut codes: Vec<String> = Vec::new();
    for l in layouts {
        for c in &l.languages {
            if !codes.contains(c) {
                codes.push(c.clone());
            }
        }
    }
    let user_dir = config::data_dir().join("user");
    codes
        .iter()
        .map(|code| {
            let dict = config::find_resource(&format!("dict/{code}.gld")).and_then(|p| match Dict::open(&p) {
                Ok(d) => Some(d),
                Err(e) => {
                    eprintln!("glissando: {}: {e}", p.display());
                    None
                }
            });
            if dict.is_none() {
                eprintln!("glissando: no dictionary for {code}; suggestions limited to learned words");
            }
            Language::new(code, dict, UserModel::load(&user_dir.join(format!("{code}.tsv"))))
        })
        .collect()
}

/// Offscreen rendering benchmark with phone-like geometry (no compositor needed).
fn bench_render() {
    let config = Config::load();
    let layouts = load_layouts(&config);
    let engine = Engine::new(Vec::new(), engine_settings(&config));
    let mut kb = keyboard::Keyboard::new(config.clone(), layouts, engine);
    let (w, scale) = (413.0f32, 1.74f64);
    kb.resize(w, false);
    let h = kb.desired_height();
    let (pw, ph) = ((w as f64 * scale).round() as i32, (h as f64 * scale).round() as i32);
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, pw, ph).unwrap();
    let mut r = render::Renderer::new(&config.font);
    let theme = theme::Theme::dark();
    let time = |name: &str, kb: &mut keyboard::Keyboard, r: &mut render::Renderer, n: u32| {
        let t = std::time::Instant::now();
        for i in 0..n {
            let v = kb.view(1000 + i);
            r.render(&surface, scale, &v, &theme);
        }
        println!("{name:<28} {:6.2} ms/frame", t.elapsed().as_secs_f64() * 1000.0 / n as f64);
    };
    time("cold key layer", &mut kb, &mut r, 1);
    time("idle frame (cached)", &mut kb, &mut r, 30);
    kb.bench_press(5);
    time("key pressed + preview", &mut kb, &mut r, 30);
    kb.bench_trail(1000);
    time("glide trail", &mut kb, &mut r, 30);
    kb.toggle_shift_for_bench();
    time("shift toggle (first time)", &mut kb, &mut r, 1);
    kb.toggle_shift_for_bench();
    time("shift toggle back (cached)", &mut kb, &mut r, 1);
    println!("buffer {pw}x{ph} at scale {scale}");
}

fn main() {
    // A keyboard needs no GVfs; keep gio local-only (no daemons, no network mounts).
    // SAFETY: no other threads exist yet.
    unsafe { std::env::set_var("GIO_USE_VFS", "local") };
    let mut opts = wl::Options { replace: false, show: false, dump_geometry: None, no_dbus: false };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--replace" => opts.replace = true,
            "--show" => opts.show = true,
            "--no-dbus" => opts.no_dbus = true,
            "--dump-geometry" => opts.dump_geometry = args.next().map(PathBuf::from),
            "--allow-replacement" => {}
            "--bench-render" => {
                bench_render();
                return;
            }
            "--version" => {
                println!("glissando {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                return;
            }
            other => {
                eprintln!("glissando: unknown option {other}\n{USAGE}");
                std::process::exit(2);
            }
        }
    }
    let config = Config::load();
    let layouts = load_layouts(&config);
    let langs = load_languages(&layouts);
    let engine = Engine::new(langs, engine_settings(&config));
    let mut kb = keyboard::Keyboard::new(config.clone(), layouts, engine);
    kb.resize(360.0, false);
    kb.voice_available = config.voice && wl::voice_available();
    if let Some(p) = config::find_resource("emoji/emoji.tsv") {
        panels::init_emoji(emoji::EmojiDb::load(&p), emoji::Recents::load(&config::state_dir().join("recent-emoji")));
    }
    let main_loop = glib::MainLoop::new(None, false);
    let app = match wl::run(opts, config, kb) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("glissando: {e}");
            std::process::exit(1);
        }
    };
    // Reload the configuration when the settings app changes it.
    let cfg_file = gio::File::for_path(Config::path());
    let monitor = cfg_file.monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE).ok();
    if let Some(m) = &monitor {
        let a = app.clone();
        m.connect_changed(move |_, _, _, ev| {
            if matches!(
                ev,
                gio::FileMonitorEvent::ChangesDoneHint
                    | gio::FileMonitorEvent::Created
                    | gio::FileMonitorEvent::Changed
            ) {
                // A broken file must not count as the defaults: their data epoch
                // would delete the user's words.
                let cfg = match Config::read() {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("glissando: ignoring invalid {}: {e}", Config::path().display());
                        return;
                    }
                };
                if let Ok(mut app) = a.try_borrow_mut()
                    && cfg != app.kb.config
                {
                    let layouts = load_layouts(&cfg);
                    let layout_ids: Vec<&str> = layouts.iter().map(|l| l.id.as_str()).collect();
                    let lang_reload = layout_ids.join(",") != app.kb.config.layouts.join(",");
                    if lang_reload {
                        let langs = load_languages(&layouts);
                        let mut e = Engine::new(langs, engine_settings(&cfg));
                        std::mem::swap(&mut app.kb.engine, &mut e);
                        e.save_user_data();
                    }
                    let old = app.kb.config.clone();
                    if cfg.user_data_epoch != old.user_data_epoch {
                        app.kb.engine.clear_user_data();
                        panels::RECENTS.with(|r| r.borrow_mut().clear());
                        clipboard::HISTORY.with(|h| h.borrow_mut().clear_all());
                    } else if cfg.user_dict_epoch != old.user_dict_epoch {
                        let dir = config::data_dir().join("user");
                        for l in app.kb.engine.langs.iter_mut() {
                            l.user = UserModel::load(&dir.join(format!("{}.tsv", l.code)));
                        }
                    }
                    app.apply_config(cfg, layouts);
                }
            }
        });
    }
    for sig in [libc_sig::SIGTERM, libc_sig::SIGINT, libc_sig::SIGHUP] {
        let a = app.clone();
        let ml = main_loop.clone();
        glib_unix::unix_signal_add_local(sig, move || {
            a.borrow_mut().shutdown();
            ml.quit();
            glib::ControlFlow::Break
        });
    }
    let a = app.clone();
    let ml = main_loop.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
        if a.borrow().quit {
            a.borrow_mut().shutdown();
            ml.quit();
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
    main_loop.run();
    drop(monitor);
}

mod libc_sig {
    pub const SIGHUP: i32 = 1;
    pub const SIGINT: i32 = 2;
    pub const SIGTERM: i32 = 15;
}
