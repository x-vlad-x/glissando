#!/usr/bin/env python3
"""Stands in for gnome-session on the private bus of the e2e tests.

Prints every call it receives. The E2E interface lets a test send the client
one of the signals that the real session manager sends at the end of a session.
"""
from gi.repository import Gio, GLib

NAME = "org.gnome.SessionManager"
MANAGER_PATH = "/org/gnome/SessionManager"
CLIENT_PATH = "/org/gnome/SessionManager/Client1"
CLIENT_IFACE = NAME + ".ClientPrivate"
SIGNAL_ARGS = {"QueryEndSession": "(u)", "EndSession": "(u)", "Stop": None}

MANAGER_XML = f"""
<node>
  <interface name="{NAME}">
    <method name="RegisterClient">
      <arg type="s" direction="in"/>
      <arg type="s" direction="in"/>
      <arg type="o" direction="out"/>
    </method>
  </interface>
  <interface name="{NAME}.E2E">
    <method name="Emit">
      <arg type="s" direction="in"/>
    </method>
  </interface>
</node>"""

CLIENT_XML = f"""
<node>
  <interface name="{CLIENT_IFACE}">
    <method name="EndSessionResponse">
      <arg type="b" direction="in"/>
      <arg type="s" direction="in"/>
    </method>
    <signal name="Stop"/>
    <signal name="QueryEndSession"><arg type="u"/></signal>
    <signal name="EndSession"><arg type="u"/></signal>
  </interface>
</node>"""

client = None


def emit(conn, signal):
    args = SIGNAL_ARGS[signal]
    params = GLib.Variant(args, (0,)) if args else None
    conn.emit_signal(client, CLIENT_PATH, CLIENT_IFACE, signal, params)


def on_manager_call(conn, sender, path, iface, method, params, invocation):
    global client
    if method == "Emit":
        emit(conn, params.unpack()[0])
        invocation.return_value(None)
        return
    app_id, startup_id = params.unpack()
    owner = conn.call_sync("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "GetNameOwner",
                           GLib.Variant("(s)", (app_id,)), None, 0, -1, None).unpack()[0]
    client = sender
    print(f"RegisterClient {app_id} {startup_id} owned={owner == sender}", flush=True)
    invocation.return_value(GLib.Variant("(o)", (CLIENT_PATH,)))


def on_client_call(conn, sender, path, iface, method, params, invocation):
    print(method, *params.unpack(), flush=True)
    invocation.return_value(None)


def main():
    def on_bus(conn, _name):
        for iface in Gio.DBusNodeInfo.new_for_xml(MANAGER_XML).interfaces:
            conn.register_object(MANAGER_PATH, iface, on_manager_call)
        for iface in Gio.DBusNodeInfo.new_for_xml(CLIENT_XML).interfaces:
            conn.register_object(CLIENT_PATH, iface, on_client_call)

    Gio.bus_own_name(Gio.BusType.SESSION, NAME, Gio.BusNameOwnerFlags.NONE, on_bus,
                     lambda *_: print("ready", flush=True), None)
    GLib.MainLoop().run()


if __name__ == "__main__":
    main()
