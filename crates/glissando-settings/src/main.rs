//! glissando-settings: GTK4/libadwaita preferences for the keyboard.
//!
//! Edits `config.toml`; the running keyboard watches the file and applies
//! changes immediately. Personal data (learned words, recents, pinned clips)
//! can be reviewed and erased here.

mod strings;

use adw::prelude::*;
use glissando_config::{Config, OneHanded, ThemeMode, config_dir, data_dir, find_resource, resource_dirs, state_dir};
use glissando_lang::user::{LEARN_THRESHOLD, UserModel};
use std::cell::RefCell;
use std::rc::Rc;
use strings::tr;

const APP_ID: &str = "io.github.x_vlad_x.Glissando.Settings";

type State = Rc<RefCell<Config>>;

fn save(state: &State) {
    if let Err(e) = state.borrow().save() {
        eprintln!("glissando-settings: cannot save {}: {e}", Config::path().display());
    }
}

/// Bind a switch row to a boolean config field.
fn switch(
    state: &State,
    title: &str,
    subtitle: Option<&str>,
    get: fn(&Config) -> bool,
    set: fn(&mut Config, bool),
) -> adw::SwitchRow {
    let row = adw::SwitchRow::new();
    row.set_title(title);
    if let Some(s) = subtitle {
        row.set_subtitle(s);
    }
    row.set_active(get(&state.borrow()));
    let st = state.clone();
    row.connect_active_notify(move |r| {
        set(&mut st.borrow_mut(), r.is_active());
        save(&st);
    });
    row
}

fn spin(
    state: &State,
    title: &str,
    min: f64,
    max: f64,
    step: f64,
    digits: u32,
    get: fn(&Config) -> f64,
    set: fn(&mut Config, f64),
) -> adw::SpinRow {
    let adj = gtk::Adjustment::new(get(&state.borrow()), min, max, step, step * 5.0, 0.0);
    let row = adw::SpinRow::new(Some(&adj), step, digits);
    row.set_title(title);
    let st = state.clone();
    row.connect_value_notify(move |r| {
        set(&mut st.borrow_mut(), r.value());
        save(&st);
    });
    row
}

fn combo(
    state: &State,
    title: &str,
    items: &[&str],
    get: fn(&Config) -> u32,
    set: fn(&mut Config, u32),
) -> adw::ComboRow {
    let row = adw::ComboRow::new();
    row.set_title(title);
    row.set_model(Some(&gtk::StringList::new(items)));
    row.set_selected(get(&state.borrow()));
    let st = state.clone();
    row.connect_selected_notify(move |r| {
        set(&mut st.borrow_mut(), r.selected());
        save(&st);
    });
    row
}

fn group(title: &str, rows: &[&gtk::Widget]) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(title);
    for r in rows {
        g.add(*r);
    }
    g
}

fn page(nav: &adw::NavigationView, title: &str, groups: &[adw::PreferencesGroup]) -> adw::NavigationPage {
    let prefs = adw::PreferencesPage::new();
    for g in groups {
        prefs.add(g);
    }
    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&adw::HeaderBar::new());
    tv.set_content(Some(&prefs));
    let p = adw::NavigationPage::new(&tv, title);
    let _ = nav;
    p
}

fn nav_row(
    nav: &adw::NavigationView,
    title: &str,
    subtitle: &str,
    icon: &str,
    target: adw::NavigationPage,
) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(title);
    row.set_subtitle(subtitle);
    row.add_prefix(&gtk::Image::from_icon_name(icon));
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    row.set_activatable(true);
    let n = nav.clone();
    row.connect_activated(move |_| n.push(&target));
    row
}

/// Available layouts: (id, display name), from all resource directories.
fn available_layouts() -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = Vec::new();
    for dir in resource_dirs() {
        let Ok(rd) = std::fs::read_dir(dir.join("layouts")) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().map(|x| x != "toml").unwrap_or(true)
                || p.file_stem().map(|s| s == "symbols").unwrap_or(false)
            {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(&p) else { continue };
            let Ok(t) = src.parse::<toml::Table>() else { continue };
            let id = t.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string();
            let name = t.get("name").and_then(|v| v.as_str()).unwrap_or(&id).to_string();
            if !id.is_empty() && !v.iter().any(|x| x.0 == id) {
                v.push((id, name));
            }
        }
    }
    v.sort_by(|a, b| a.1.cmp(&b.1));
    v
}

fn languages_page(nav: &adw::NavigationView, state: &State) -> adw::NavigationPage {
    let g = adw::PreferencesGroup::new();
    g.set_title(tr("Keyboard layouts"));
    g.set_description(Some(tr("Enabled layouts are switched with the globe key, in the order shown.")));
    let layouts = available_layouts();
    let rows: Rc<RefCell<Vec<(String, adw::SwitchRow)>>> = Rc::new(RefCell::new(Vec::new()));
    for (id, name) in &layouts {
        let row = adw::SwitchRow::new();
        row.set_title(name);
        row.set_subtitle(id);
        row.set_active(state.borrow().layouts.contains(id));
        let up = gtk::Button::from_icon_name("go-up-symbolic");
        up.set_valign(gtk::Align::Center);
        up.add_css_class("flat");
        up.set_tooltip_text(Some(tr("Move up")));
        row.add_prefix(&up);
        let st = state.clone();
        let idc = id.clone();
        up.connect_clicked(move |_| {
            let mut c = st.borrow_mut();
            if let Some(i) = c.layouts.iter().position(|l| *l == idc) {
                if i > 0 {
                    c.layouts.swap(i, i - 1);
                }
            }
            drop(c);
            save(&st);
        });
        let st = state.clone();
        let idc = id.clone();
        row.connect_active_notify(move |r| {
            let mut c = st.borrow_mut();
            if r.is_active() {
                if !c.layouts.contains(&idc) {
                    c.layouts.push(idc.clone());
                }
            } else if c.layouts.len() > 1 {
                c.layouts.retain(|l| *l != idc);
            } else {
                // At least one layout stays enabled.
                drop(c);
                r.set_active(true);
                return;
            }
            drop(c);
            save(&st);
        });
        g.add(&row);
        rows.borrow_mut().push((id.clone(), row));
    }
    let multi = switch(
        state,
        tr("Multilingual typing"),
        Some(tr("Suggest and correct in all enabled languages that share an alphabet")),
        |c| c.multilingual,
        |c, v| c.multilingual = v,
    );
    page(nav, tr("Languages"), &[g, group("", &[multi.upcast_ref()])])
}

fn preferences_page(nav: &adw::NavigationView, state: &State) -> adw::NavigationPage {
    let keys = group(
        tr("Keys"),
        &[
            switch(
                state,
                tr("Number row"),
                Some(tr("Always show digits above the letters")),
                |c| c.number_row,
                |c, v| c.number_row = v,
            )
            .upcast_ref(),
            switch(
                state,
                tr("Terminal keys"),
                Some(tr("Esc, Tab, Ctrl, Alt and arrows above the letters in terminals")),
                |c| c.terminal_keys,
                |c, v| c.terminal_keys = v,
            )
            .upcast_ref(),
            switch(
                state,
                tr("Touch & hold for digits and symbols"),
                Some(tr("Show hints on letter keys")),
                |c| c.hints,
                |c, v| c.hints = v,
            )
            .upcast_ref(),
            switch(
                state,
                tr("Key pop-up"),
                Some(tr("Show the pressed key above the finger")),
                |c| c.key_popup,
                |c, v| c.key_popup = v,
            )
            .upcast_ref(),
            spin(
                state,
                tr("Touch & hold duration (ms)"),
                150.0,
                1000.0,
                25.0,
                0,
                |c| c.long_press_ms as f64,
                |c, v| c.long_press_ms = v as u32,
            )
            .upcast_ref(),
        ],
    );
    let layout = group(
        tr("Layout"),
        &[
            combo(
                state,
                tr("One-handed mode"),
                &[tr("Off"), tr("Left"), tr("Right")],
                |c| match c.one_handed {
                    OneHanded::Off => 0,
                    OneHanded::Left => 1,
                    OneHanded::Right => 2,
                },
                |c, v| c.one_handed = [OneHanded::Off, OneHanded::Left, OneHanded::Right][v as usize % 3],
            )
            .upcast_ref(),
            spin(
                state,
                tr("Keyboard height (portrait)"),
                0.7,
                1.4,
                0.05,
                2,
                |c| c.height as f64,
                |c, v| c.height = v as f32,
            )
            .upcast_ref(),
            spin(
                state,
                tr("Keyboard height (landscape)"),
                0.7,
                1.4,
                0.05,
                2,
                |c| c.height_landscape as f64,
                |c, v| c.height_landscape = v as f32,
            )
            .upcast_ref(),
        ],
    );
    let tap = group(
        tr("Key tap"),
        &[
            switch(state, tr("Vibrate on keypress"), None, |c| c.haptics, |c, v| c.haptics = v).upcast_ref(),
            spin(
                state,
                tr("Vibration strength (%)"),
                5.0,
                100.0,
                5.0,
                0,
                |c| (c.haptic_strength * 100.0).round() as f64,
                |c, v| c.haptic_strength = v as f32 / 100.0,
            )
            .upcast_ref(),
            spin(
                state,
                tr("Vibration duration (ms)"),
                3.0,
                60.0,
                1.0,
                0,
                |c| c.haptic_ms as f64,
                |c, v| c.haptic_ms = v as u32,
            )
            .upcast_ref(),
        ],
    );
    let voice = group(
        tr("Voice typing"),
        &[switch(
            state,
            tr("Voice typing"),
            Some(tr("Local speech recognition with whisper.cpp; audio never leaves the device")),
            |c| c.voice,
            |c, v| c.voice = v,
        )
        .upcast_ref()],
    );
    page(nav, tr("Preferences"), &[keys, layout, tap, voice])
}

fn theme_page(nav: &adw::NavigationView, state: &State) -> adw::NavigationPage {
    let g = group(
        tr("Appearance"),
        &[
            combo(
                state,
                tr("Theme"),
                &[tr("Follow system"), tr("Light"), tr("Dark")],
                |c| match c.theme {
                    ThemeMode::System => 0,
                    ThemeMode::Light => 1,
                    ThemeMode::Dark => 2,
                },
                |c, v| c.theme = [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark][v as usize % 3],
            )
            .upcast_ref(),
            switch(
                state,
                tr("Key borders"),
                Some(tr("Higher contrast between keys")),
                |c| c.key_borders,
                |c, v| c.key_borders = v,
            )
            .upcast_ref(),
            spin(state, tr("Font size"), 0.8, 1.5, 0.05, 2, |c| c.font_scale as f64, |c, v| c.font_scale = v as f32)
                .upcast_ref(),
        ],
    );
    let font = adw::EntryRow::new();
    font.set_title(tr("Font family"));
    font.set_text(&state.borrow().font);
    font.set_show_apply_button(true);
    let st = state.clone();
    font.connect_apply(move |e| {
        st.borrow_mut().font = e.text().to_string();
        save(&st);
    });
    g.add(&font);
    page(nav, tr("Theme"), &[g])
}

fn glide_page(nav: &adw::NavigationView, state: &State) -> adw::NavigationPage {
    let g = group(
        tr("Glide typing"),
        &[
            switch(
                state,
                tr("Glide typing"),
                Some(tr("Type words by sliding from letter to letter")),
                |c| c.glide,
                |c, v| c.glide = v,
            )
            .upcast_ref(),
            switch(
                state,
                tr("Glide trail"),
                Some(tr("Show how the finger moves")),
                |c| c.glide_trail,
                |c, v| c.glide_trail = v,
            )
            .upcast_ref(),
            switch(
                state,
                tr("Glide delete"),
                Some(tr("Delete words by sliding left from the delete key")),
                |c| c.glide_delete,
                |c, v| c.glide_delete = v,
            )
            .upcast_ref(),
            switch(
                state,
                tr("Glide cursor control"),
                Some(tr("Slide on the space bar to move the cursor")),
                |c| c.glide_cursor,
                |c, v| c.glide_cursor = v,
            )
            .upcast_ref(),
            switch(
                state,
                tr("Trackpad on long-press space"),
                None,
                |c| c.trackpad_long_press_space,
                |c, v| c.trackpad_long_press_space = v,
            )
            .upcast_ref(),
        ],
    );
    page(nav, tr("Glide typing"), &[g])
}

fn correction_page(nav: &adw::NavigationView, state: &State) -> adw::NavigationPage {
    let g = group(
        tr("Corrections & suggestions"),
        &[
            switch(
                state,
                tr("Suggestion strip"),
                Some(tr("Show word suggestions while typing")),
                |c| c.suggestions,
                |c, v| c.suggestions = v,
            )
            .upcast_ref(),
            switch(state, tr("Next-word suggestions"), None, |c| c.next_word, |c, v| c.next_word = v).upcast_ref(),
            switch(state, tr("Auto-correction"), None, |c| c.autocorrect, |c, v| c.autocorrect = v).upcast_ref(),
            switch(state, tr("Auto-capitalization"), None, |c| c.autocap, |c, v| c.autocap = v).upcast_ref(),
            switch(state, tr("Auto-space after suggestions"), None, |c| c.autospace, |c, v| c.autospace = v)
                .upcast_ref(),
            switch(state, tr("Double-space period"), None, |c| c.double_space_period, |c, v| c.double_space_period = v)
                .upcast_ref(),
            switch(
                state,
                tr("Underline the word being typed"),
                Some(tr("Turn off for apps with broken preedit support")),
                |c| c.composing_underline,
                |c, v| c.composing_underline = v,
            )
            .upcast_ref(),
        ],
    );
    page(nav, tr("Text correction"), &[g])
}

fn clipboard_page(nav: &adw::NavigationView, state: &State) -> adw::NavigationPage {
    let g = group(
        tr("Clipboard"),
        &[
            switch(
                state,
                tr("Clipboard history"),
                Some(tr("Keep copied text in memory to paste it later")),
                |c| c.clipboard_history,
                |c, v| c.clipboard_history = v,
            )
            .upcast_ref(),
            spin(
                state,
                tr("Keep history for (hours)"),
                1.0,
                72.0,
                1.0,
                0,
                |c| c.clipboard_hours as f64,
                |c, v| c.clipboard_hours = v as u32,
            )
            .upcast_ref(),
        ],
    );
    g.set_description(Some(tr("Text copied from password managers is never recorded. History is kept in memory only; pinned items are stored on this device.")));
    page(nav, tr("Clipboard"), &[g])
}

/// Installed apps that appear in the launcher: (app id, name, icon). The id
/// is what the window reports as its Wayland app_id: StartupWMClass when the
/// desktop file sets it, otherwise the desktop file name.
fn installed_apps() -> Vec<(String, String, Option<gtk::gio::Icon>)> {
    let mut v: Vec<_> = gtk::gio::AppInfo::all()
        .into_iter()
        .filter(|a| a.should_show())
        .filter_map(|a| {
            let file_id = a.id()?.to_string();
            let stem = file_id.strip_suffix(".desktop").unwrap_or(&file_id).to_string();
            let kf = gtk::glib::KeyFile::new();
            let wm_class = kf
                .load_from_data_dirs(format!("applications/{file_id}"), gtk::glib::KeyFileFlags::NONE)
                .ok()
                .and_then(|_| kf.string("Desktop Entry", "StartupWMClass").ok())
                .map(|s| s.to_string());
            Some((wm_class.unwrap_or(stem), a.display_name().to_string(), a.icon()))
        })
        .collect();
    v.sort_by_key(|a| a.1.to_lowercase());
    v.dedup_by(|a, b| a.0 == b.0);
    v
}

fn app_row(id: &str, apps: &[(String, String, Option<gtk::gio::Icon>)]) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    let found = apps.iter().find(|a| a.0.eq_ignore_ascii_case(id));
    row.set_title(&found.map(|a| a.1.clone()).unwrap_or_else(|| id.to_string()));
    row.set_subtitle(id);
    let icon = match found.and_then(|a| a.2.clone()) {
        Some(i) => gtk::Image::from_gicon(&i),
        None => gtk::Image::from_icon_name("application-x-executable-symbolic"),
    };
    icon.set_pixel_size(32);
    row.add_prefix(&icon);
    row
}

/// Stevia's list of apps without automatic keyboard, if it is installed.
fn stevia_ignored_apps() -> Vec<String> {
    let schema = "mobi.phosh.osk";
    let has = gtk::gio::SettingsSchemaSource::default()
        .and_then(|s| s.lookup(schema, true))
        .map(|s| s.has_key("ignore-activation"))
        .unwrap_or(false);
    if !has {
        return Vec::new();
    }
    gtk::gio::Settings::new(schema).strv("ignore-activation").iter().map(|s| s.to_string()).collect()
}

fn add_blocked_app(state: &State, id: &str) {
    let id = id.trim();
    if id.is_empty() || state.borrow().auto_show_blocked(id) {
        return;
    }
    state.borrow_mut().no_auto_show.push(id.to_string());
    save(state);
}

fn fill_blocked_list(list: &adw::PreferencesGroup, rows: &Rc<RefCell<Vec<adw::ActionRow>>>, state: &State) {
    for r in rows.borrow_mut().drain(..) {
        list.remove(&r);
    }
    let apps = installed_apps();
    for id in state.borrow().no_auto_show.clone() {
        let row = app_row(&id, &apps);
        let del = gtk::Button::from_icon_name("user-trash-symbolic");
        del.set_valign(gtk::Align::Center);
        del.add_css_class("flat");
        del.set_tooltip_text(Some(tr("Remove")));
        let (st, l, rs) = (state.clone(), list.clone(), rows.clone());
        del.connect_clicked(move |_| {
            st.borrow_mut().no_auto_show.retain(|a| *a != id);
            save(&st);
            fill_blocked_list(&l, &rs, &st);
        });
        row.add_suffix(&del);
        list.add(&row);
        rows.borrow_mut().push(row);
    }
}

/// Picker with all installed apps; tapping one adds it to the list.
fn app_picker_page(state: &State, on_added: Rc<dyn Fn()>) -> adw::NavigationPage {
    let search = gtk::SearchEntry::new();
    search.set_margin_start(12);
    search.set_margin_end(12);
    search.set_margin_top(6);
    search.set_margin_bottom(6);
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::None);
    list.set_margin_start(12);
    list.set_margin_end(12);
    list.set_margin_bottom(12);
    list.set_valign(gtk::Align::Start);
    let page_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&list));
    page_box.append(&search);
    page_box.append(&scroll);
    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&adw::HeaderBar::new());
    tv.set_content(Some(&page_box));
    let page = adw::NavigationPage::new(&tv, tr("Add app"));
    let apps = installed_apps();
    let mut rows = Vec::new();
    for (id, name, _) in &apps {
        let row = app_row(id, &apps);
        row.set_activatable(true);
        let (st, cb, app, p) = (state.clone(), on_added.clone(), id.clone(), page.clone());
        row.connect_activated(move |_| {
            add_blocked_app(&st, &app);
            cb();
            if let Some(nav) = p.parent().and_then(|w| w.downcast::<adw::NavigationView>().ok()) {
                nav.pop();
            }
        });
        list.append(&row);
        rows.push((row, format!("{} {}", name.to_lowercase(), id.to_lowercase())));
    }
    search.connect_search_changed(move |e| {
        let q = e.text().to_lowercase();
        for (row, key) in &rows {
            row.set_visible(q.is_empty() || key.contains(q.as_str()));
        }
    });
    page
}

fn apps_page(nav: &adw::NavigationView, state: &State) -> adw::NavigationPage {
    let list = adw::PreferencesGroup::new();
    list.set_title(tr("Don't open automatically in"));
    list.set_description(Some(tr(
        "In these apps the keyboard does not appear when a text field gets focus, for example in a calculator with its own keypad. It can still be opened from the bottom bar.",
    )));
    let rows: Rc<RefCell<Vec<adw::ActionRow>>> = Rc::new(RefCell::new(Vec::new()));
    fill_blocked_list(&list, &rows, state);
    let refresh: Rc<dyn Fn()> = {
        let (l, r, s) = (list.clone(), rows.clone(), state.clone());
        Rc::new(move || fill_blocked_list(&l, &r, &s))
    };
    let add = adw::ActionRow::new();
    add.set_title(tr("Add app"));
    add.add_prefix(&gtk::Image::from_icon_name("list-add-symbolic"));
    add.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    add.set_activatable(true);
    let (n, st, cb) = (nav.clone(), state.clone(), refresh.clone());
    add.connect_activated(move |_| n.push(&app_picker_page(&st, cb.clone())));
    let manual = adw::EntryRow::new();
    manual.set_title(tr("Add by app ID"));
    manual.set_show_apply_button(true);
    let (st, cb) = (state.clone(), refresh.clone());
    manual.connect_apply(move |e| {
        add_blocked_app(&st, &e.text());
        e.set_text("");
        cb();
    });
    let mut actions: Vec<&gtk::Widget> = vec![add.upcast_ref(), manual.upcast_ref()];
    let import = adw::ActionRow::new();
    let stevia = stevia_ignored_apps();
    if !stevia.is_empty() {
        import.set_title(tr("Import from the Phosh keyboard (Stevia)"));
        import.set_subtitle(&stevia.join(", "));
        let btn = gtk::Button::with_label(tr("Import"));
        btn.set_valign(gtk::Align::Center);
        import.add_suffix(&btn);
        let (st, cb) = (state.clone(), refresh.clone());
        btn.connect_clicked(move |_| {
            for id in &stevia {
                add_blocked_app(&st, id);
            }
            cb();
        });
        actions.push(import.upcast_ref());
    }
    page(nav, tr("Apps"), &[list, group("", &actions)])
}

fn bump_dict_epoch(state: &State) {
    state.borrow_mut().user_dict_epoch += 1;
    save(state);
}

fn dictionary_group(state: &State, code: &str) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&format!("{} ({code})", tr("Personal dictionary")));
    let path = data_dir().join("user").join(format!("{code}.tsv"));
    let add = adw::EntryRow::new();
    add.set_title(tr("Add a word"));
    add.set_show_apply_button(true);
    let st = state.clone();
    let p2 = path.clone();
    add.connect_apply(move |e| {
        let w = e.text().trim().to_string();
        if w.is_empty() {
            return;
        }
        let mut m = UserModel::load(&p2);
        m.pin(&w);
        let _ = m.save();
        e.set_text("");
        bump_dict_epoch(&st);
    });
    g.add(&add);
    let model = UserModel::load(&path);
    let mut words: Vec<(String, u32, bool)> = model
        .words
        .iter()
        .filter(|(_, u)| !u.blocked && (u.pinned || u.count >= LEARN_THRESHOLD))
        .map(|(w, u)| (w.clone(), u.count, u.pinned))
        .collect();
    words.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.to_lowercase().cmp(&b.0.to_lowercase())));
    for (w, count, pinned) in words.into_iter().take(500) {
        let row = adw::ActionRow::new();
        row.set_title(&w);
        row.set_subtitle(&if pinned {
            tr("added by you").to_string()
        } else {
            format!("{} × {count}", tr("learned"))
        });
        let del = gtk::Button::from_icon_name("user-trash-symbolic");
        del.set_valign(gtk::Align::Center);
        del.add_css_class("flat");
        del.set_tooltip_text(Some(tr("Forget this word")));
        let st = state.clone();
        let p2 = path.clone();
        let r2 = row.clone();
        let g2 = g.clone();
        del.connect_clicked(move |_| {
            let mut m = UserModel::load(&p2);
            m.block(&w);
            let _ = m.save();
            g2.remove(&r2);
            bump_dict_epoch(&st);
        });
        row.add_suffix(&del);
        g.add(&row);
    }
    g
}

fn privacy_page(nav: &adw::NavigationView, state: &State, window: &adw::ApplicationWindow) -> adw::NavigationPage {
    let info = adw::PreferencesGroup::new();
    info.set_title(tr("Privacy"));
    info.set_description(Some(tr(
        "glissando works entirely on this device. It has no network access, no telemetry and no accounts. Nothing you type leaves the phone.",
    )));
    info.add(&switch(
        state,
        tr("Personalization"),
        Some(tr("Learn new words and your word order on this device (never in password fields)")),
        |c| c.personalization,
        |c, v| c.personalization = v,
    ));
    info.add(&switch(state, tr("Remember recent emoji"), None, |c| c.emoji_recent, |c, v| c.emoji_recent = v));
    let erase = adw::ActionRow::new();
    erase.set_title(tr("Delete learned words and data"));
    erase.set_subtitle(tr("Personal dictionary, learned words, recent emoji and pinned clipboard items"));
    let btn = gtk::Button::with_label(tr("Delete"));
    btn.add_css_class("destructive-action");
    btn.set_valign(gtk::Align::Center);
    erase.add_suffix(&btn);
    info.add(&erase);
    let st = state.clone();
    let win = window.clone();
    btn.connect_clicked(move |_| {
        let d = adw::AlertDialog::new(Some(tr("Delete all personal data?")), Some(tr("This cannot be undone.")));
        d.add_response("cancel", tr("Cancel"));
        d.add_response("delete", tr("Delete"));
        d.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        d.set_close_response("cancel");
        let st2 = st.clone();
        d.connect_response(None, move |_, r| {
            if r == "delete" {
                let _ = std::fs::remove_dir_all(data_dir().join("user"));
                let _ = std::fs::remove_file(state_dir().join("recent-emoji"));
                let _ = std::fs::remove_file(state_dir().join("pinned-clips"));
                st2.borrow_mut().user_data_epoch += 1;
                save(&st2);
            }
        });
        d.present(Some(&win));
    });
    let mut groups = vec![info];
    let mut codes: Vec<String> = Vec::new();
    for id in state.borrow().layouts.iter() {
        if let Some(p) = find_resource(&format!("layouts/{id}.toml")) {
            if let Ok(t) = std::fs::read_to_string(p).unwrap_or_default().parse::<toml::Table>() {
                for l in t.get("languages").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
                    if let Some(s) = l.as_str() {
                        if !codes.iter().any(|c| c == s) {
                            codes.push(s.to_string());
                        }
                    }
                }
            }
        }
    }
    for c in codes {
        groups.push(dictionary_group(state, &c));
    }
    page(nav, tr("Privacy & dictionary"), &groups)
}

fn about_page(nav: &adw::NavigationView) -> adw::NavigationPage {
    let g = adw::PreferencesGroup::new();
    g.set_title("glissando");
    g.set_description(Some(&format!(
        "{} {}\n\n{}",
        tr("Version"),
        env!("CARGO_PKG_VERSION"),
        tr("Free software under the GNU GPL v3 or later. Word lists: Android Open Source Project (Apache-2.0); word pair statistics: Leipzig Corpora Collection (CC BY 4.0); emoji names: Unicode CLDR (Unicode License v3).")
    )));
    let cfg = adw::ActionRow::new();
    cfg.set_title(tr("Configuration file"));
    cfg.set_subtitle(&Config::path().display().to_string());
    cfg.set_subtitle_selectable(true);
    g.add(&cfg);
    let data = adw::ActionRow::new();
    data.set_title(tr("Personal data"));
    data.set_subtitle(&format!("{}\n{}", data_dir().display(), state_dir().display()));
    data.set_subtitle_selectable(true);
    g.add(&data);
    page(nav, tr("About"), &[g])
}

fn build(app: &adw::Application) {
    let _ = std::fs::create_dir_all(config_dir());
    let state: State = Rc::new(RefCell::new(Config::load()));
    let window = adw::ApplicationWindow::new(app);
    window.set_default_size(400, 720);
    window.set_title(Some(tr("Keyboard settings")));
    let nav = adw::NavigationView::new();
    let main = adw::PreferencesPage::new();
    let g = adw::PreferencesGroup::new();
    let entries = [
        (
            tr("Languages"),
            tr("Layouts and multilingual typing"),
            "preferences-desktop-locale-symbolic",
            languages_page(&nav, &state),
        ),
        (
            tr("Preferences"),
            tr("Keys, layout, height, vibration"),
            "preferences-system-symbolic",
            preferences_page(&nav, &state),
        ),
        (tr("Theme"), tr("Light, dark, font"), "preferences-desktop-appearance-symbolic", theme_page(&nav, &state)),
        (
            tr("Text correction"),
            tr("Suggestions, auto-correction, capitalization"),
            "tools-check-spelling-symbolic",
            correction_page(&nav, &state),
        ),
        (tr("Glide typing"), tr("Gestures and trail"), "input-touchpad-symbolic", glide_page(&nav, &state)),
        (tr("Clipboard"), tr("History and retention"), "edit-paste-symbolic", clipboard_page(&nav, &state)),
        (
            tr("Apps"),
            tr("Where the keyboard does not open by itself"),
            "view-app-grid-symbolic",
            apps_page(&nav, &state),
        ),
        (
            tr("Privacy & dictionary"),
            tr("Learned words, delete data"),
            "security-high-symbolic",
            privacy_page(&nav, &state, &window),
        ),
        (tr("About"), tr("Version and licences"), "help-about-symbolic", about_page(&nav)),
    ];
    for (title, sub, icon, target) in entries {
        g.add(&nav_row(&nav, title, sub, icon, target));
    }
    main.add(&g);
    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&adw::HeaderBar::new());
    tv.set_content(Some(&main));
    nav.add(&adw::NavigationPage::new(&tv, tr("Keyboard settings")));
    window.set_content(Some(&nav));
    window.present();
}

fn main() -> gtk::glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(build);
    app.run()
}
