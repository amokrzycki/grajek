# Development and diagnostics

See the [README](../README.md) for setup and everyday use,
[PRODUCT.md](../PRODUCT.md) for the behavior contract and
[DESIGN.md](../DESIGN.md) for the native interface. Past test results belong in
[historical validation](validation.md); they do not establish current results.

## Build and verification

The manifest requires Rust 1.92 or newer. The lockfile uses GPUI Kit 0.7.3,
GPUI 0.3.9, Serenity 0.12.5, Songbird 0.6.1 and Symphonia 0.5.5. Kit's Linux
`oo7` dependency sets the Rust 1.92 minimum. Keep the locked GPUI crates together.

Use the selected toolchain; a project override is unnecessary. If Clippy or
rustfmt is missing, install the components with `rustup component add clippy rustfmt`.
The [cross-platform CI workflow](../.github/workflows/ci.yml) uses Rust 1.99.0 on Ubuntu
22.04 and Windows Server 2022 (MSVC) and lists its native build packages. It runs:

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
cargo build --locked
```

There are no application feature flags. `--all-features` does not enable every
optional dependency feature. GPUI's `test-support` comes from the dev-dependency;
normal builds retain both native Linux backends.

Unit tests cover validation, queue and repeat behavior, persistence, subprocess
cancellation, seeking and playback/recovery events. Controller tests use fake
`Backend`/`Handle` implementations; the production backend is
[`src/live.rs`](../src/live.rs). GPUI tests exercise input, focus, controls and
compact layouts without Discord credentials. These tests do not prove audible
Discord output. The subprocess fixtures run the Rust test executable on both OSes; cancellation
proves release of a child-owned loopback socket. No test depends on sh, sleep,
head or /proc. HTTP fixtures need loopback permission. Linux MPRIS tests spawn
an isolated `dbus-daemon`, never the normal desktop service name on the user bus.

### Optional media checks

The network checks need `yt-dlp`, Deno and public YouTube access. The local
loopback check needs neither extractor tools nor YouTube access. None needs a
Discord token:

```sh
# Network: extract, open and decode a public video stream.
cargo test --locked public_video_stream_initializes_with_songbird -- --ignored --nocapture
# Network: investigate a specific video through the same resolver and decoder.
GRAJEK_TEST_VIDEO_URL='https://www.youtube.com/watch?v=Gz1AVEf5K-8' \
  cargo test --locked public_video_stream_initializes_with_songbird -- --ignored --nocapture
# Network: seek in both directions through the production range source.
cargo test --locked public_video_bidirectional_seek -- --ignored --nocapture
# Network: capture mixed audio through Songbird's real mixer for both formats.
python3 scripts/seek-spike.py
# Local loopback: later HTTP 403 and a valid three-second range response.
python3 scripts/seek-spike.py --local
```

The [spike script](../scripts/seek-spike.py) expects Songbird 0.6.1 source in
Cargo's local registry cache and enough cached dependencies for `cargo run --offline`.
It exposes Songbird's existing test harness in a temporary copy and compiles
against the production media pipeline. It leaves the project's dependencies
unchanged. Mixer captures establish mixed audio, not live Discord audibility.

## Linux windowing

GPUI Kit enables `wayland` and `x11` on its pinned `gpui-pre-platform` dependency,
which forwards them to `gpui-pre-linux`. Both compile in normal builds,
independently of `test-support`. GPUI selects Wayland when `WAYLAND_DISPLAY` is
nonempty, even if `DISPLAY` is also set; otherwise a nonempty `DISPLAY` selects X11.
Grajek uses that selection without overriding environment variables.

```sh
env -u DISPLAY cargo run          # Native Wayland; run inside a Wayland session.
env -u WAYLAND_DISPLAY cargo run  # X11 / XWayland; requires a working DISPLAY.
cargo tree --locked -e normal,build,features -i gpui-pre-platform
```

An unreachable Wayland socket fails startup; it does not trigger an automatic
retry through X11. Unset `WAYLAND_DISPLAY` explicitly to try X11/XWayland.
`ZED_HEADLESS` forces the headless platform and should be unset for desktop use.
GTK/Qt backend variables do not select GPUI's backend. Grajek uses neither WebKit
nor an ALSA microphone input, web frontend or server.

## Check YouTube extraction

Run extraction independently with Blender's openly licensed Big Buck Bunny video:

```sh
yt-dlp --ignore-config --no-playlist --js-runtimes deno --remote-components ejs:npm \
  --socket-timeout 15 --retries 0 --extractor-retries 0 \
  -f 'bestaudio[acodec=opus][ext=webm]/bestaudio[ext=m4a]/bestaudio' \
  --skip-download --print '%(id)s %(ext)s %(acodec)s' \
  'https://www.youtube.com/watch?v=aqz-KE-bpKQ'
```

Add `--verbose` for extractor diagnostics. Successful extraction does not prove
that the resulting stream can be fetched or decoded. Keep `yt-dlp`, Deno and
compatible EJS scripts updated together. Packaged or bundled scripts can satisfy
EJS locally; `--remote-components ejs:npm` permits fallback retrieval when matching
scripts are absent. User yt-dlp configuration, cookies and authentication options
are not loaded. Playlist extraction also disables plugin directories.

## Media pipeline and controller

[`src/media.rs`](../src/media.rs) owns the Tokio yt-dlp subprocess with
`kill_on_drop(true)`, because Songbird's `YoutubeDl::query` does not terminate
its child on cancellation. Stop and shutdown cancel audio preparation. Extraction
selects `bestaudio[acodec=opus][ext=webm]/bestaudio[ext=m4a]/bestaudio` and uses a
60-second outer deadline. Symphonia enables MKV/WebM and ISO-MP4/AAC;
Songbird supplies Opus decoding/passthrough. Playback does not require a full-file
download or FFmpeg conversion.

The same extraction provides stream headers, title and duration. Titles normalize
whitespace, use a 200-character cap and fall back to "Untitled track". Compatible
known-duration streams use the [HTTP range source](../src/media/range.rs);
other streams use `YoutubeDl::get_stream` with the extracted metadata. A range
probe failure can fall back to ordinary playback with seeking disabled.

Before either HTTP path opens, Grajek respects the selected stream's `available_at`
Unix timestamp in seconds. yt-dlp 2026.10.21 derives this YouTube field from
pre-roll ad timing and its downloader waits until the timestamp; JSON extraction
alone does not perform that wait. Single-audio selection copies the selected
format's field to the top level. If metadata instead contains `requested_formats`,
Grajek can read the entry matching the URL it will open; unselected `formats`
do not delay playback. See the upstream [timestamp calculation](https://github.com/yt-dlp/yt-dlp/blob/2026.10.21/yt_dlp/extractor/youtube/_video.py)
and [downloader wait](https://github.com/yt-dlp/yt-dlp/blob/2026.10.21/yt_dlp/downloader/common.py).

Missing, invalid, zero and past timestamps add no wait. Future values use a
cancellable Tokio sleep, matching yt-dlp's whole-second current-time calculation;
fractional timestamps are supported. Extraction, waiting, opening and parsing
share the existing 60-second resolution deadline. A wait that would consume the
remaining budget fails immediately with an availability/deadline error. Stop,
Next, replacement, Leave voice and shutdown abort the same preparation task;
Pause/Resume do not resolve streams again.

### Investigating HTTP 403

The `stream_availability` log stage reports the required delay. A failed range
probe logs `http_range_probe` and its cause before attempting Songbird's existing
fallback: HTTP rejection includes the status (for example 403), a successful
response ignoring ranges reports unsupported byte ranges (for example 200), and
transport/read failures retain their error context. Request URLs are removed
from range and fallback-opening transport errors so signed stream query parameters
are not logged. Raw yt-dlp stderr is not logged: failed extraction exposes a
classified access/challenge error, while other diagnostics direct you to reproduce
the extraction manually. Keep that command's verbose output private.
Probe failure alone does not establish that ordinary streaming will fail.

For the affected video `Gz1AVEf5K-8`, a controlled test of the same fresh format-251
URL and yt-dlp headers returned 403 before its future `available_at`, then 206
after a four-second availability wait. This demonstrates premature access for
that attempt; other 403s may have different causes. Compare the selected format,
protocol, availability time and current Unix time using the extraction command
above with `--print '%(format_id)s %(protocol)s %(available_at)s'`. Check the system
clock and update yt-dlp/Deno/EJS before investigating other access restrictions.
Use the optional production decode test with `GRAJEK_TEST_VIDEO_URL` to distinguish
successful metadata extraction from successful HTTP access and decoded audio.
In PowerShell, set `$env:GRAJEK_TEST_VIDEO_URL` before running the same Cargo test.
For a before/after comparison, reuse one fresh URL and the exact extracted headers;
keep signed URLs, query parameters, cookies and authentication headers private.
Do not infer an availability issue from status 403 alone, or infer audible Discord
playback from a successful decode test.

### Controller state

The [controller](../src/player/controller.rs) owns queue state and track handles;
the window renders snapshots. A 250 ms `TrackHandle::get_info()` poll supplies
elapsed time, published when the displayed second changes. Playing and history
recording require actual position advancement. History records startup, not
successful completion. Pause/Resume use `TrackHandle::pause()` / `play()` without
re-extraction. Natural End advances once; unexpected Stop or decode errors do not
count as successful completion, and stale events cannot affect a successor.
Status reads remain cancellable by commands and shutdown.

Volume maps 0..=100% to `TrackHandle::set_volume(0.0..=1.0)`. Invalid nonfinite
values are ignored. A watch channel coalesces drags to the latest value; release
or a keyboard change requests persistence. New Songbird `Track`s start at the
selected level. Failed live updates report an error and restore the applied value.

### Seeking

The native slider previews locally during a drag and sends one seek on release.
It returns to the confirmed position until the controller answers. Keyboard
and native accessibility actions use the same bounded targets; Increment and
Decrement move five seconds, and Set Value takes seconds. Disabled controls reject
actions. Targets clamp between zero and one second before duration, or zero for
subsecond tracks. Track replacement or an unavailable control restores URL focus.

One timed seek task retains one latest pending target. Track IDs reject stale
results and gestures. Stop, Next, Leave voice, replacement, disconnect and shutdown
cancel seeking. It preserves pause state, volume, history, queue/repeat and recovery
budgets. A failed seek disables further seeks on that track; a terminal audio
failure still follows media-failure handling.

The controller seek deadline is ten seconds. Dropping a Songbird callback cannot
cancel the audio worker, so source cancellation, five-second HTTP requests and
an eight-second / 4 MiB container read budget bound its work. A stalled read can
delay audio-thread teardown up to the HTTP deadline. Delayed info callbacks retain
the confirmed position, with a 15-second liveness deadline. Range failures remain
playback errors even when Songbird emits End; they cannot trigger successful
repeat or rotation.

### Queue metadata and playlist import

Queue titles reuse known history/current/pending metadata. One metadata-only
lookup runs at a time with a 20-second timeout; duplicate pending URLs share the
lookup. "Loading title…" and "Title unavailable" describe metadata status and do
not block playback. Runtime IDs prevent stale results from changing other rows.

[`src/playlist.rs`](../src/playlist.rs) owns one cancellable metadata-only process:
a 45-second deadline, 2 MiB stdout limit and 64 KiB stderr limit bound extraction.
It scans the first 100 playlist entries and appends valid results as one ordered
batch. Queue capacity is recalculated at completion. Invalid IDs, nested results
and known private/deleted/unavailable entries are skipped. Feedback distinguishes
unavailable entries and known capacity omissions; reaching the scan boundary
reports that more may remain without claiming an exact count. Full feedback is
available in a tooltip and accessibility label. Missing titles reuse the title
worker. Cancel Import, Clear queue and shutdown invalidate the job; Stop and Next
leave import independent of audio. Failed, cancelled or stale results append nothing.

Repeat Track uses fresh extraction after confirmed completion. Repeat Queue
pops the successor before appending the completed queued track, preserving the
100-pending-entry limit even if the queue filled during playback. Errors and
interrupted tracks never rotate. The mode at completion applies, and an End
before playback confirmation is a failure when repeat is enabled.

### Recovery and idle leave

Continue on error applies to queued media failures after voice connects, including
extraction, initialization, startup and decode failures. It waits one cancellable
second before advancing and stops after three consecutive failures or an empty
queue. Only advancing playback resets the failure count; extraction, manual
commands and repeat changes do not. Stop, Next, Play URL, shutdown or disabling
the preference cancels pending continuation. Its failure notice survives automatic
continuation; manual playback or Stop clears it. Direct Play URL/History Play now,
gateway/authorization failures and voice loss never skip queued media.

Songbird 0.6.1 handles transient voice reconnects with its own bounded backoff.
After its terminal disconnect event, positively identified I/O/timeouts, expired
sessions and voice-server crashes can permit up to two application joins, delayed
one and three seconds and capped at 40 seconds each. Another join requires
another classified transient failure. Kicks, moves, authorization/protocol errors,
unknown close reasons and terminal gateway failures require explicit action.
Grajek does not add gateway retries to Serenity's handling.

An observed reconnect stops interrupted audio and restores connectivity only.
It neither restarts the track nor advances the queue. Stop, Next, Play URL,
Leave voice and shutdown cancel pending recovery. Connection-generation/session
checks reject old callbacks; intentional leave is not a connection error, and
healthy connection updates cannot erase unrelated media errors. Songbird's driver
includes DAVE through `davey`, without a separate application feature flag.

One controller deadline schedules idle leave and recovery. Ten minutes without a
track, playback/voice job or recovery causes voice leave. Playing, Paused and
preparation prevent it; pending entries, queue edits and volume changes do not.
Stop starts a fresh idle period. Explicit Leave voice waits for teardown, reports
failures for retry and never automatically rejoins.

## Persistence

[`src/store.rs`](../src/store.rs) creates the data directory with mode 0700.
A single writer coalesces snapshots, writes `state.json.tmp`, syncs it and renames
it over `state.json`. Missing state starts empty. Malformed state is backed up as
`state.json.corrupt-<unix time>` when possible; failure to preserve it disables
saving so the damaged file stays intact. Load drops invalid video URLs, caps
history at 50 unique entries and the pending queue at 100, and normalizes titles.

Only pending entries are saved when Remember queue is on. Dequeue updates the
snapshot before extraction/startup, so a failed or interrupted active item may be
lost across restarts. Normal close waits for the latest writer; abrupt termination
can recover an older snapshot, lose a recent mutation or restore an already
consumed entry. A failed write logs an error and retains the previous file;
a later change can retry saving. Old files default new preferences to Off and
retain compatible history and volume. Restored lists stay visible on startup
failure, but edits require a running controller; gateway loss after startup
still permits local list edits.

## Lint policy

[`Cargo.toml`](../Cargo.toml) follows the
[Random Frame desktop policy](https://github.com/amokrzycki/random-frame/blob/main/src-tauri/Cargo.toml)
and [server policy](https://github.com/amokrzycki/random-frame-server/blob/main/Cargo.toml).
Unsafe code is forbidden in the application. The local `grajek-smtc` crate denies
unsafe code except in `interop.rs`, which documents its three WinRT/ HWND FFI
calls. Its apartment guard is not Send; the UI owns it and all COM objects. Unused code, Clippy `all`/`pedantic`, panic paths,
debugging artifacts and selected ownership/async/restriction checks warn locally
and fail under `-D warnings`. The full `nursery` and `restriction` groups are not
enabled; checks already in `all`/`pedantic` are not declared twice.

Keep the GUI watch task on GPUI's `Context::spawn`, whose future has no `Send`
bound. Do not move it to Tokio or add synchronization merely to satisfy
`future_not_send`. Serenity/Songbird callbacks keep their async trait signatures.
Use local `#[expect(..., reason = "...")]` exceptions. The biased controller
select loop keeps cancellation, priority and track ownership together; event
work lives in `Controller` methods. GPUI tests use contextual `expect` failures
because their macro requires unit-returning callbacks; ordinary Rust tests can
return `Result`.

## API references

- [GPUI Kit installation](https://gpui-kit.com/docs/installation/),
  [getting started](https://gpui-kit.com/docs/getting-started/),
  [Input](https://gpui-kit.com/component/input/) and [Button](https://gpui-kit.com/component/button/).
- [Songbird 0.6.1](https://docs.rs/songbird/0.6.1/songbird/) and
  [Serenity 0.12.5](https://docs.rs/serenity/0.12.5/serenity/).
- [yt-dlp EJS setup](https://github.com/yt-dlp/yt-dlp/wiki/EJS).
- [Discord voice/DAVE](https://docs.discord.com/developers/topics/voice-connections).
- [Arch yt-dlp package](https://archlinux.org/packages/extra/any/yt-dlp/).

## Native media adapters and Windows

`src/native.rs` projects authoritative snapshots and translates external intents
inside the controller. `Endpoint::send` uses the existing 16-slot command channel
with `try_send`; native callbacks never await network work. Duplicate Play/Pause
are evaluated when consumed. Track IDs reject stale absolute/relative seek
intents; a controller seek serial records successful discontinuities even if
watch updates coalesce. Queue, recovery, repeat and seek implementation stay in
the existing controller.

`src/native/linux.rs` implements the MPRIS root/player interfaces with zbus's
Tokio backend, already present in the locked Linux graph. It explicitly checks
exclusive name acquisition, emits only changed properties, omits Position change
signals, and emits Seeked only after confirmed seek completion. Position queries
return the last controller position, never interpolation. Volume is the reported
bot gain; setting volume/rate is unsupported. Startup is bounded to five seconds;
missing bus, collisions and connection loss are nonfatal. Shutdown closes the
connection with no retry loop.

`src/native/windows.rs` owns a foreground GPUI task, cancelled with PlayerApp.
`crates/grajek-smtc` borrows GPUI's HasWindowHandle only during GetForWindow and
retains no raw HWND. RoInitialize joins the existing GPUI OLE STA and balances
RoUninitialize on the same thread after COM objects drop. Button/timeline tokens
are removed on Drop, including partial registration failures. Callbacks capture
only a bounded controller sender; each position delegate captures its registered
track ID, so a queued old callback cannot acquire a successor's identity. Unsupported
buttons are disabled. Timeline seek bounds collapse to the confirmed position
for unseekable media. Unknown duration clears the Windows timeline to zero
rather than fabricating an end time; MPRIS can still report elapsed position.
No keyboard hook or local MediaPlayer is created.

Build Windows from an x64 Visual Studio developer shell with CMake, the MSVC
Rust toolchain and the Windows SDK installed (see README). The pinned GPUI
release build uses SDK fxc.exe; set GPUI_FXC_PATH if SDK discovery fails. Tests
are headless; hosted CI does not prove actual Windows rendering or shell media
session discovery. Native release CI additionally saves an unsigned executable,
README and .env.example as a build artifact; it never includes real .env files.

Windows configuration uses the first existing per-user .env or executable-side
.env, independently of cwd. Linux dotenv lookup is unchanged. Windows state
uses LOCALAPPDATA/Grajek; std::fs::rename replaces an existing file through the
platform API without deleting it first. The same-directory temporary, sync,
serialized latest-snapshot writer and corrupt-file preservation remain. Windows
tests cover a locked destination failing safely and successful replacement after
closing the lock. Unix directory mode 0700 remains behind cfg(unix).

For manual Linux inspection, start Grajek with an isolated XDG_DATA_HOME, then:

```sh
qdbus6 org.mpris.MediaPlayer2.grajek /org/mpris/MediaPlayer2
qdbus6 org.mpris.MediaPlayer2.grajek /org/mpris/MediaPlayer2 org.mpris.MediaPlayer2.Player.Pause
qdbus6 org.mpris.MediaPlayer2.grajek /org/mpris/MediaPlayer2 org.mpris.MediaPlayer2.Player.Play
qdbus6 org.mpris.MediaPlayer2.grajek /org/mpris/MediaPlayer2 org.mpris.MediaPlayer2.Player.Seek 5000000
```

For native Windows acceptance use a test LOCALAPPDATA and launch the release exe
from Explorer with cwd outside the executable folder. Verify 100/150/200% DPI,
Tab/Space/Enter, Ctrl+Enter and Ctrl+1/2, queue/history, both slider input methods,
playlist import, persistence/restart and normal titlebar close. With a Discord
listener, test actual audible playback and volume. Then use the shell's media
panel/multimedia keys while Grajek is unfocused; verify title, Pause/Play/Next/Stop,
seek when offered, and removal on close. Record OS build, GPU, results and any
shell-specific limitations in the release report. No native result may be
inferred from a successful hosted build or Wine run.

Authoritative references: [MPRIS Player specification](https://specifications.freedesktop.org/mpris/latest/Player_Interface.html),
[desktop GetForWindow](https://learn.microsoft.com/en-us/windows/win32/api/systemmediatransportcontrolsinterop/nf-systemmediatransportcontrolsinterop-isystemmediatransportcontrolsinterop-getforwindow),
[SMTC manual integration](https://learn.microsoft.com/en-us/windows/apps/develop/media-playback/integrate-with-systemmediatransportcontrols),
[Windows DPI testing](https://learn.microsoft.com/en-gb/windows/win32/hidpi/high-dpi-desktop-application-development-on-windows).

### Linux-hosted Windows release evidence

A supplementary optimized cross-build was completed with cargo-xwin 0.23.1.
The pinned GPUI Windows build script runs shader generation only on Windows
hosts, so the Linux attempt initially lacked shaders_bytes.rs. Microsoft Windows
SDK CPP 10.0.26100.9169's x64 FXC and compiler DLL were extracted to /tmp from
Microsoft's NuGet feed and run under isolated Wine. All 18 unchanged GPUI shader
entry points compiled with the exact upstream /O3 vs_4_1/ps_4_1 flags; upstream's
header-to-Rust byte-array conversion populated only the ignored OUT_DIR. The
release executable then linked normally. No shader, GPUI source, Cargo dependency
version or shipped build script was patched. The local clang-cl/libopus cache
workarounds described in the release report also apply. Native Windows builds
and CI run the existing GPUI shader build script directly and need none of these
cross-host steps. Debug builds reference source-tree shader files and should not
be distributed as a portable executable; use the release profile.
