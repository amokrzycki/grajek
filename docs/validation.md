# Historical validation

These reports come from the README at commit
`cdf0654a629d880a3a665e8e90ebc7188077514e`, before its documentation reorganization.
They retain their original release scope and limitations. They were not rerun
as part of the README edit and are not current-checkout test results.

The recorded strict checks were formatting, Clippy with warnings denied,
workspace tests and a locked build. See the [development guide](development.md)
for runnable commands and optional media checks. Counts below describe the suite
at each reported release, including the number of ignored tests at that time.

## v0.1.0 initial validation (date not recorded)

The original README did not give a date for this initial report. It recorded
Rust 1.99.0 and the following results:

The strict commands passed from a clean target directory in a disposable Ubuntu
22.04 container (v0.1.0): 10 tests passed and the existing optional network test
stayed ignored. That validation did not change dependency versions or features.

After enabling the lint policy, a native Wayland smoke test with the existing
Discord configuration reached Playing through Discord voice and yt-dlp/Songbird. Stop
returned to Ready, and normal window close logged clean Discord shutdown. This
checked the UI and playback state path; audible output was not independently
verified.

Verified locally on CachyOS with Rust 1.99.0: formatting, check, Clippy with warnings
denied, build and 10 offline tests passed (v0.1.0). The optional network test also passed:
yt-dlp 2026.08.19 / Deno 2.9.7 / EJS 0.8.0 extracted Opus/WebM, and Songbird opened,
initialized and decoded an audio packet from that stream. The real native X11 window was checked for typing,
clipboard paste, visible configuration errors, disabled controls and normal close.
The user subsequently verified Discord voice connection and audible YouTube playback.

## v0.1.0 Wayland audit (2026-10-12)

On 2026-10-12, `env -u DISPLAY cargo run` was tested in the actual CachyOS
KDE/KWin 6.7.5 Wayland session, using the existing Vulkan driver. Native
keyboard/pointer input verified typing, Ctrl+V paste and Ctrl+C clipboard round
trip, input focus after button clicks, Play, Stop and Enter. Play joined Discord
voice and reached Playing through yt-dlp/Songbird; Stop returned to Ready.
Enter dispatched a second request, which encountered a YouTube HTTP 403.
Titlebar close exited successfully and logged `Discord shutdown complete`
without a controller timeout. A separate launch with both display variables set
exchanged native Wayland window protocol; missing-configuration controls remained
disabled. With `WAYLAND_DISPLAY` unset, X11 client properties confirmed an
XWayland fallback window, which also closed successfully.

The unchanged suite passed with `cargo test --locked --offline`: 10 passed,
1 optional network test ignored. It required execution outside the agent sandbox;
the sandbox run failed the existing `/proc`-based child cancellation check.
This audit did not independently listen to audio or observe the bot leaving in
the Discord client. Other Wayland compositors, IME composition, fractional scaling
and a standalone Xorg session were not tested. No implementation, Cargo feature
or dependency changes were needed.

## v0.2.0 validation (2026-10-12)

Automated checks (Rust 1.99.0, 2026-10-12): `cargo fmt --all -- --check`, Clippy with
`-D warnings`, `cargo test --locked --workspace --all-features` and
`cargo build --locked` passed; 97 offline tests passed and the optional network test
stayed ignored. Coverage: volume conversion and bounds, live/new-track/paused/failed
volume handling, persistence (round trip, missing, malformed, repaired, atomic
replace, newest-snapshot writer, XDG paths), history rules (50 entries, dedupe,
order, only after playback begins), the release's queue rules (FIFO, 100 limit,
natural end exactly once, stale End events, Next while playing/idle/extracting,
rapid Next, Stop, Pause, failures without retry, invalid URLs, reorder/remove/clear,
shutdown) and the GPUI window (slider by mouse and keyboard, tabs, button states,
row actions, history confirmation, scrolling, long titles/errors, focus).
The controller tests used a fake `Backend`/`Handle` because a detached Songbird
driver does not answer or emit events; the real Songbird/Discord path is
[`src/live.rs`](../src/live.rs).

GUI smoke test (KDE Plasma/KWin Wayland, native Wayland launch and an
XWayland run for input injection; a scratch `XDG_DATA_HOME`, not the real one).
The 520 x 560 window opened and rendered correctly (header status, URL row, now
playing, controls, volume, tabs, lists, ellipsized titles). With real Discord
credentials, driven by synthetic keyboard events (the compositor ignores
synthetic pointer clicks, so no mouse interaction was exercised): Play reached
Playing with the title and elapsed time; Pause and Resume; Ctrl+Enter queued
videos without starting playback while idle; Next started a queued track; a
natural end advanced to the next queued entry once, and an unavailable video
showed its error, kept the rest of the queue and did not retry; Stop returned to
Ready, kept the queue, did not advance and stayed connected; History showed
restored entries and Play now from History started the video; volume (keyboard)
and history persisted to `state.json` and were restored after a restart (the
queue was empty, as designed). The slider, Tab order and focus were fixed after
this test exposed a stuck-focus bug and an unreachable slider.

Not verified: audible output in Discord (nobody listened, so "volume changes
what you hear" is untested; the value reaches `set_volume` and new `Track`s),
mouse operation of the real window, graceful window-close shutdown (checked in
earlier versions, not re-run), a live Next during extraction, the bot's behavior
in the Discord client, and compositors other than KWin. During one run the bot was
disconnected from the voice channel by Discord (WebSocket closed with
`Disconnected`), presumably not by Grajek; it showed the "Press Play to reconnect"
error as designed.

## v0.2.1 validation (2026-10-21)

Automated checks (Rust 1.99.0, 2026-10-21): the four strict commands in the
[development guide](development.md#build-and-verification) passed:
format check, Clippy with warnings denied, 121 passed / 0 failed / 1 optional
network test ignored, and the locked build. Running the optional network test
explicitly also passed (1 passed), including extraction, stream initialization
and decoding. That release added only the dev-only `test-util` feature to the
existing Tokio dependency for controlled time; production dependencies were unchanged.

Coverage added in that release included queued-only opt-in recovery, failed track
startup, the three-failure budget and reset after actual playback, pending queue preservation,
Stop/Next cancellation, bounded voice recovery and exhaustion, terminal errors
arriving during backoff or a rejoin, duplicate/stale connection callbacks,
manual playback after recovery failure and during recovery/idle leave, shutdown,
idle leave while inactive and suppression while Playing/Paused/extracting, and
responsive Stop with an unresponsive track-status request. Existing queue,
volume, history, subprocess cancellation and compact-window GPUI tests also passed.

Native smoke test: the real XWayland window, existing Discord configuration
and an isolated `/tmp` data directory were used. Play joined voice and reached
Playing with title/elapsed time. Actual mouse interaction toggled and persisted
Continue on error; separate mouse checks reached Paused and resumed to Playing.
Stop returned to Ready. Next was clicked while a real yt-dlp child was observed;
logs showed the replacement request and subsequent Playing. Volume changes were
displayed and persisted in scratch data. Normal titlebar/Alt+F4 close exited with
code 0 and logged `Discord shutdown complete`, with no controller timeout. Both
test windows were closed; real saved preferences/history were not modified.

Not verified: audible output/volume changes (no independent listener), a
forced live Discord disconnect/reconnect, the complete ten-minute idle cycle in
a live voice session, and native Wayland for this release. Connection fault and
idle behavior are covered by deterministic fake-backend/event-handler tests;
those tests do not prove Discord service behavior.

## v0.3.2 seeking note (date not recorded)

The original README stated that forward and backward seeks had been verified on
public YouTube WebM/Opus and streamable M4A/AAC. It described a real Songbird
mixer spike, native-input tests and independent review, but linked to a release
report absent from this checkout. The README gave no date, test count or detailed
review record for that claim. Retain it as an undated historical statement;
it does not establish a fresh verification of seeking.

The mixer script captures mixed audio through the production pipeline. That
scope does not prove audible output in a live Discord channel. Reproduction
commands and prerequisites are in the
[development guide](development.md#optional-media-checks).

## Missing release reports

The original README linked to `docs/releases/v0.3.0.md`,
`docs/releases/v0.3.1.md` and `docs/releases/v0.3.2.md`. None was present in this
checkout at the documentation reorganization. Those links have been removed;
this document preserves only the evidence recorded in the README and does not
reconstruct missing reports, review findings or unrecorded results.
