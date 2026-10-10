# Mock org.freedesktop.portal.Settings on a private session bus: color-scheme
# starts at 2 (light), flips to 1 (dark) at +6 s and back to 2 at +12 s.
import sys, time
import dbus, dbus.service, dbus.mainloop.glib
from gi.repository import GLib

dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)
NS, KEY = "org.freedesktop.appearance", "color-scheme"
state = {"v": 2}

class Settings(dbus.service.Object):
    @dbus.service.method("org.freedesktop.portal.Settings", in_signature="as", out_signature="a{sa{sv}}")
    def ReadAll(self, namespaces):
        return {NS: {KEY: dbus.UInt32(state["v"]), "accent-color": dbus.Struct((1.0, 0.35, 0.12), signature="ddd"), "contrast": dbus.UInt32(0)}}
    @dbus.service.method("org.freedesktop.portal.Settings", in_signature="ss", out_signature="v")
    def ReadOne(self, ns, key):
        if ns == NS and key == KEY:
            return dbus.UInt32(state["v"])
        raise dbus.exceptions.DBusException("org.freedesktop.portal.Error.NotFound", "not found")
    @dbus.service.method("org.freedesktop.portal.Settings", in_signature="ss", out_signature="v")
    def Read(self, ns, key):
        return dbus.UInt32(state["v"]) if (ns, key) == (NS, KEY) else dbus.String("")
    @dbus.service.signal("org.freedesktop.portal.Settings", signature="ssv")
    def SettingChanged(self, ns, key, value):
        pass
    @dbus.service.method(dbus.PROPERTIES_IFACE, in_signature="ss", out_signature="v")
    def Get(self, iface, prop):
        return dbus.UInt32(2)
    @dbus.service.method(dbus.PROPERTIES_IFACE, in_signature="s", out_signature="a{sv}")
    def GetAll(self, iface):
        return {"version": dbus.UInt32(2)}

bus = dbus.SessionBus()
name = dbus.service.BusName("org.freedesktop.portal.Desktop", bus)
obj = Settings(bus, "/org/freedesktop/portal/desktop")
t0 = time.time()
def flip(v):
    state["v"] = v
    print(f"[portal] t={time.time()-t0:.1f}s color-scheme={v}", flush=True)
    obj.SettingChanged(NS, KEY, dbus.UInt32(v, variant_level=1))
    return False
GLib.timeout_add(6000, flip, 1)
GLib.timeout_add(12000, flip, 2)
print("[portal] ready", flush=True)
GLib.MainLoop().run()
