#!/usr/bin/env python3
"""Capture only the offline fixture's KWin window, independent of desktop focus."""

import json
import os
from pathlib import Path
import sys
import tempfile
import time

import dbus
import dbus.service
from dbus.mainloop.glib import DBusGMainLoop
from gi.repository import GLib
from PIL import Image


def main():
    pid, destination, width, height = sys.argv[1:]
    expected = (int(width), int(height))
    DBusGMainLoop(set_as_default=True)
    bus = dbus.SessionBus()
    loop = GLib.MainLoop()
    handle = []

    class FixtureWindow(dbus.service.Object):
        @dbus.service.method("org.grajek.NativeCapture", in_signature="s", out_signature="")
        def found(self, uuid):
            handle.append(str(uuid))
            loop.quit()

    receiver = FixtureWindow(bus, "/GrajekCapture")
    scripting = dbus.Interface(bus.get_object("org.kde.KWin", "/Scripting"), "org.kde.kwin.Scripting")
    plugin = f"grajek-capture-{pid}"
    script = (
        f"const w = workspace.windowList().find(w => w.pid === {int(pid)} "
        '&& w.caption.includes("offline UX evidence")); '
        f"if (w) callDBus({json.dumps(bus.get_unique_name())}, '/GrajekCapture', "
        "'org.grajek.NativeCapture', 'found', String(w.internalId));"
    )
    with tempfile.NamedTemporaryFile(mode="w", suffix=".js") as source:
        source.write(script)
        source.flush()
        index = bus.call_blocking(
            "org.kde.KWin", "/Scripting", "org.kde.kwin.Scripting", "loadScript", "ss", (source.name, plugin)
        )
        try:
            dbus.Interface(
                bus.get_object("org.kde.KWin", f"/Scripting/Script{index}"), "org.kde.kwin.Script"
            ).run()
            timeout = GLib.timeout_add(3000, lambda: (loop.quit(), False)[1])
            loop.run()
            if handle:
                GLib.source_remove(timeout)
        finally:
            scripting.unloadScript(plugin)
            receiver.remove_from_connection()
    if len(handle) != 1:
        raise RuntimeError("Could not identify exactly one fixture window")

    capture = dbus.Interface(
        bus.get_object("org.kde.KWin", "/org/kde/KWin/ScreenShot2"), "org.kde.KWin.ScreenShot2"
    )
    with tempfile.TemporaryFile() as pixels:
        info = capture.CaptureWindow(
            handle[0],
            {"include-decoration": False, "include-shadow": False, "include-cursor": False},
            dbus.types.UnixFd(pixels.fileno()),
            timeout=3,
        )
        actual = (int(info["width"]), int(info["height"]))
        if actual != expected:
            raise RuntimeError(f"Fixture dimensions {actual}, expected {expected}")
        stride = int(info["stride"])
        length = stride * actual[1]
        deadline = time.monotonic() + 3
        while os.fstat(pixels.fileno()).st_size < length and time.monotonic() < deadline:
            time.sleep(0.01)
        pixels.seek(0)
        raw = pixels.read()
    # KWin returns a raw QImage. The fixture is opaque; preserve native RGB pixels.
    formats = {4: "BGRX", 5: "BGRA", 6: "BGRA", 17: "RGBA", 18: "RGBA"}
    mode = formats[int(info["format"])]
    if len(raw) != length:
        raise RuntimeError("Incomplete compositor image")
    image = Image.frombytes("RGBA", actual, raw, "raw", mode, stride)
    image.save(Path(destination))


if __name__ == "__main__":
    main()
