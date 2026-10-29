#!/usr/bin/env python3
"""Native KDE test: python3 scripts/check-tray-linux.py [wayland|x11].

Requires a running Plasma session, python-dbus and PyGObject. Uses only its own
Grajek process and temporary state; no owner credential is read. Build first.
"""
import json
import os
import re
from pathlib import Path
import subprocess
import sys
import tempfile
import time

import dbus
import dbus.mainloop.glib
import dbus.service
from gi.repository import GLib

MODE = sys.argv[1] if len(sys.argv) > 1 else 'wayland'
assert MODE in ('wayland', 'x11')
BINARY = Path(__file__).resolve().parents[1] / 'target/debug/grajek'
dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)
bus = dbus.SessionBus()
name = dbus.service.BusName('org.grajek.NativeProbe', bus, do_not_queue=True)


class Probe(dbus.service.Object):
    result = None

    @dbus.service.method('org.grajek.NativeProbe', in_signature='s', out_signature='')
    def Report(self, data):
        self.result = json.loads(data)
        self.loop.quit()


probe = Probe(bus, '/Probe')
kwin = dbus.Interface(bus.get_object('org.kde.KWin', '/Scripting'), 'org.kde.kwin.Scripting')


def window_state(pid, action=''):
    with tempfile.NamedTemporaryFile(mode='w', suffix='.js') as script:
        script.write('const windows = workspace.windowList().filter(w => w.pid === ' + str(pid) + ');\n'
                     + action + '\ncallDBus("org.grajek.NativeProbe", "/Probe", "org.grajek.NativeProbe", "Report", '
                     'JSON.stringify(windows.map(w => ({id:String(w.internalId), minimized:w.minimized, '
                     'hidden:w.hidden, active:w.active, caption:w.caption}))));')
        script.flush()
        plugin = 'grajek-native-probe'
        idx = bus.call_blocking('org.kde.KWin', '/Scripting', 'org.kde.kwin.Scripting',
                                'loadScript', 'ss', (script.name, plugin))
        assert idx >= 0, 'KWin refused the test script'
        probe.result = None
        probe.loop = GLib.MainLoop()
        timer = GLib.timeout_add(3000, lambda: probe.loop.quit() or False)
        try:
            dbus.Interface(bus.get_object('org.kde.KWin', f'/Scripting/Script{idx}'),
                           'org.kde.kwin.Script').run()
            probe.loop.run()
            assert probe.result is not None, 'KWin probe timed out'
            return probe.result
        finally:
            GLib.source_remove(timer)
            kwin.unloadScript(plugin)


def wait_for(check, label):
    deadline = time.monotonic() + 8
    while time.monotonic() < deadline:
        context = GLib.MainContext.default()
        while context.pending():
            context.iteration(False)
        result = check()
        if result:
            return result
        time.sleep(0.1)
    raise AssertionError(label)


def tray_services(pid):
    props = dbus.Interface(bus.get_object('org.kde.StatusNotifierWatcher', '/StatusNotifierWatcher'),
                           'org.freedesktop.DBus.Properties')
    items = props.Get('org.kde.StatusNotifierWatcher', 'RegisteredStatusNotifierItems')
    return [str(item).split('/')[0] for item in items
            if dbus.Interface(bus.get_object('org.freedesktop.DBus', '/org/freedesktop/DBus'), 'org.freedesktop.DBus').GetConnectionUnixProcessID(str(item).split('/')[0]) == pid]


def visible(pid):
    windows = window_state(pid)
    assert len(windows) <= 1, f'Duplicate windows: {windows}'
    if windows and not windows[0]['minimized'] and not windows[0]['hidden']:
        # KWin can expose a new mapping before its title update is applied.
        return windows if windows[0]['caption'] == 'Grajek' else None
    return None


def hidden(pid):
    windows = window_state(pid)
    assert len(windows) <= 1, f'Duplicate windows: {windows}'
    return not windows or windows[0]['hidden']


def run_case(close_to_tray):
    with tempfile.TemporaryDirectory(prefix='grajek-native-') as temp:
        root = Path(temp)
        data = root / 'grajek'
        data.mkdir()
        queue = [{'url': 'https://www.youtube.com/watch?v=dQw4w9WgXcQ', 'title': 'Pending fixture'}]
        (data / 'state.json').write_text(json.dumps({'close_to_tray': close_to_tray,
                                                    'remember_queue': True, 'queue': queue}))
        env = os.environ.copy()
        for key in ('DISCORD_BOT_TOKEN', 'DISCORD_GUILD_ID', 'DISCORD_VOICE_CHANNEL_ID'):
            env.pop(key, None)
        # Nonempty invalid token bypasses OS credential reads; Serenity rejects it locally.
        env.update(DISCORD_BOT_TOKEN='native-tray-test-invalid', XDG_DATA_HOME=temp,
                   WAYLAND_DEBUG='1' if MODE == 'wayland' else '0')
        if MODE == 'x11':
            env.pop('WAYLAND_DISPLAY', None)
            env.pop('WAYLAND_SOCKET', None)
        log_path = Path('/tmp') / f'grajek-recovery-{MODE}-{int(close_to_tray)}.log'
        with log_path.open('w') as log:
            process = subprocess.Popen([str(BINARY)], cwd=temp, env=env, stdout=log, stderr=log)
            try:
                wait_for(lambda: visible(process.pid), 'Initial window not visible')
                original_xids = subprocess.check_output(['xdotool', 'search', '--pid', str(process.pid), '--name', 'Grajek']).split() if MODE == 'x11' else None
                service = wait_for(lambda: tray_services(process.pid), 'Tray did not register')[0]
                menu = dbus.Interface(bus.get_object(service, '/Menu'), 'com.canonical.dbusmenu')
                def choose(item):
                    menu.Event(dbus.Int32(item), 'clicked', dbus.Int32(0, variant_level=1), dbus.UInt32(0))
                def show():
                    choose(1)
                    windows = wait_for(lambda: visible(process.pid), 'Show left window minimized/hidden or lost its title')
                    if MODE == 'x11':
                        xids = subprocess.check_output(['xdotool', 'search', '--pid', str(process.pid), '--name', 'Grajek']).split()
                        assert xids == original_xids and len(xids) == 1, 'Native X window replaced/duplicated'
                    assert len(tray_services(process.pid)) == 1, 'Duplicate tray registration'
                for _ in range(3):
                    choose(2)
                    wait_for(lambda: hidden(process.pid), 'Hide did not unmap the window')
                    assert process.poll() is None, 'Hide terminated Grajek'
                    status = dbus.Interface(bus.get_object('org.mpris.MediaPlayer2.grajek', '/org/mpris/MediaPlayer2'),
                                            'org.freedesktop.DBus.Properties').Get('org.mpris.MediaPlayer2.Player', 'PlaybackStatus')
                    assert status == 'Stopped', status
                    show()
                # Burst requests exercise configure ordering without waiting between actions.
                for item in (1, 1, 2, 1, 1, 2, 1):
                    choose(item)
                show()
                window_state(process.pid, 'windows.forEach(w => w.minimized = true);')
                wait_for(lambda: window_state(process.pid)[0]['minimized'], 'Minimize did not take effect')
                show()
                # Icon activation uses the same restore path as menu Show.
                choose(2)
                wait_for(lambda: hidden(process.pid), 'Hide before activation failed')
                dbus.Interface(bus.get_object(service, '/StatusNotifierItem'), 'org.kde.StatusNotifierItem').Activate(0, 0)
                wait_for(lambda: visible(process.pid), 'Icon activation did not restore')
                window_state(process.pid, 'windows.forEach(w => w.closeWindow());')
                if close_to_tray:
                    wait_for(lambda: hidden(process.pid), 'Close-to-tray did not hide')
                    assert process.poll() is None
                    show()
                    choose(2)
                    wait_for(lambda: hidden(process.pid), 'Hide before Quit failed')
                    choose(7)
                assert process.wait(timeout=12) == 0, 'Quit/Close failed'
                wait_for(lambda: not bus.name_has_owner(service), 'Tray registration leaked')
                wait_for(lambda: not bus.name_has_owner('org.mpris.MediaPlayer2.grajek'), 'MPRIS registration leaked')
                saved = json.loads((data / 'state.json').read_text())
                assert saved['queue'] == queue, 'Visibility/shutdown changed the pending queue'
                assert saved['close_to_tray'] == close_to_tray
                if MODE == 'wayland':
                    # The same wl_surface and xdg_toplevel survive every remap.
                    protocol = log_path.read_text()
                    surfaces = re.findall(r'get_xdg_surface\(new id xdg_surface#\d+, (wl_surface#\d+)\)', protocol)
                    assert len(surfaces) == 1, 'Native application surface recreated'
                    assert protocol.count('.get_toplevel(new id xdg_toplevel') == 1, 'Native toplevel recreated'
                print(f'PASS {MODE} close_to_tray={close_to_tray}: Hide/Show x3, minimize restore, icon activation, Close, Quit, queue, tray/MPRIS cleanup', flush=True)
            finally:
                if process.poll() is None:
                    process.terminate()
                    process.wait(timeout=5)


class Host(dbus.service.Object):
    registered = None

    @dbus.service.method('org.freedesktop.DBus.Properties', in_signature='ss', out_signature='v')
    def Get(self, interface, property_name):
        assert interface == 'org.kde.StatusNotifierWatcher'
        assert property_name == 'IsStatusNotifierHostRegistered'
        return dbus.Boolean(True, variant_level=1)

    @dbus.service.method('org.freedesktop.DBus.Properties', in_signature='s', out_signature='a{sv}')
    def GetAll(self, interface):
        assert interface == 'org.kde.StatusNotifierWatcher'
        return {'IsStatusNotifierHostRegistered': dbus.Boolean(True)}

    @dbus.service.method('org.kde.StatusNotifierWatcher', in_signature='s', out_signature='', sender_keyword='sender')
    def RegisterStatusNotifierItem(self, service, sender):
        self.registered = sender


def run_unavailable_case(disappear):
    # Private bus: never stop or alter the real Plasma tray host.
    config = tempfile.NamedTemporaryFile(mode='w', suffix='.conf')
    config.write('<busconfig><type>session</type><listen>unix:tmpdir=/tmp</listen><auth>EXTERNAL</auth>'
                 '<policy context="default"><allow send_destination="*"/><allow receive_sender="*"/><allow own="*"/></policy></busconfig>')
    config.flush()
    daemon = subprocess.Popen(['dbus-daemon', '--config-file=' + config.name, '--nofork', '--print-address=1'], stdout=subprocess.PIPE, text=True)
    private = host = host_name = None
    try:
        address = daemon.stdout.readline().strip()
        private = dbus.bus.BusConnection(address)
        if disappear:
            host_name = dbus.service.BusName('org.kde.StatusNotifierWatcher', private)
            host = Host(private, '/StatusNotifierWatcher')
        with tempfile.TemporaryDirectory(prefix='grajek-native-fallback-') as temp:
            root = Path(temp) / 'grajek'
            root.mkdir()
            (root / 'state.json').write_text(json.dumps({'close_to_tray': True}))
            env = os.environ.copy()
            env.update(DBUS_SESSION_BUS_ADDRESS=address, DISCORD_BOT_TOKEN='native-tray-test-invalid', XDG_DATA_HOME=temp)
            for key in ('DISCORD_GUILD_ID', 'DISCORD_VOICE_CHANNEL_ID', 'WAYLAND_DEBUG'):
                env.pop(key, None)
            if MODE == 'wayland':
                env['WAYLAND_DEBUG'] = '1'
            if MODE == 'x11':
                env.pop('WAYLAND_DISPLAY', None)
                env.pop('WAYLAND_SOCKET', None)
            with open(f'/tmp/grajek-recovery-{MODE}-fallback-{int(disappear)}.log', 'w') as log:
                process = subprocess.Popen([str(BINARY)], cwd=temp, env=env, stdout=log, stderr=log)
                try:
                    wait_for(lambda: visible(process.pid), 'Fallback fixture did not open')
                    if disappear:
                        service = wait_for(lambda: host.registered, 'Test host did not register Grajek')
                        menu = dbus.Interface(private.get_object(service, '/Menu'), 'com.canonical.dbusmenu')
                        menu.Event(dbus.Int32(2), 'clicked', dbus.Int32(0, variant_level=1), dbus.UInt32(0))
                        wait_for(lambda: hidden(process.pid), 'Fallback fixture did not hide')
                        private.release_name('org.kde.StatusNotifierWatcher')
                        wait_for(lambda: visible(process.pid), 'Lost tray host stranded a hidden window')
                    window_state(process.pid, 'windows.forEach(w => w.closeWindow());')
                    assert process.wait(timeout=12) == 0, 'Unavailable tray prevented Close from quitting'
                    assert not private.name_has_owner('org.mpris.MediaPlayer2.grajek'), 'Fallback leaked MPRIS'
                    print(f'PASS {MODE} tray_host_disappears={disappear}: restore safety and Close fallback', flush=True)
                finally:
                    if process.poll() is None:
                        process.terminate()
                        process.wait(timeout=5)
    finally:
        if host:
            host.remove_from_connection()
        host_name = None
        if private:
            private.close()
        daemon.terminate()
        daemon.wait(timeout=5)
        daemon.stdout.close()
        config.close()


for preference in (False, True):
    run_case(preference)
for disappears in (False, True):
    run_unavailable_case(disappears)
