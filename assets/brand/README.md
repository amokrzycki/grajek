# Application icons

Primary dark Grajek icons imported from the `grajek-brand` artwork pack (2026-10-27).
The rounded charcoal tile keeps the ivory/brass mark visible on light and dark desktops.

- Windows: `grajek-windows.ico` is embedded by `build.rs` as resource **1**, which
  GPUI uses for the native window icon. Explorer uses the same multiresolution ICO.
- Linux: `make install` installs the PNG sizes and scalable SVG in `hicolor`,
  alongside `org.grajek.desktop.desktop`. Its filename, icon name and
  `StartupWMClass` match the existing GPUI app ID `org.grajek.desktop`.
- Tray: idle uses this app mark; playing, paused and error keep their state artwork.
  All tray PNGs are embedded, so they need no runtime files or icon theme lookup.

To refresh idle after changing the artwork, copy the matching 16/32/48 PNGs to
`assets/icons/tray-idle-{size}.png`, copy the SVG to `assets/icons/tray-idle.svg`,
and render the missing 22 px size from the vector:

```sh
rsvg-convert --width 22 --height 22 assets/icons/tray-idle.svg -o assets/icons/tray-idle-22.png
```

Update the multiresolution ICO and installed app PNGs together when the app mark changes.
