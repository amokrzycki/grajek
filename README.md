# Grajek

A compact native Rust desktop music controller: paste a YouTube video URL, press
**Play URL** (or Enter), and broadcast one audio stream to a configured Discord
voice channel. The window shows the track title and elapsed time, and offers
**Pause / Resume**, **Next**, **Stop** and **Leave voice**, a **volume** slider, an optionally remembered
**queue** and a persistent **history**. Another Play URL replaces the current
track, even while paused; Stop cancels pending work and stops playback in both
Playing and Paused states. The bot stays connected for up to ten idle minutes
after Stop or natural completion, then leaves voice while the window stays open.
It also leaves on window close.

## Playback controls (v0.2.0)

- **Now playing:** the title and duration come from the same yt-dlp JSON that
  initializes the stream; no extra request, process, cache or artwork. A missing
  title shows "Untitled track". Titles are whitespace-normalized, capped at 200
  characters and truncated with an ellipsis to the window width.
- **Pause / Resume:** one button, labelled for the action it performs. Both use
  Songbird's `TrackHandle::pause()` / `play()` on the existing track; Resume does
  not reconnect to Discord or resolve YouTube again. The button is disabled while
  a request is in flight and the GUI shows the state the controller reports, never
  a guess. If the track is gone, the controller reports an error instead.
- **Elapsed time:** `01:24 / 03:45`, or `01:24 (duration unavailable)` (live
  streams, missing metadata). It is read from the existing 250 ms
  `TrackHandle::get_info()` poll and published to the UI only when the displayed
  second changes. It stops while paused and restarts at 00:00 for a new track.
  There is no seeking.

## Volume, history and queue (v0.2.0)

**Volume.** A 0 to 100 % slider with the percentage beside it. It maps to
Songbird's `TrackHandle::set_volume(0.0..=1.0)`; there is no amplification above
100 %, and NaN or infinite values are ignored. It changes the live track without
restarting the stream, also while paused, and new tracks start at the selected
volume (it is part of the `Track` handed to Songbird), so Next, Play and
replacement keep it. Mouse drags send the newest value through a `watch` channel
(not one command per pixel), so dragging stays live and cheap. The value is saved
when you let go. Keyboard: click the slider or Tab to it, then Left/Down and
Right/Up change it by 5, Page Up/Down by 10, Home/End jump to 0/100 (each key
press is saved). GPUI Kit's `Slider` has no keyboard handling of its own, so the
window adds it. If the track is gone when the volume is set, the controller
reports an error and the slider returns to the volume actually applied.
Existing users start at 100 %, as before.

**History.** The 50 most recently played unique videos, newest first, in the
**History** tab: title, duration, **Play now** (replaces the current track),
**Add to queue**, remove, and **Clear history** (asks for confirmation inline). A
video is recorded only once its playback has really begun (the position advanced),
never merely because yt-dlp succeeded. Replaying a video moves it to the top.
Only the canonical `watch?v=` URL, title and duration are stored; never
expiring stream URLs. History is independent of the queue.

**Local data.** One small JSON file holds history, volume, repeat mode, Continue on error and the opt-in pending queue:
`$XDG_DATA_HOME/grajek/state.json`, else `$HOME/.local/share/grajek/state.json`
(directory mode 0700). Writes go to `state.json.tmp`, are synced, then renamed
over the real file, so a crash leaves the old or the new file. A single writer
saves the newest snapshot, so a slow write can never overwrite newer data. A
missing file starts empty. A malformed file is renamed to
`state.json.corrupt-<unix time>` (never deleted), a warning appears in the
History tab and the log, and Grajek starts empty. Entries that are not valid
video URLs are dropped on load. Nothing is stored in the repository, `.env` or
the working directory.

**Queue.** A FIFO of at most 100 pending entries (canonical URL plus a known
title). Titles from History, the pending queue or current track are reused. New
titles load asynchronously through a metadata-only yt-dlp request, independently
of playback. One title lookup runs at a time with a 20-second timeout; duplicate
pending URLs share it. Rows show “Loading title…” while waiting, then the title
or “Title unavailable” with an explanatory tooltip. Lookup failure never
prevents playback. No audio is downloaded or decoded and voice is not joined
when an entry is added. **Remember queue** optionally saves pending entries and
resolved titles; it defaults to Off. The controller owns the only copy; the
window renders snapshots. Exact semantics:

- **Play URL** plays immediately, replacing the active or paused track, and
  leaves the queue alone. Invalid input never interrupts the current track.
- **Add to queue** (button, or Ctrl+Enter in the URL field) appends and never stops
  or replaces anything; while idle it does not start playback.
- **Natural completion** with repeat Off starts the first queued entry (exactly
  once); with an empty queue the player returns to Ready. Repeat modes are below.
- **Next** cancels the current track or any request in progress and starts the
  first queued entry; while idle the same button reads **Play Queue**. It is
  disabled when nothing is queued. Only one audio preparation runs; each Next cancels the previous request. Queue title lookup is separate.
- **Stop** cancels pending work and the active track, keeps the queue, never
  advances, and starts the ten-minute idle period.
- **Leave voice**, beside the header connection status, cancels preparation,
  playback and automatic recovery, then disconnects. It keeps the queue, history,
  repeat preferences and volume. “Leaving voice…” remains until teardown finishes;
  failures are reported and can be retried. It never automatically rejoins after
  an intentional leave; a later **Play URL** or **Play Queue** reconnects.
- **Pause / Resume** affect only the active track and never advance the queue.
- **Failure** (yt-dlp, stream or decode) shows the error and keeps the remaining
  queue. By default it does not advance or retry. With **Continue on error**
  enabled, queued media failures may advance as described below. The failed
  entry has already left the queue; explicit Play URL failures always stop.
- Per-entry **move up / move down / remove** and **Clear queue** (does not stop
  the current track); the Queue tab shows the pending count. No drag and drop.

Ctrl+1 / Ctrl+2 switch between the Queue and History tabs (the kit's tabs are not
focusable). If a focused button turns off (for example Stop after stopping), focus
returns to the URL field so Tab keeps working.

## YouTube playlist import (v0.3.1)

Paste a public playlist link into the existing URL field and select **Import Playlist**.
Supported forms are `https://www.youtube.com/playlist?list=…`, the same link on
`youtube.com` or `music.youtube.com`, and YouTube watch links containing both
`v=…` and `list=…`. The import action uses the playlist. **Play URL** / Enter and
**Add to queue** / Ctrl+Enter still use only the selected video; a playlist-only
link is rejected by those single-video actions. Pasting alone does nothing.

- Import reads metadata only, without joining voice, downloading audio, resolving
  stream URLs per entry or starting playback. Current playback, pause, transport,
  volume, repeat and Continue on error remain independent.
- **Cancel Import** replaces the import action while it runs. Only one import can
  run. Cancel, **Clear queue** and normal application close terminate the job;
  cancelled or stale results cannot append tracks. Stop stops audio and leaves
  the import running. Removing/reordering individual entries remains available.
- The scan checks at most the **first 100 playlist entries**. Invalid IDs, nested
  results and known private/deleted/unavailable entries are skipped. Valid entries,
  including deliberate duplicates, keep their original order. This does not
  guarantee they will be playable later.
- One atomic append uses the queue's **available slots at completion**. With 75
  pending entries, at most 25 are appended; a full queue appends nothing. Queue
  edits during extraction are respected. Feedback reports imported/unavailable
  entries and known entries that did not fit. At the scan boundary it says more
  may remain; it never invents an exact count beyond the scan. Full feedback is
  available in the status tooltip and native accessibility label.
- Valid playlist titles are reused and normalized with the existing 200-character
  persisted-title rules. Missing/invalid titles use the existing serialized title
  worker and truthful loading/unavailable labels, without a second metadata pipeline.
- Successful entries participate in the ordinary queue and Repeat Queue cycle.
  **Remember queue** saves pending URLs/titles only when enabled, including imports,
  and restores without starting playback. Cancellation/failure saves no imported
  entries. The usual atomic-save and abrupt-exit limits still apply.

Extraction uses one cancellable yt-dlp process with a 45-second deadline, a 2 MiB
stdout limit and a 64 KiB stderr limit. It ignores user configuration and plugins;
no browser cookies, authentication or restriction bypass is used. A restricted,
unavailable, malformed or completely unusable playlist leaves the queue unchanged.
No playlist library, private-playlist support or unrestricted scan is provided.
See [v0.3.1 release evidence](docs/releases/v0.3.1.md) for validation and limitations.

## Queue experience (v0.3.0)

**Play Queue** starts the first pending entry while idle (Ready or Error), keeping
all remaining entries. The same transport becomes **Next** during preparation,
playback or pause. Availability comes from the controller: no gateway, an empty
queue, initialization or stopping disables it. It reconnects idle voice as before,
and Next cancels an earlier extraction before starting another.

**Repeat** is a native keyboard-accessible button beside the URL actions. Each
click or Enter/Space cycles **Off → Repeat Track → Repeat Queue → Off**. Its label
always shows the controller's selected mode; active repeat uses the existing brass
selection accent. The preference is saved; older files default to Off.

- **Off:** ordinary FIFO advancement after natural completion, then Ready.
- **Repeat Track:** a successfully completed track starts again from its canonical
  video URL with fresh yt-dlp extraction. Pending entries are untouched. Applies
  to both queued and direct Play URL tracks.
- **Repeat Queue:** only tracks started through the queue join the cycle. A
  successfully completed queued track moves behind the pending entries, then the
  next entry starts: A, B, C, A, B, C. Duplicate URLs remain distinct entries. A
  single queued track repeats. A direct Play URL track is never inserted into the
  cycle; its completion advances to the pending queue or Ready. Rotation pops the
  successor before appending the completed item, retaining the 100-pending limit
  even if entries were added while playing.

The selected mode at natural completion applies, including changes while playing
or paused. Only confirmed playback can repeat; an End before position advancement
is treated as a media failure when repeat is enabled. Extraction/startup/decode
errors and unexpected termination never recycle a track. Continue on error still
applies only to queued failures, with its cancellable one-second delay and
three-consecutive-failure limit. Starting an extraction or changing repeat never
resets that budget. Stale terminal events cannot repeat or skip a successor.

**Next** bypasses Repeat Track and discards the interrupted track from Repeat Queue;
it needs a pending successor. **Stop**, replacement Play URL and cancellation do
not requeue the interrupted track. Stop keeps pending entries. Clear queue removes
pending entries only: with Repeat Queue the current queued track can still repeat
alone after success. Use Stop or switch repeat Off to end that cycle.

**Remember queue: Off/On** sits beside Queue/History and uses ordinary native
Tab/Enter/Space navigation. Turning it on saves the current pending FIFO. Turning
it off removes stored entries without touching the active in-memory queue. Every
add, remove, reorder, clear, rotation and dequeue updates the serialized snapshot.
Restoration preserves duplicates/order, creates unique runtime IDs, drops invalid
entries, caps the queue at 100 and normalizes titles to 200 characters. Only
canonical YouTube URLs and safe titles are stored, never streams, credentials,
audio or active handles. Old v0.2.1 files keep history, volume and recovery settings.

Restoration **never starts playback or joins voice automatically**. If startup
fails (for example missing credentials or tools), restored lists remain visible
and their edit controls are disabled until a successful restart. A running
controller still permits local list edits after gateway loss. Only pending
entries are saved: an entry leaves the snapshot when dequeued, before extraction
or startup, and may therefore be lost if playback fails or the app closes then.
The active track is not restored, including during repeat. Normal shutdown waits
for the latest snapshot writer. Writes are asynchronous and atomically replaced;
an abrupt exit can restore the last completed snapshot, so a recent mutation may
be missing or an already-dequeued entry may reappear. There is no guarantee of
recovering an in-flight track. Failed writes are logged and leave the prior file
intact; saving can resume on a later change. No database or extra file format is added.

See [v0.3.0 release evidence](docs/releases/v0.3.0.md) for validation, independent
review and unverified live scenarios.

## Reliability and playback polish (v0.2.1)

**Continue on error.** The Queue tab has one native button, **Continue on error:
Off/On**, usable with the mouse or normal Tab/Space button navigation. It is off
by default and saved in the existing JSON file. Older files retain their history
and volume and default to off. Failed URLs are not retried; pending entries are
saved only when Remember queue is enabled. Only failures after voice is connected, for entries consumed by Next or
natural completion, can continue. This includes extraction, stream initialization,
playback-start and decode failures. Explicit Play URL (including History's Play
now), gateway/authorization failures and voice connection loss never skip media.

A failed queued track shows an error and waits one second before consuming the
next entry. Stop, Next, Play URL, shutdown, or turning the preference off can
cancel that wait. The failed-track explanation remains visible through automatic
continuation, including successful playback; a manual playback action or Stop
clears it. Recovery stops after **three consecutive queued failures**, or when
nothing remains queued, with an explanation. Only an advancing playback position
resets the counter; successful extraction, a new handle and manual commands do
not. Pending queue entries are never cleared by recovery. History still means
“actually started playing”, not “completed successfully”.

**Discord recovery.** Songbird 0.6.1 owns transient voice reconnects and its default
bounded backoff (five retries). Grajek waits for its terminal disconnect event
before considering a fresh gateway join. For positively identified I/O/timeouts,
expired sessions and voice-server crashes, it allows at most **two application
rejoin attempts**, delayed by **one and three seconds**, each capped at **40
seconds**. Another attempt requires another positively classified transient
failure. Kicks, channel moves, authorization/protocol failures, unknown close
reasons and terminal gateway failures require an explicit action; Grajek does not add
gateway retries to Serenity's own handling.

An observed reconnect stops the interrupted track and restores connectivity
only. It never re-extracts or restarts that video, advances the queue, or claims
Playing. Press **Next** or **Play URL** to start audio again. Stop, Next, Play URL
and shutdown cancel pending rejoin work. Failed recovery leaves an actionable
error and the queue intact; subsequent Next/Play uses the ordinary connect path.
Connection-generation/session checks reject old driver callbacks, and an
intentional leave does not become a connection error. Healthy connection updates
cannot erase an unrelated media error.

**Idle voice leave.** After **ten minutes** with no track and no playback/voice
job or recovery pending, Grajek gracefully leaves voice. Playing and Paused are
both active; extraction and connection work also prevent idle leave. The pending
queue alone does not. Stop starts a fresh idle period and preserves the queue.
The window, queue and history survive idle leave. The header distinguishes
**Discord ready · voice idle** from **Discord voice connected**; Next and Play URL
reconnect normally without an idle-error callout. Queue edits and volume changes
alone do not postpone the deadline. One controller deadline handles idle leave
and recovery; there is no idle polling task or extra runtime.

**Lifecycle.** Natural End still advances once. An unexpected Stop/error is not
reported as successful completion, and a late End cannot erase a decode error.
Playback status reads are cancellable by commands and shutdown. Pending GUI
commands disable their controls while retaining the last controller snapshot;
the GUI does not invent transport phases.

## Prerequisites (CachyOS / Arch, Linux x86_64)

Rust stable **1.92+**, a graphical Wayland or X11 session, and working Vulkan GPU
drivers. The checked-in lockfile uses GPUI Kit 0.7.3 with its pinned GPUI 0.3.9,
Serenity 0.12.5, Songbird 0.6.1 and Symphonia 0.5.5. Kit's Linux dependency `oo7`
sets the Rust 1.92 minimum; do not override individual GPUI crates.

Install missing prerequisites yourself; the app does not change system packages:

```sh
sudo pacman -Syu --needed base-devel rust clang cmake pkgconf opus \
  fontconfig freetype2 wayland libxkbcommon libxkbcommon-x11 libx11 libxcb \
  openssl zstd vulkan-headers vulkan-icd-loader yt-dlp deno yt-dlp-ejs
```

If using rustup instead of Arch's `rust` package, use `rustup update stable` and
ensure `rustc`/`cargo` select that stable toolchain. Vulkan also needs the driver
appropriate for your GPU (for example `vulkan-radeon` or `vulkan-intel`, or your
installed NVIDIA driver). Keep your existing driver setup if it already works.
No WebKit, FFmpeg, ALSA microphone support, web frontend or server is used.

```sh
rustc --version
cargo --version
pkg-config --modversion opus fontconfig wayland-client xkbcommon xkbcommon-x11 x11 xcb vulkan
yt-dlp --version
deno --version
```

## Discord setup

1. Create an application in the [Discord Developer Portal](https://discord.com/developers/applications).
   Open **Bot**, create/reset its bot token and keep it private. Use a bot account.
2. Under **OAuth2 → URL Generator**, select the `bot` scope and **View Channels**,
   **Connect**, **Speak** permissions. Open the generated invite URL and install
   the bot into your private server. Administrator and application commands are unnecessary.
3. Enable Discord's **User Settings → Advanced → Developer Mode**. Right-click
   the server and select **Copy Server ID**; right-click an ordinary voice channel
   and select **Copy Channel ID**. Stage channels are unsupported.
4. Make sure channel/category overrides allow the bot to View Channel, Connect
   and Speak. Do not server-mute the bot. Allow outbound Discord HTTPS/WebSocket
   and voice UDP traffic.

The app requests only `GUILDS` and `GUILD_VOICE_STATES` gateway intents. Both are
non-privileged; leave Message Content, Server Members and Presence intents off.
Songbird 0.6.1's driver includes DAVE end-to-end encryption through `davey`; no
legacy encryption implementation or separate DAVE feature flag is needed.

## Configure and run

```sh
cp .env.example .env
# Edit .env with your bot token, server ID and voice channel ID.
cargo run
```

Required variables: `DISCORD_BOT_TOKEN`, `DISCORD_GUILD_ID`,
`DISCORD_VOICE_CHANNEL_ID`. Environment values take precedence over `.env`.
Launch from the project directory to load that file. `.env` is gitignored;
credentials are never displayed by the application. Restart after changing
configuration. Without credentials the window still opens, explains what's
missing and disables playback controls.

Normal `youtube.com/watch?v=…`, `youtu.be/…` and
`music.youtube.com/watch?v=…` links are supported. Links must identify an
11-character video ID. Playlist/tracking/start-time parameters are discarded;
playlist-only URLs and unrelated domains are rejected.

## Linux windowing

GPUI Kit 0.7.3 enables both `wayland` and `x11` on its pinned
`gpui-pre-platform` 0.3.9 dependency, which forwards them to `gpui-pre-linux`.
Both backends are compiled for normal builds, independently of `test-support`.
GPUI prefers native Wayland when `WAYLAND_DISPLAY` is nonempty, even when
`DISPLAY` is also set; otherwise it selects X11 when `DISPLAY` is nonempty.
The app uses the default platform selection and does not override these variables.

```sh
env -u DISPLAY cargo run          # Native Wayland; run inside a Wayland session.
env -u WAYLAND_DISPLAY cargo run  # X11 / XWayland; requires a working DISPLAY.
cargo tree --locked -e normal,build,features -i gpui-pre-platform
```

Fallback is environment-based: if `WAYLAND_DISPLAY` names an unreachable socket,
GPUI fails startup rather than automatically retrying X11. Unset it explicitly
to use X11/XWayland. `ZED_HEADLESS` also forces GPUI's headless platform; leave it
unset for this desktop app. GTK/Qt backend variables do not select GPUI's backend.

## yt-dlp, Deno and audio

Use current yt-dlp with **Deno 2.3+**. On current Arch, `yt-dlp` depends on
`yt-dlp-ejs`; keep the packages updated together. The app passes
`--ignore-config --no-playlist --js-runtimes deno --remote-components ejs:npm`.
Compatible packaged/bundled EJS scripts are used locally; the npm flag permits
yt-dlp's documented fallback if matching scripts are absent. It does not require
an EJS download when local components are available. User yt-dlp configuration,
cookies and authentication options are intentionally not loaded.

Extraction selects `bestaudio[acodec=opus][ext=webm]/bestaudio[ext=m4a]/bestaudio`.
Songbird handles HTTP headers and streaming through `YoutubeDl::get_stream`.
The app owns the Tokio subprocess because Songbird's `YoutubeDl::query` does not
terminate yt-dlp on cancellation. Stop/shutdown abort that request with
`kill_on_drop(true)`; no full audio file is downloaded before playback.
Symphonia enables MKV/WebM and ISO-MP4/AAC for the selected fallback formats;
Songbird supplies Opus decoding/passthrough. No FFmpeg conversion is required.

To check extraction independently using Blender's openly licensed Big Buck Bunny:

```sh
yt-dlp --ignore-config --no-playlist --js-runtimes deno --remote-components ejs:npm \
  --socket-timeout 15 --retries 0 --extractor-retries 0 \
  -f 'bestaudio[acodec=opus][ext=webm]/bestaudio[ext=m4a]/bestaudio' \
  --skip-download --print '%(id)s %(ext)s %(acodec)s' \
  'https://www.youtube.com/watch?v=aqz-KE-bpKQ'
```

Use media only where access and playback are authorized by applicable service
terms. Private/age-restricted/region-blocked videos, CAPTCHA requests and HTTP
403 responses produce an error. The app does not bypass restrictions, acquire
cookies or generate PO tokens.

## Validation

Lint levels live in `Cargo.toml`, following the
[Random Frame desktop policy](https://github.com/amokrzycki/random-frame/blob/main/src-tauri/Cargo.toml)
and [server policy](https://github.com/amokrzycki/random-frame-server/blob/main/Cargo.toml).
Unsafe code is forbidden; unused code, Clippy `all` and `pedantic`, panic paths,
debugging artifacts, and selected ownership/async/restriction checks warn during
development and fail the strict checks below. Lints already included in
`all`/`pedantic` are not declared twice. The entire `nursery` and `restriction`
groups are intentionally not enabled.

Use the currently selected rustup toolchain (minimum Rust 1.92); no upgrade or
project override is required. Install the tools if they are missing:

```sh
rustup component add clippy rustfmt
```

Formatting uses default rustfmt settings. Run the complete strict validation:

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
cargo build --locked
# Optional network test: extraction, stream access and Songbird initialization.
cargo test --locked public_video_stream_initializes_with_songbird -- --ignored --nocapture
```

There are no application feature flags, so `--all-features` is supported and does
not enable every optional feature of dependencies. GPUI's `test-support` comes
from the existing dev-dependency; normal builds keep both native Linux backends.

`future_not_send` is enabled after auditing the Tokio tasks and GPUI executor.
The UI watch task stays on `Context::spawn`, whose future deliberately has no
`Send` bound; do not move it to Tokio or add synchronization to satisfy a lint.
Serenity/Songbird callbacks keep their required async trait signatures.

Exceptions use local `#[expect(..., reason = "...")]`, which also warns if an
exception is no longer needed. The controller's biased select loop stays in one
function so cancellation, priority and track ownership remain explicit; its
per-event work lives in small `Controller` methods. Each GPUI test function and window helper expects `expect_used`: the GPUI
test macro requires a unit-returning callback, so window failures must fail the
test with context. Ordinary Rust tests propagate setup errors with `Result`.
Production code has no `unwrap()` or `expect()` calls.

The minimal [Linux CI workflow](.github/workflows/ci.yml) runs the four strict
commands on Ubuntu 22.04 with explicit GPUI/Opus system packages and Rust 1.99.0,
the toolchain used for this lint audit. It does not change the local toolchain.
The commands also passed from a clean target directory in a disposable Ubuntu
22.04 container (v0.1.0): 10 tests passed and the existing optional network test stayed
ignored. Dependency versions and features are unchanged.

After enabling this policy, a native Wayland smoke test with the existing Discord
configuration reached Playing through Discord voice and yt-dlp/Songbird. Stop
returned to Ready, and normal window close logged clean Discord shutdown. This
checks the UI and playback state path; audible output was not independently
verified.

Unit tests cover URL/ID validation, action availability, child-process cancellation
and terminal playback errors. GPUI tests exercise clipboard paste, Enter, button clicks, duplicate
Play prevention and disabled controls without requiring Discord credentials.

Verified locally on CachyOS with Rust 1.99.0: formatting, check, Clippy with warnings
denied, build and 10 offline tests passed (v0.1.0). The optional network test also passed:
yt-dlp 2026.08.19 / Deno 2.9.7 / EJS 0.8.0 extracted Opus/WebM, and Songbird opened,
initialized and decoded an audio packet from that stream. The real native X11 window was checked for typing,
clipboard paste, visible configuration errors, disabled controls and normal close.
The user subsequently verified Discord voice connection and audible YouTube playback.

On **2026-10-12**, `env -u DISPLAY cargo run` was tested in the actual CachyOS
KDE/KWin **6.7.5 Wayland session**, using the existing Vulkan driver. Native
keyboard/pointer input verified typing, Ctrl+V paste and Ctrl+C clipboard round
trip, input focus after button clicks, Play, Stop and Enter. Play joined Discord
voice and reached Playing through yt-dlp/Songbird; Stop returned to Ready.
Enter dispatched a second request, which encountered a YouTube HTTP 403.
Titlebar close exited successfully and logged `Discord shutdown complete`
without a controller timeout. A separate launch with both display variables set
exchanged native Wayland window protocol; missing-configuration controls remained
disabled. With `WAYLAND_DISPLAY` unset, X11 client properties confirmed an
**XWayland** fallback window, which also closed successfully.

The unchanged suite passed with `cargo test --locked --offline`: **10 passed,
1 optional network test ignored**. It required execution outside the agent sandbox;
the sandbox run failed the existing `/proc`-based child cancellation check.
This audit did not independently listen to audio or observe the bot leaving in
the Discord client. Other Wayland compositors, IME composition, fractional scaling
and a standalone Xorg session were not tested. No implementation, Cargo feature
or dependency changes were needed.

## v0.2.0 validation

**Automated** (Rust 1.99.0, 2026-10-12): `cargo fmt --all -- --check`, Clippy with
`-D warnings`, `cargo test --locked --workspace --all-features` and
`cargo build --locked` pass; 97 offline tests pass and the optional network test
stays ignored. Coverage: volume conversion and bounds, live/new-track/paused/failed
volume handling, persistence (round trip, missing, malformed, repaired, atomic
replace, newest-snapshot writer, XDG paths), history rules (50 entries, dedupe,
order, only after playback begins), every queue rule above (FIFO, 100 limit,
natural end exactly once, stale End events, Next while playing/idle/extracting,
rapid Next, Stop, Pause, failures without retry, invalid URLs, reorder/remove/clear,
shutdown) and the GPUI window (slider by mouse and keyboard, tabs, button states,
row actions, history confirmation, scrolling, long titles/errors, focus).
The controller tests use a fake `Backend`/`Handle` because a detached Songbird
driver does not answer or emit events; the real Songbird/Discord path is `live.rs`.

**GUI smoke test** (KDE Plasma/KWin Wayland, native Wayland launch and an
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

**Not verified:** audible output in Discord (nobody listened, so "volume changes
what you hear" is untested; the value reaches `set_volume` and new `Track`s),
mouse operation of the real window, graceful window-close shutdown (checked in
earlier versions, not re-run), a live Next during extraction, the bot's behavior
in the Discord client, and compositors other than KWin. During one run the bot was
disconnected from the voice channel by Discord (WebSocket closed with
`Disconnected`), presumably not by Grajek; it showed the "Press Play to reconnect"
error as designed.

## Troubleshooting and limits

- **Missing configuration/token rejected:** replace placeholders with a real bot
  token and numeric IDs; invite the bot into that server and restart.
- **Cannot join:** check IDs, permissions/overrides, channel capacity and UDP
  access. The app checks permissions before joining and uses bounded timeouts.
- **YouTube extraction/challenge failed:** update `yt-dlp`, `deno`, `yt-dlp-ejs`
  together; run the extraction command above with `--verbose`. A stream URL can
  resolve successfully and still be rejected when fetched or decoded.
- **No window:** run in a desktop session with a working Vulkan driver.
  To try X11/Xwayland from a Wayland session, use `env -u WAYLAND_DISPLAY cargo run`.
- **Logs:** `RUST_LOG=grajek=debug,warn cargo run`. Detailed failures go
  to stderr; the window shows short, actionable messages. Do not share your `.env`.
- One guild, one ordinary voice channel, one track at a time; no seek, playlist library,
  shuffle, search, Discord commands or drag and drop.
  A queued entry is resolved only when it starts, so it can still fail then (private,
  removed or region-blocked videos). Network interruptions stop playback; Play can
  retry after gateway recovery. Fatal configuration/gateway startup failures
  require restart.
- Volume is the bot's own output level; it cannot override Discord-side server mute
  or each listener's volume for the bot.
- If the data directory cannot be determined or written, the app works but logs a
  warning and history and settings are not saved.
- The GUI runs on GPUI's main executor; bounded commands and watch snapshots
  bridge to Tokio. Songbird track handles/events report completion and errors;
  Playing waits for actual track position advancement. This cannot prove that
  a participant hears audio (server mute/client volume still matter).

## API references

[GPUI Kit installation](https://gpui-kit.com/docs/installation/),
[getting started](https://gpui-kit.com/docs/getting-started/),
[Input](https://gpui-kit.com/component/input/),
[Button](https://gpui-kit.com/component/button/),
[Songbird](https://docs.rs/songbird/0.6.1/songbird/),
[Serenity](https://docs.rs/serenity/0.12.5/serenity/),
[yt-dlp EJS setup](https://github.com/yt-dlp/yt-dlp/wiki/EJS),
[Discord voice/DAVE](https://docs.discord.com/developers/topics/voice-connections),
[Arch yt-dlp package](https://archlinux.org/packages/extra/any/yt-dlp/).

## v0.2.1 validation

**Automated** (Rust 1.99.0, 2026-10-21): all four strict commands above pass:
format check, Clippy with warnings denied, **121 passed / 0 failed / 1 optional
network test ignored**, and the locked build. Running the optional network test
explicitly also passes (**1 passed**), including extraction, stream initialization
and decoding. Tokio's existing dependency gains only the dev-only `test-util`
feature for controlled time; production dependencies are unchanged.

New coverage includes queued-only opt-in recovery, failed track startup, the
three-failure budget and reset after actual playback, pending queue preservation,
Stop/Next cancellation, bounded voice recovery and exhaustion, terminal errors
arriving during backoff or a rejoin, duplicate/stale connection callbacks,
manual playback after recovery failure and during recovery/idle leave, shutdown,
idle leave while inactive and suppression while Playing/Paused/extracting, and
responsive Stop with an unresponsive track-status request. Existing queue,
volume, history, subprocess cancellation and compact-window GPUI tests still pass.

**Native smoke test:** the real XWayland window, existing Discord configuration
and an isolated `/tmp` data directory were used. Play joined voice and reached
Playing with title/elapsed time. Actual mouse interaction toggled and persisted
Continue on error; separate mouse checks reached Paused and resumed to Playing.
Stop returned to Ready. Next was clicked while a real yt-dlp child was observed;
logs showed the replacement request and subsequent Playing. Volume changes were
displayed and persisted in scratch data. Normal titlebar/Alt+F4 close exited with
code 0 and logged `Discord shutdown complete`, with no controller timeout. Both
test windows were closed; real saved preferences/history were not modified.

**Not verified:** audible output/volume changes (no independent listener), a
forced live Discord disconnect/reconnect, the complete ten-minute idle cycle in
a live voice session, and native Wayland for this release. Connection fault and
idle behavior are covered by deterministic fake-backend/event-handler tests;
those tests do not prove Discord service behavior. See
the v0.2.1 validation above for the behavior matrix and review.
