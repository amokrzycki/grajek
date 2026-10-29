# Agent instructions

## Model delegation

- Use the configured default subagent model for delegated work.
- Do not explicitly request GPT-6 Astra for subagents.
- Do not request GPT-6 Astra for review, exploration, verification or testing.
- Keep the orchestrator model unchanged; use a stronger subagent only when the user explicitly requests it.

## Vendored GPUI Linux backend

- `vendor/gpui-pre-linux/` is a required source dependency, not a cache or generated output. Keep the complete directory, license and Cargo patch configuration together in version control; do not add it to `.gitignore`.
- Before changing it, read `vendor/gpui-pre-linux/README.grajek.md` and `visibility.patch`. Most source files are copied unchanged from registry version 0.3.9; the patch records our actual backend changes. Cargo compiles the sources, not the patch file.
- Keep edits scoped to native visibility. Do not mass-format, replace or upgrade the vendored crate as incidental cleanup. Update `visibility.patch` against the exact documented upstream baseline when changing its source.
- Preserve the existing GPUI window, playback controller and media sessions. Hide must unmap rather than minimize or close; every tray Show path must restore hidden/minimized windows. Respect compositor focus restrictions.
- Validate backend changes with the documented Cargo checks and real KDE Wayland/XWayland tests in `scripts/check-tray-linux.py`. Cross-compilation does not establish native Windows behavior; stopped fixtures do not establish live Discord audio.
- Remove the vendor override only after a replacement API passes those native checks; update Cargo configuration/lockfile and `src/window.rs` together. A local registry edit or ignored source directory is not a reproducible replacement.
