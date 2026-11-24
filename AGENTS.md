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
- Remove the vendor override only after a replacement API passes those native checks; update Cargo configuration/lockfile and `src/platform/window.rs` together. A local registry edit or ignored source directory is not a reproducible replacement.

## Code structure

- Keep non-test source files under 500 lines. Before a change pushes a file past that, split it by concern in the same change.
- One responsibility per module, named for the concern (`seek`, `rejoin`, `settings/connection`). No `utils`, `helpers` or `common`. A directory module's `mod.rs` holds its shared state and wiring, not feature logic.
- Dispatchers only route: `PlayerApp`, `Controller::command` and the tray menu call a method in the owning module. Do not inline feature branches or special cases into them.
- Search before adding a helper. Limits, URL/ID parsing, paths, yt-dlp invocation and formatting each have one home: `youtube`, `paths`, `ytdlp`, `app::fmt`. Put a helper next to its domain, not in the first module that needs it.
- Use the narrowest visibility (`pub(super)`, then `pub(crate)`). Do not add a wrapper that has one caller and adds no boundary.
- Model states and modes as enums with exhaustive matches. Do not use parallel booleans or `Option` pairs.

## Refactors

- Moves and splits are behavior-neutral: `git diff -M` should show moves, and any behavior change goes in a separate commit.
- Do not delete or weaken tests while moving code. Keep each test next to its module (`tests.rs` or `tests/`), with shared fixtures in `test_support` or `tests/support.rs`. The passing test count must not drop.
- New logic needs a test in its own module. Tray, window and Discord behavior still need the native checks above.

## Verification

Before calling work done, run `cargo fmt --all -- --check`, `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`, and `cargo test --locked --workspace --all-features`.

## Agent skills

### Issue tracker

Use GitHub Issues in `amokrzycki/grajek`. Read `docs/agents/issue-tracker.md` before ticket operations.

### Triage labels

Use the five default triage labels. Read `docs/agents/triage-labels.md` before triaging issues.

### Domain docs

Use single-context domain docs. Read `docs/agents/domain.md` before exploring domain concepts or decisions.
