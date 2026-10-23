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
The [Linux CI workflow](../.github/workflows/ci.yml) uses Rust 1.99.0 on Ubuntu
22.04 and lists its native build packages. It runs:

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
Discord output. The child-cancellation test uses `/proc`; a historical sandbox
restriction and its effect are recorded in [validation](validation.md#v010-wayland-audit-2026-10-12).

### Optional media checks

The network checks need `yt-dlp`, Deno and public YouTube access. The local
loopback check needs neither extractor tools nor YouTube access. None needs a
Discord token:

```sh
# Network: extract, open and decode a public video stream.
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
Unsafe code is forbidden. Unused code, Clippy `all`/`pedantic`, panic paths,
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
