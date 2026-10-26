# Pinned GPUI Linux visibility patch

This is gpui-pre-linux 0.3.9 from crates.io, unchanged except for the files
listed in `visibility.patch` (against that exact registry release). The Apache
license is retained. No dependency versions were upgraded. Registry archive
SHA256: `e28213c3fde6f13e91ba0b74b90cbaadec9373e36a031eda98561d31059870f3`.

## Why this directory is versioned

The root `[patch.crates-io]` points Cargo at this directory, including when
resolving dependencies for CI or a fresh checkout. The complete source tree,
manifest and license must be included with Grajek. Do not ignore or delete this
directory while the path override exists. `visibility.patch` is a review and
maintenance artifact; Cargo does not apply it or reconstruct missing sources.

The large initial diff is mostly a copy of one upstream crate. For v0.5.0,
21 registry-origin files are byte-identical and eight source files are modified.
The additional files are this note and the focused patch. Review our backend
changes through `visibility.patch` and their caller/native-test context.

## Runtime contract

Grajek needs native Hide and restore while keeping the GPUI window and its
renderer alive. The pinned public GPUI API exposes only minimize/activate.
`LinuxPlatform::set_window_visible` runs on the existing foreground thread and
looks up the existing window by GPUI handle; it never creates a window.

X11 withdraws/maps the existing X window. Wayland detaches the surface buffer
and suppresses rendering while hidden. Show remaps the same inactive surface,
waits for configure before presenting, and reapplies title/app ID/minimum size/GPU maximum size
and maximized/fullscreen requests. This clears compositor minimization without
depending on activation permission. A visible inactive Wayland window can also
be remapped because xdg-shell has no portable minimized-state query. KWin may
choose a new placement on remap; focus remains compositor-controlled.

## Maintenance and verification

- Edit the vendored source in place, then regenerate `visibility.patch` as a
  unified diff against the exact registry 0.3.9 baseline named above, with paths
  relative to this directory (`a/src/...`, `b/src/...`). Do not patch Cargo's
  global registry cache or mass-format unchanged upstream files.
- From Grajek's root, apply the patch to a disposable baseline copy with
  `patch --batch --fuzz=0 -p1 -d /path/to/disposable-baseline < vendor/gpui-pre-linux/visibility.patch`.
  Check that it reproduces all vendored registry-origin files exactly. Keep
  upstream license/provenance intact; update them if deliberately changing the
  baseline in a separate dependency-update task.
- Run the root Cargo formatting, strict Clippy, test and build checks documented
  in Grajek's README. After building, run both commands below in a real Plasma
  session (requires python-dbus, PyGObject and `xdotool` for XWayland):

  ```sh
  python3 scripts/check-tray-linux.py wayland
  python3 scripts/check-tray-linux.py x11
  ```

  The fixtures cover mapping, minimized restore, caption and native identity,
  queue persistence, Close/Quit, host loss and tray/MPRIS teardown. They use an
  invalid token and do not prove live audio continuity or native Windows behavior.

Retire this copy when a chosen upstream version exposes equivalent supported
APIs and passes the native checks. Change `src/window.rs`, the root Cargo
dependency/patch configuration and lockfile together before removing the
directory. Moving the patch to a fork pinned to an immutable commit is another
possible future migration; it still requires an available, maintained remote.
