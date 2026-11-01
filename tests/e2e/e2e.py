#!/usr/bin/env python3
"""End-to-end tests: real Wayland compositor (headless sway), the real
keyboard binary, a text-input-v3 test client and a virtual pointer.

Every test starts a private session (own XDG dirs, runtime dir and D-Bus via
dbus-run-session) so nothing touches the developer's desktop.

    tests/e2e/e2e.py [-k NAME] [--keep] [--shots DIR] [--release]
"""
import argparse, json, os, pathlib, shutil, subprocess, sys, tempfile, time, traceback

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SWAY = os.path.join(ROOT, ".devtools/root/usr/bin/sway")
GRIM = os.path.join(ROOT, ".devtools/root/usr/bin/grim")
OUT_W, OUT_H = 360, 780
LANDSCAPE = os.environ.get("E2E_LANDSCAPE") == "1"
if LANDSCAPE:
    OUT_W, OUT_H = 780, 360


DESKTOP_SCHEMAS = """<schemalist>
  <enum id="e2e.ColorScheme">
    <value nick="default" value="0"/>
    <value nick="prefer-dark" value="1"/>
    <value nick="prefer-light" value="2"/>
  </enum>
  <schema id="org.gnome.desktop.interface" path="/org/gnome/desktop/interface/">
    <key name="color-scheme" enum="e2e.ColorScheme"><default>'default'</default></key>
  </schema>
  <schema id="org.gnome.desktop.a11y.applications" path="/org/gnome/desktop/a11y/applications/">
    <key name="screen-keyboard-enabled" type="b"><default>true</default></key>
  </schema>
</schemalist>
"""
A11Y = "org.gnome.desktop.a11y.applications"


class Session:
    def __init__(self, args, purpose=None, hint=None, config=None, kb_args=(), toolkit=None, app_id=None,
                 text=None, dbus=False, session_manager=False, expect_keyboard=True, desktop_settings=False):
        self.args = args
        self.tmp = tempfile.mkdtemp(prefix="glissando-e2e-")
        self.env = dict(os.environ)
        rt = os.path.join(self.tmp, "rt")
        os.mkdir(rt, 0o700)
        for k in ["WAYLAND_DISPLAY", "DISPLAY", "SWAYSOCK"]:
            self.env.pop(k, None)
        self.env.update({
            "XDG_RUNTIME_DIR": rt,
            "XDG_CONFIG_HOME": os.path.join(self.tmp, "config"),
            "XDG_DATA_HOME": os.path.join(self.tmp, "data"),
            "XDG_STATE_HOME": os.path.join(self.tmp, "state"),
            "GLISSANDO_DATA_DIR": os.path.join(ROOT, "data"),
            "GLISSANDO_AUTOSHOW": "1",
            "WLR_BACKENDS": "headless",
            "WLR_LIBINPUT_NO_DEVICES": "1",
            "WLR_RENDERER": "pixman",
            "GLISSANDO_TK_EXTENT": f"{OUT_W}x{OUT_H}",
        })
        voice_setup(self.env)
        if desktop_settings:
            self.use_desktop_settings()
        if config:
            os.makedirs(self.env["XDG_CONFIG_HOME"] + "/glissando", exist_ok=True)
            with open(self.env["XDG_CONFIG_HOME"] + "/glissando/config.toml", "w") as f:
                f.write(config)
        self.procs = []
        prof = "release" if args.release else "debug"
        self.kb_bin = os.path.join(ROOT, "target", prof, "glissando")
        self.tk_bin = os.path.join(ROOT, "target", prof, "glissando-testkit")
        self.geom_file = os.path.join(self.tmp, "geom.json")
        self.state_file = os.path.join(self.tmp, "state.json")
        conf = os.path.join(ROOT, "tools/devenv/sway-test.conf")
        if LANDSCAPE:
            conf = os.path.join(self.tmp, "sway.conf")
            with open(conf, "w") as f:
                f.write("output * resolution 1080x2340 scale 3 transform 90 bg #202124 solid_color\ndefault_border none\nfocus_follows_mouse no\n")
        self.spawn("sway", [SWAY, "-c", conf])
        for _ in range(100):
            socks = [s for s in os.listdir(rt) if s.startswith("wayland-") and not s.endswith(".lock")]
            if socks:
                self.env["WAYLAND_DISPLAY"] = socks[0]
                break
            time.sleep(0.05)
        else:
            raise RuntimeError("sway did not start: " + self.log("sway"))
        if dbus:
            # A private session bus for sm.puri.OSK0.
            bus = os.path.join(rt, "bus")
            self.spawn("dbus", ["dbus-daemon", "--session", "--nofork", "--address=unix:path=" + bus])
            self.env["DBUS_SESSION_BUS_ADDRESS"] = "unix:path=" + bus
            self.wait(lambda: os.path.exists(bus), "session bus")
        if session_manager:
            self.spawn("session-manager", [sys.executable, "-I", os.path.join(ROOT, "tests/e2e/fake_session_manager.py")])
            self.wait(lambda: "ready" in self.log("session-manager"), "fake session manager")
            self.env["DESKTOP_AUTOSTART_ID"] = "e2e-startup-id"
        self.spawn("kb", [self.kb_bin, *([] if dbus else ["--no-dbus"]), "--dump-geometry", self.geom_file, *kb_args],
                   extra_env={"GLISSANDO_DEBUG": "1",
                              **({"GLISSANDO_DEBUG_INPUT": "1"} if os.environ.get("E2E_INPUT") else {}),
                              **({"LD_PRELOAD": os.environ["E2E_KB_PRELOAD"]} if os.environ.get("E2E_KB_PRELOAD") else {})})
        time.sleep(0.4)
        tc = [self.tk_bin, "textclient", "--state", self.state_file]
        if purpose:
            tc += ["--purpose", purpose]
        if hint:
            tc += ["--hint", hint]
        if toolkit:
            tc += ["--toolkit", toolkit]
        if app_id:
            tc += ["--app-id", app_id]
        if text:
            tc += ["--text", text]
        self.spawn("tc", tc)
        if not expect_keyboard:
            return
        self.wait(lambda: os.path.exists(self.geom_file) and self.surface_height() > 0, "keyboard surface")
        time.sleep(0.3)

    def use_desktop_settings(self):
        """Desktop settings the test can change, without dconf or the
        schemas of the developer's system. Follows them instead of forcing
        the keyboard on."""
        schemas = os.path.join(self.tmp, "schemas")
        os.mkdir(schemas)
        with open(os.path.join(schemas, "e2e.gschema.xml"), "w") as f:
            f.write(DESKTOP_SCHEMAS)
        subprocess.run(["glib-compile-schemas", schemas], check=True)
        del self.env["GLISSANDO_AUTOSHOW"]
        self.env.update({"GSETTINGS_SCHEMA_DIR": schemas, "GSETTINGS_BACKEND": "keyfile", "GIO_USE_VFS": "local"})

    def gsettings(self, schema, key, value):
        subprocess.run(["gsettings", "set", schema, key, value], env=self.env, check=True, timeout=5)

    def spawn(self, name, cmd, extra_env=None):
        env = dict(self.env)
        env.update(extra_env or {})
        log = open(os.path.join(self.tmp, name + ".log"), "w")
        p = subprocess.Popen(cmd, env=env, stdout=log, stderr=subprocess.STDOUT)
        self.procs.append((name, p))
        return p

    def log(self, name):
        try:
            return open(os.path.join(self.tmp, name + ".log")).read()
        except OSError:
            return ""

    def wait(self, cond, what, timeout=6.0):
        t = time.time()
        while time.time() - t < timeout:
            try:
                if cond():
                    return
            except (OSError, ValueError):
                pass
            time.sleep(0.05)
        raise AssertionError(f"timeout waiting for {what}\n--- kb log ---\n{self.log('kb')[-2000:]}")

    def geometry(self):
        with open(self.geom_file) as f:
            return json.load(f)

    def surface_height(self):
        for line in self.log("kb").splitlines()[::-1]:
            if "surface created, height" in line:
                return int(line.rsplit(" ", 1)[1])
        return 0

    def kb_height(self):
        g = self.geometry()
        return max(k["y"] + k["h"] / 2 for k in g) + 5

    def key_pos(self, name, nth=0):
        g = self.geometry()
        matches = [k for k in g if k["key"] == name]
        if not matches:
            raise KeyError(f"no key {name!r} in {[k['key'] for k in g]}")
        k = matches[nth]
        top = OUT_H - self.current_height()
        return k["x"], top + k["y"]

    def current_height(self):
        h = 0
        for line in self.log("kb").splitlines():
            if line.startswith("glissando: configure "):
                dims = line.split()[-1]
                hh = int(dims.split("x")[1])
                if hh:
                    h = hh
            if "surface created, height" in line:
                h = int(line.rsplit(" ", 1)[1])
        return h

    def state(self):
        with open(self.state_file) as f:
            return json.load(f)

    def pointer_async(self, script):
        """Send a script to the persistent virtual pointer; returns a waiter."""
        if not hasattr(self, "ptr"):
            self.ptr = subprocess.Popen([self.tk_bin, "pointer-server"], env=self.env, stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, text=True)
            self.procs.append(("pointer", self.ptr))
            time.sleep(0.2)
        self.ptr.stdin.write(script.replace("\n", " ") + "\n")
        self.ptr.stdin.flush()
        return lambda: self.ptr.stdout.readline()

    def pointer(self, script):
        assert self.pointer_async(script)().strip() == "ok"
        time.sleep(0.15)

    def tap(self, key, nth=0):
        x, y = self.key_pos(key, nth)
        self.pointer(f"tap {x} {y}")

    def type(self, text):
        """Tap the keys for `text` (lowercase letters, space, punctuation)."""
        for ch in text:
            name = {" ": "{space}", "\n": "{enter}", "\b": "{backspace}"}.get(ch, ch)
            self.tap(name)

    def screenshot(self, name):
        if self.args.shots:
            os.makedirs(self.args.shots, exist_ok=True)
            subprocess.run([GRIM, os.path.join(self.args.shots, name + ".png")], env=self.env, timeout=10)

    def close(self):
        for name, p in reversed(self.procs):
            p.terminate()
            try:
                p.wait(timeout=3)
            except subprocess.TimeoutExpired:
                p.kill()
        if not self.args.keep:
            shutil.rmtree(self.tmp, ignore_errors=True)
        else:
            print("kept", self.tmp)


TESTS = []


def test(fn):
    TESTS.append(fn)
    return fn


@test
def keyboard_appears_on_focus(s):
    assert s.current_height() > 150, s.current_height()
    names = [k["key"] for k in s.geometry()]
    assert "q" in names and "{space}" in names, names
    s.screenshot("appears")


@test
def typing_letters(s):
    s.type("hello")
    s.wait(lambda: (s.state()["text"] + s.state()["preedit"]) == "Hello", "Hello typed")
    s.type(" ")
    s.wait(lambda: s.state()["text"] == "Hello " and s.state()["preedit"] == "", "committed with space")
    s.screenshot("typed")


@test
def backspace_and_symbols(s):
    s.type("ab")
    s.tap("{backspace}")
    s.wait(lambda: s.state()["preedit"] == "A", "backspace in composing")
    s.tap("{layer=symbols}")
    s.wait(lambda: any(k["key"] == "@" for k in s.geometry()), "symbols layer")
    s.tap("@")
    s.wait(lambda: s.state()["text"] == "A@", "symbol committed")
    s.screenshot("symbols")


def click_moves_cursor_to_start(s):
    s.type("good hel")
    s.wait(lambda: s.state()["text"] == "Good " and s.state()["preedit"] == "hel", "word being composed")
    s.pointer("tap 60 40")  # click into the text field
    s.wait(lambda: s.state()["text"] == "Good hel" and s.state()["cursor"] == 0, "preedit committed, cursor moved")
    s.type("x ")
    # Before the fix "hel" continued at the new position ("helx ...").
    s.wait(lambda: s.state()["text"] == "X Good hel", "new word at the new position")


@test
def editing_with_cursor_moves(s):
    click_moves_cursor_to_start(s)


@test
def editing_with_cursor_moves_qt(s):
    click_moves_cursor_to_start(s)
editing_with_cursor_moves_qt.toolkit = "qt"


@test
def typing_over_a_selected_search(s):
    # GTK deletes the selected text as the preedit starts; before the fix
    # the second letter replaced the first.
    s.type("ma")
    s.wait(lambda: s.state()["text"] == "" and s.state()["preedit"].lower() == "ma", "both letters composed")
    s.type("nn ")
    s.wait(lambda: s.state()["text"].lower() == "mann ", "word committed")
typing_over_a_selected_search.text = "ZI-Klinik"


@test
def terminal_keys(s):
    s.wait(lambda: any(k["key"] == "{ctrl}" for k in s.geometry()), "terminal row")
    s.type("ls")
    s.tap("{ctrl}")
    s.screenshot("terminal-ctrl")
    s.tap("c")
    s.tap("{alt}")
    s.tap("{left}")
    s.tap("{escape}")
    s.wait(lambda: s.state()["text"] == "ls<C-c><A-Left><Esc>", "keys with modifiers")
    s.screenshot("terminal")
terminal_keys.purpose = "terminal"


@test
def no_auto_show_in_listed_apps(s):
    pass


def run_no_auto_show(args):
    s = Session(args, config='no_auto_show = ["org.example.Calc"]\n', app_id="org.example.Calc", expect_keyboard=False)
    try:
        s.wait(lambda: 'active app Some("org.example.Calc")' in s.log("kb") and "activate" in s.log("kb"),
               "focused field in the listed app")
        time.sleep(0.5)
        assert "surface created" not in s.log("kb"), "keyboard opened in a listed app"
    finally:
        s.close()


@test
def home_bar_toggle(s):
    pass


def shown(s):
    events = [l for l in s.log("kb").splitlines() if "surface created" in l or "surface destroyed" in l]
    return bool(events) and "created" in events[-1]


def run_home_bar_toggle(args):
    # Phosh's home bar long press calls SetVisible(!Visible) with Visible
    # taken from its proxy's cache, which follows PropertiesChanged.
    from gi.repository import Gio, GLib
    s = Session(args, dbus=True)
    try:
        os.environ["DBUS_SESSION_BUS_ADDRESS"] = s.env["DBUS_SESSION_BUS_ADDRESS"]
        bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
        s.wait(lambda: bus.call_sync("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus",
                                     "NameHasOwner", GLib.Variant("(s)", ("sm.puri.OSK0",)), None, 0, -1,
                                     None).unpack()[0], "sm.puri.OSK0 on the bus")
        proxy = Gio.DBusProxy.new_sync(bus, Gio.DBusProxyFlags.NONE, None, "sm.puri.OSK0", "/sm/puri/OSK0",
                                       "sm.puri.OSK0", None)
        ctx = GLib.MainContext.default()

        def cached():
            while ctx.iteration(False):
                pass
            v = proxy.get_cached_property("Visible")
            return v.unpack() if v is not None else None

        # Shown on focus; before the fix the proxy never saw it and the long
        # press only ever asked to show.
        s.wait(lambda: cached() is True, "Visible after focus in the proxy")
        for want in (False, True, False):
            proxy.call_sync("SetVisible", GLib.Variant("(b)", (not cached(),)), Gio.DBusCallFlags.NONE, -1, None)
            s.wait(lambda: cached() == want, f"Visible {want} in the proxy")
            s.wait(lambda: shown(s) == want, f"keyboard shown={want}")
    finally:
        os.environ.pop("DBUS_SESSION_BUS_ADDRESS", None)
        s.close()


@test
def session_manager_end_of_session(s):
    pass  # see run_session_manager


def run_session_manager(args):
    s = Session(args, dbus=True, session_manager=True, config="autocorrect = false\n")
    try:
        def calls():
            return s.log("session-manager").splitlines()

        def emit(signal):
            subprocess.run(["gdbus", "call", "--session", "--dest", "org.gnome.SessionManager",
                            "--object-path", "/org/gnome/SessionManager",
                            "--method", "org.gnome.SessionManager.E2E.Emit", signal],
                           env=s.env, check=True, timeout=5, stdout=subprocess.DEVNULL)

        s.wait(lambda: "RegisterClient sm.puri.OSK0 e2e-startup-id owned=True" in calls(), "registration")
        emit("QueryEndSession")
        s.wait(lambda: calls().count("EndSessionResponse True ") == 1, "answer to QueryEndSession")
        emit("EndSession")
        s.wait(lambda: calls().count("EndSessionResponse True ") == 2, "answer to EndSession")

        s.type("blorp ")
        s.wait(lambda: s.state()["text"] == "Blorp ", "typed word")
        kb = dict(s.procs)["kb"]
        assert kb.poll() is None, "the keyboard must keep running until Stop"
        emit("Stop")
        s.wait(lambda: kb.poll() is not None, "exit on Stop")
        assert kb.returncode == 0, f"exit status {kb.returncode}"
        user = pathlib.Path(s.env["XDG_DATA_HOME"], "glissando", "user")
        learned = "".join(f.read_text() for f in user.glob("*.tsv"))
        assert "\tblorp\t" in learned.lower(), f"learned word lost on Stop: {learned!r}"
    finally:
        s.close()


@test
def follows_screen_keyboard_enabled(s):
    s.gsettings(A11Y, "screen-keyboard-enabled", "false")
    s.wait(lambda: not shown(s), "hidden while disabled")
    s.gsettings(A11Y, "screen-keyboard-enabled", "true")
    s.wait(lambda: shown(s), "shown again once enabled")
follows_screen_keyboard_enabled.desktop_settings = True


@test
def follows_system_colour_scheme(s):
    s.gsettings("org.gnome.desktop.interface", "color-scheme", "prefer-dark")
    s.wait(lambda: "system theme dark" in s.log("kb"), "dark theme")
    s.gsettings("org.gnome.desktop.interface", "color-scheme", "default")
    s.wait(lambda: "system theme light" in s.log("kb"), "light theme")
follows_system_colour_scheme.desktop_settings = True


@test
def fixed_theme_ignores_system_colour_scheme(s):
    s.gsettings("org.gnome.desktop.interface", "color-scheme", "prefer-dark")
    # The accessibility key is watched the same way and proves the settings
    # were delivered.
    s.gsettings(A11Y, "screen-keyboard-enabled", "false")
    s.wait(lambda: not shown(s), "settings delivered")
    assert "system theme" not in s.log("kb")
fixed_theme_ignores_system_colour_scheme.config = 'theme = "light"\n'
fixed_theme_ignores_system_colour_scheme.desktop_settings = True


@test
def glide_typing(s):
    pts = []
    for ch in "hello":
        x, y = s.key_pos(ch)
        pts += [x, y]
    pts_s = " ".join(f"{v:.1f}" for v in pts)
    s.pointer(f"swipe 450 {pts_s}")
    s.wait(lambda: (s.state()["text"] + s.state()["preedit"]).lower() == "hello", "glided word", timeout=8)
    s.screenshot("glide")


@test
def password_field_is_private(s):
    pass


def run_password(args):
    s = Session(args, purpose="password")
    try:
        s.type("helo ")
        s.wait(lambda: s.state()["text"] == "helo ", "password typed verbatim")
        assert s.state()["preedit"] == ""
        user_dir = os.path.join(s.env["XDG_DATA_HOME"], "glissando", "user")
        time.sleep(0.3)
        assert not os.path.exists(user_dir) or all(os.path.getsize(os.path.join(user_dir, f)) == 0 for f in os.listdir(user_dir))
    finally:
        s.close()


@test
def space_cursor_and_glide_delete(s):
    s.type("we go ")
    s.wait(lambda: s.state()["text"] == "We go ", "typed")
    x, y = s.key_pos("{space}")
    s.pointer(f"swipe 300 {x} {y} {x-60} {y}")
    s.wait(lambda: s.state()["cursor"] < 6, "cursor moved left")
    x, y = s.key_pos("{space}")
    s.pointer(f"swipe 300 {x-60} {y} {x+140} {y}")
    s.wait(lambda: s.state()["cursor"] == 6, "cursor back at end")
    bx, by = s.key_pos("{backspace}")
    s.pointer(f"swipe 300 {bx} {by} {bx-40} {by}")
    s.wait(lambda: s.state()["text"] == "We ", "glide delete removed a word")


MULTI = 'layouts = ["en", "ru", "de"]\n'


def multi(fn):
    fn.config = MULTI
    return fn


@test
def long_press_popup(s):
    x, y = s.key_pos("e")
    kw = next(k["w"] for k in s.geometry() if k["key"] == "e")
    done = s.pointer_async(f"down {x} {y}; sleep 900; move {x+kw} {y-40}; sleep 400; up")
    time.sleep(0.8)
    s.screenshot("longpress-popup")
    time.sleep(0.5)
    s.screenshot("longpress-select")
    done()
    s.wait(lambda: s.state()["preedit"] in ("È", "É", "Ê"), "popup entry committed")
    s.screenshot("longpress-result")


@test
@multi
def switch_to_russian_and_type(s):
    assert any(k["key"] == "{lang}" for k in s.geometry()), "globe key with several layouts"
    s.tap("{lang}")
    s.wait(lambda: any(k["key"] == "й" for k in s.geometry()), "russian layout")
    s.type("привет ")
    s.wait(lambda: s.state()["text"] == "Привет ", "russian text")
    s.screenshot("russian")


@test
@multi
def multilingual_latin(s):
    # English layout with German enabled: German words are recognised.
    s.type("danke ")
    s.wait(lambda: s.state()["text"] == "Danke ", "german word on english layout")


@test
def emoji_panel(s):
    s.tap("{emoji}")
    time.sleep(0.4)
    s.screenshot("emoji")
    # First emoji cell sits at the top left of the panel area.
    top = OUT_H - s.current_height()
    s.pointer(f"tap 22 {top + 44 + 26}")
    s.wait(lambda: len(s.state()["text"]) > 0 and not s.state()["text"].isascii(), "emoji committed")


@test
def emoji_search(s):
    s.tap("{emoji}")
    time.sleep(0.4)
    s.pointer(f"tap 72 {OUT_H - 22}")  # search icon in the emoji footer
    s.wait(lambda: any(k["key"] == "p" for k in s.geometry()), "letters for the query")
    s.type("pizza")
    time.sleep(0.3)
    s.screenshot("emoji-search")
    top = OUT_H - s.current_height()
    s.pointer(f"tap {44 + 116 + 22} {top + 22}")  # first result
    s.wait(lambda: "\U0001F355" in s.state()["text"], "pizza emoji inserted")


@test
def emoji_suggestion_while_typing(s):
    s.type("pizza")
    time.sleep(0.3)
    top = OUT_H - s.current_height()
    s.pointer(f"tap {OUT_W - 24} {top + 22}")  # emoji slot at the right of the strip
    s.wait(lambda: s.state()["text"].endswith("\U0001F355"), "word followed by emoji")
    assert s.state()["text"].lower().startswith("pizza "), s.state()


@test
def quick_menu_number_row(s):
    top = OUT_H - s.current_height()
    s.pointer(f"tap 29 {top + 22}")  # menu button in the toolbar
    time.sleep(0.3)
    s.screenshot("menu")
    # Number row toggle: third button of the first menu row.
    w = OUT_W / 3
    s.pointer(f"tap {2 * w + w / 2} {top + 44 + 30}")
    time.sleep(0.3)
    s.pointer(f"tap 36 {OUT_H - 22}")  # ABC in the footer
    s.wait(lambda: sum(1 for k in s.geometry() if k["key"] in "1234567890") >= 10, "number row shown")
    s.screenshot("numrow")


@test
def typing_without_emoji_suggestions(s):
    s.type("hello")
    s.wait(lambda: (s.state()["text"] + s.state()["preedit"]) == "Hello", "Hello typed")
typing_without_emoji_suggestions.config = "emoji_suggestions = false\n"


VOICE_ENGINE = os.path.join(ROOT, "build/whisper-host/bin")
VOICE_MODEL = os.path.join(ROOT, ".devtools/voice/ggml-base-q8_0.bin")
VOICE_INPUT = os.path.join(ROOT, ".devtools/voice/jfk.raw")


@test
def voice_input(s):
    if not (os.path.exists(VOICE_MODEL) and os.path.exists(VOICE_INPUT) and os.path.isdir(VOICE_ENGINE)):
        print("  (skipped: run packaging/voice/build-whisper-host.sh and fetch the model)")
        return
    top = OUT_H - s.current_height()
    s.pointer(f"tap {OUT_W - 79} {top + 22}")  # microphone in the toolbar
    time.sleep(0.5)
    s.screenshot("voice-listening")
    s.wait(lambda: "fellow Americans" in s.state()["text"], "transcript inserted", timeout=60)


def voice_setup(session_env):
    voice_dir = os.path.join(session_env["XDG_DATA_HOME"], "glissando", "voice")
    os.makedirs(voice_dir, exist_ok=True)
    if os.path.exists(VOICE_MODEL) and not os.path.exists(os.path.join(voice_dir, "model.bin")):
        os.symlink(VOICE_MODEL, os.path.join(voice_dir, "model.bin"))
    session_env["PATH"] = VOICE_ENGINE + ":" + os.path.join(ROOT, "target", "debug") + ":" + session_env.get("PATH", "")
    session_env["GLISSANDO_VOICE_TEST_INPUT"] = VOICE_INPUT


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("-k", default="")
    ap.add_argument("--keep", action="store_true")
    ap.add_argument("--shots", default=None)
    ap.add_argument("--release", action="store_true")
    args = ap.parse_args()
    if not os.path.exists(SWAY):
        print("sway not found; run tools/devenv/fetch-sway.sh", file=sys.stderr)
        return 2
    failed = 0
    for t in TESTS:
        if args.k not in t.__name__:
            continue
        start = time.time()
        try:
            if t.__name__ == "password_field_is_private":
                run_password(args)
            elif t.__name__ == "no_auto_show_in_listed_apps":
                run_no_auto_show(args)
            elif t.__name__ == "home_bar_toggle":
                run_home_bar_toggle(args)
            elif t.__name__ == "session_manager_end_of_session":
                run_session_manager(args)
            else:
                s = Session(args, config=getattr(t, "config", None), toolkit=getattr(t, "toolkit", None),
                            purpose=getattr(t, "purpose", None), text=getattr(t, "text", None),
                            desktop_settings=getattr(t, "desktop_settings", False))
                try:
                    t(s)
                finally:
                    s.close()
            print(f"PASS {t.__name__} ({time.time()-start:.1f}s)")
        except Exception:
            failed += 1
            print(f"FAIL {t.__name__}")
            traceback.print_exc()
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
