//! Session bus integration: the `sm.puri.OSK0` interface Phosh uses to show
//! and hide on-screen keyboards, and haptic feedback through feedbackd.

use gio::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const APP_ID: &str = "io.github.x_vlad_x.Glissando";
const OSK_NAME: &str = "sm.puri.OSK0";
const OSK_PATH: &str = "/sm/puri/OSK0";
const OSK_XML: &str = r#"<node>
  <interface name="sm.puri.OSK0">
    <method name="SetVisible"><arg type="b" direction="in" name="visible"/></method>
    <property name="Visible" type="b" access="read"/>
  </interface>
</node>"#;

pub struct OskService {
    conn: RefCell<Option<gio::DBusConnection>>,
    visible: Cell<bool>,
    _owner: Cell<Option<gio::OwnerId>>,
    /// gnome-session client proxy. Kept alive so its signals keep arriving; the
    /// registration itself lives as long as the bus connection.
    session: Rc<RefCell<Option<gio::DBusProxy>>>,
}

impl OskService {
    /// Own `sm.puri.OSK0`. `replace` takes the name over from a running
    /// keyboard that allows replacement (Stevia runs with
    /// `--allow-replacement`); we allow being replaced in turn so the
    /// system keyboard can always come back.
    pub fn start(
        replace: bool,
        set_visible: impl Fn(bool) + 'static,
        lost: impl Fn() + 'static,
        session_stop: impl Fn() + 'static,
    ) -> Rc<OskService> {
        let svc = Rc::new(OskService {
            conn: RefCell::new(None),
            visible: Cell::new(false),
            _owner: Cell::new(None),
            session: Rc::default(),
        });
        let set_visible = Rc::new(set_visible);
        let mut flags = gio::BusNameOwnerFlags::ALLOW_REPLACEMENT;
        if replace {
            flags |= gio::BusNameOwnerFlags::REPLACE;
        }
        let s1 = Rc::downgrade(&svc);
        let s2 = Rc::downgrade(&svc);
        let session_stop: Rc<dyn Fn()> = Rc::new(session_stop);
        let sv = set_visible.clone();
        let id = gio::bus_own_name(
            gio::BusType::Session,
            OSK_NAME,
            flags,
            move |conn, _| {
                let node = gio::DBusNodeInfo::for_xml(OSK_XML).expect("valid introspection XML");
                let iface = node.lookup_interface("sm.puri.OSK0").expect("interface");
                let sv2 = sv.clone();
                let s_get = s1.clone();
                let r = conn
                    .register_object(OSK_PATH, &iface)
                    .method_call(move |_, _, _, _, method, params, inv| {
                        if method == "SetVisible" {
                            if let Some((v,)) = params.get::<(bool,)>() {
                                sv2(v);
                            }
                            inv.return_value(None);
                        } else {
                            inv.return_error(gio::IOErrorEnum::NotSupported, "unknown method");
                        }
                    })
                    .property(move |_, _, _, _, prop| {
                        let v = s_get.upgrade().map(|s| s.visible.get()).unwrap_or(false);
                        if prop == "Visible" { v.to_variant() } else { false.to_variant() }
                    })
                    .build();
                if let Err(e) = r {
                    eprintln!("glissando: registering {OSK_PATH} failed: {e}");
                }
                if let Some(s) = s1.upgrade() {
                    *s.conn.borrow_mut() = Some(conn);
                }
            },
            move |conn, _| {
                if let Some(s) = s2.upgrade() {
                    gnome_session_register(&conn, s.session.clone(), session_stop.clone());
                }
            },
            move |_, _| lost(),
        );
        svc._owner.set(Some(id));
        svc
    }

    pub fn set_visible(&self, v: bool) {
        if self.visible.replace(v) == v {
            return;
        }
        if let Some(conn) = self.conn.borrow().as_ref() {
            let args = visible_changed(v);
            let _ = conn.emit_signal(None, OSK_PATH, "org.freedesktop.DBus.Properties", "PropertiesChanged", Some(&args));
        }
    }
}

/// Arguments of `PropertiesChanged` for the `Visible` property, typed
/// `(sa{sv}as)`. A tuple of Rust values would wrap the dictionary in a
/// variant, and GDBus proxies (Phosh) drop such signals: Phosh then kept
/// the keyboard as hidden and its home bar long press only ever showed it.
fn visible_changed(v: bool) -> glib::Variant {
    let changed = glib::VariantDict::new(None);
    changed.insert_value("Visible", &v.to_variant());
    glib::Variant::tuple_from_iter([OSK_NAME.to_variant(), changed.end(), Vec::<String>::new().to_variant()])
}

/// What a signal on gnome-session's `ClientPrivate` interface asks of us.
#[derive(Debug, PartialEq)]
enum SessionRequest {
    /// `QueryEndSession` / `EndSession`: answer `EndSessionResponse`. Without an
    /// answer gnome-session adds a "Not responding" inhibitor after 1 s and then
    /// waits out the end-session timeout (90 s in 48, 10 s in current main).
    Acknowledge,
    /// `Stop`: leave.
    Quit,
    Ignore,
}

fn session_request(signal: &str) -> SessionRequest {
    match signal {
        "QueryEndSession" | "EndSession" => SessionRequest::Acknowledge,
        "Stop" => SessionRequest::Quit,
        _ => SessionRequest::Ignore,
    }
}

/// Register this process as the OSK with gnome-session.
///
/// Phosh's OSK requirements make this mandatory
/// (<https://phosh.mobi/posts/phosh-osk-interface/> section 3): `sm.puri.OSK0` is a
/// `RequiredComponent` of `phosh.session`, and owning the bus name is not enough.
/// gnome-session ties the client to the app it started through
/// `DESKTOP_AUTOSTART_ID`; the app id is only a fallback.
///
/// Never blocks: gnome-session is itself busy while the session starts, and the main
/// loop also serves the keyboard. Other desktops have no session manager, which is
/// not an error.
fn gnome_session_register(conn: &gio::DBusConnection, slot: Rc<RefCell<Option<gio::DBusProxy>>>, stop: Rc<dyn Fn()>) {
    const SESSION_NAME: &str = "org.gnome.SessionManager";
    const SESSION_PATH: &str = "/org/gnome/SessionManager";
    const CLIENT_IFACE: &str = "org.gnome.SessionManager.ClientPrivate";

    let startup_id = std::env::var("DESKTOP_AUTOSTART_ID").unwrap_or_default();
    let args = (OSK_NAME, startup_id.as_str()).to_variant();
    let conn2 = conn.clone();
    conn.call(
        Some(SESSION_NAME),
        SESSION_PATH,
        SESSION_NAME,
        "RegisterClient",
        Some(&args),
        Some(glib::VariantTy::new("(o)").expect("valid type")),
        gio::DBusCallFlags::NO_AUTO_START,
        -1,
        gio::Cancellable::NONE,
        move |reply| {
            let reply = match reply {
                Ok(r) => r,
                Err(e) if e.matches(gio::DBusError::ServiceUnknown) || e.matches(gio::DBusError::NameHasNoOwner) => return,
                Err(e) => {
                    eprintln!("glissando: gnome-session registration failed: {e}");
                    return;
                }
            };
            let Some(path) = reply.child_value(0).str().map(str::to_owned) else { return };
            gio::DBusProxy::new(
                &conn2,
                gio::DBusProxyFlags::DO_NOT_LOAD_PROPERTIES | gio::DBusProxyFlags::DO_NOT_AUTO_START_AT_CONSTRUCTION,
                None,
                Some(SESSION_NAME),
                &path,
                CLIENT_IFACE,
                gio::Cancellable::NONE,
                move |proxy| match proxy {
                    Ok(proxy) => {
                        // `connect_g_signal` needs a Send + Sync closure, which cannot
                        // reach the single-threaded App. Everything here runs on the
                        // main loop, so connect locally.
                        proxy.connect_local("g-signal", false, move |args| {
                            let proxy = args[0].get::<gio::DBusProxy>().ok()?;
                            let signal = args[2].get::<String>().ok()?;
                            match session_request(&signal) {
                                SessionRequest::Acknowledge => proxy.call(
                                    "EndSessionResponse",
                                    Some(&(true, "").to_variant()),
                                    gio::DBusCallFlags::NONE,
                                    -1,
                                    gio::Cancellable::NONE,
                                    |r| {
                                        if let Err(e) = r {
                                            eprintln!("glissando: EndSessionResponse failed: {e}");
                                        }
                                    },
                                ),
                                SessionRequest::Quit => stop(),
                                SessionRequest::Ignore => {}
                            }
                            None
                        });
                        *slot.borrow_mut() = Some(proxy);
                    }
                    Err(e) => eprintln!("glissando: gnome-session client proxy failed: {e}"),
                },
            );
        },
    );
}

/// Haptic feedback through feedbackd. Uses `org.sigxcpu.Feedback.Haptic.Vibrate`
/// (custom strength/duration) and falls back to the themed
/// `button-pressed` event on older feedbackd versions.
pub struct Haptics {
    conn: Option<gio::DBusConnection>,
    pub enabled: bool,
    pub strength: f64,
    pub ms: u32,
    use_vibrate: Rc<Cell<bool>>,
}

impl Haptics {
    pub fn new() -> Haptics {
        let conn = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).ok();
        Haptics { conn, enabled: true, strength: 0.5, ms: 12, use_vibrate: Rc::new(Cell::new(true)) }
    }

    pub fn vibrate(&self, scale: f64) {
        if !self.enabled || self.strength <= 0.0 {
            return;
        }
        let Some(conn) = &self.conn else { return };
        if self.use_vibrate.get() {
            let pattern = vec![((self.strength * scale).clamp(0.05, 1.0), self.ms)];
            let args = (APP_ID, pattern).to_variant();
            let flag = self.use_vibrate.clone();
            conn.call(
                Some("org.sigxcpu.Feedback"),
                "/org/sigxcpu/Feedback",
                "org.sigxcpu.Feedback.Haptic",
                "Vibrate",
                Some(&args),
                None,
                gio::DBusCallFlags::NO_AUTO_START,
                250,
                gio::Cancellable::NONE,
                move |r| {
                    if let Err(e) = r {
                        if e.matches(gio::DBusError::UnknownMethod) || e.matches(gio::DBusError::UnknownInterface) {
                            flag.set(false);
                        }
                    }
                },
            );
        } else {
            let hints = glib::VariantDict::new(None);
            let args = (APP_ID, "button-pressed", hints.end(), -1i32).to_variant();
            conn.call(
                Some("org.sigxcpu.Feedback"),
                "/org/sigxcpu/Feedback",
                "org.sigxcpu.Feedback",
                "TriggerFeedback",
                Some(&args),
                None,
                gio::DBusCallFlags::NO_AUTO_START,
                250,
                gio::Cancellable::NONE,
                |_| {},
            );
        }
    }
}

/// Whether the desktop prefers a dark appearance (GNOME/Phosh setting).
pub fn prefers_dark() -> bool {
    let Some(src) = gio::SettingsSchemaSource::default() else { return false };
    if src.lookup("org.gnome.desktop.interface", true).is_none() {
        return false;
    }
    let s = gio::Settings::new("org.gnome.desktop.interface");
    if s.settings_schema().map(|sc| sc.has_key("color-scheme")).unwrap_or(false) {
        return s.string("color-scheme") == "prefer-dark";
    }
    false
}

/// Whether the screen keyboard should show automatically on focus.
pub fn osk_enabled() -> bool {
    let Some(src) = gio::SettingsSchemaSource::default() else { return true };
    if src.lookup("org.gnome.desktop.a11y.applications", true).is_none() {
        return true;
    }
    gio::Settings::new("org.gnome.desktop.a11y.applications").boolean("screen-keyboard-enabled")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_signals_are_answered_and_stop_quits() {
        assert_eq!(session_request("QueryEndSession"), SessionRequest::Acknowledge);
        assert_eq!(session_request("EndSession"), SessionRequest::Acknowledge);
        assert_eq!(session_request("Stop"), SessionRequest::Quit);
        assert_eq!(session_request("CancelEndSession"), SessionRequest::Ignore);
    }

    #[test]
    fn visible_changed_has_the_properties_changed_signature() {
        let args = visible_changed(true);
        assert_eq!(args.type_().as_str(), "(sa{sv}as)");
        let dict = glib::VariantDict::new(Some(&args.child_value(1)));
        assert_eq!(dict.lookup::<bool>("Visible").ok().flatten(), Some(true));
    }
}

