# Discord Player

A compact native Rust desktop utility: paste a YouTube video URL, press **Play**
(or Enter), and broadcast one audio stream to a configured Discord voice channel.
Another Play replaces the track; Stop cancels pending work and stops playback.
The bot stays connected after Stop or natural completion and leaves on window close.

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
exception is no longer needed. The controller's `too_many_lines` expectation
keeps the biased select loop's cancellation, priority and track ownership
together. Each of the two GPUI test functions expects `expect_used`: the GPUI
test macro requires a unit-returning callback, so window failures must fail the
test with context. Ordinary Rust tests propagate setup errors with `Result`.
Production code has no `unwrap()` or `expect()` calls.

The minimal [Linux CI workflow](.github/workflows/ci.yml) runs the four strict
commands on Ubuntu 22.04 with explicit GPUI/Opus system packages and Rust 1.99.0,
the toolchain used for this lint audit. It does not change the local toolchain.
The commands also passed from a clean target directory in a disposable Ubuntu
22.04 container: 10 tests passed and the existing optional network test stayed
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
denied, build and 10 offline tests passed. The optional network test also passed:
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
- **Logs:** `RUST_LOG=discord_player=debug,warn cargo run`. Detailed failures go
  to stderr; the window shows short, actionable messages. Do not share your `.env`.
- One guild, one ordinary voice channel, one track; no queue, seek, commands or
  persisted state. Network interruptions stop playback; Play can retry after
  gateway recovery. Fatal configuration/gateway startup failures require restart.
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
