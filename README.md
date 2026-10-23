# Grajek

Grajek is a native Linux desktop app that plays YouTube audio in a configured
Discord voice channel through your own bot. Paste a video URL and select Play URL
to start a single audio stream.

- Pause, resume, skip tracks, seek in compatible streams and adjust output volume.
- Manage up to 100 pending tracks or import a public YouTube playlist.
- Repeat a track or the queue, keep playback history and optionally save the queue.
- Stop playback or leave voice without closing the window.

## Requirements

The documented desktop setup is Linux x86_64 on CachyOS / Arch. You need a
Wayland or X11 graphical session with working Vulkan GPU drivers, Rust stable
1.92 or newer, and the native build dependencies below. CI runs on Ubuntu 22.04.

Playback requires `yt-dlp` and Deno 2.3 or newer on `PATH`, plus compatible
`yt-dlp-ejs` scripts. Keep these tools updated together. Grajek streams audio
without FFmpeg and does not install system packages for you.

On CachyOS / Arch, install missing prerequisites:

```sh
sudo pacman -Syu --needed base-devel rust clang cmake pkgconf opus \
  fontconfig freetype2 wayland libxkbcommon libxkbcommon-x11 libx11 libxcb \
  openssl zstd vulkan-headers vulkan-icd-loader yt-dlp deno yt-dlp-ejs
```

If you use rustup instead of Arch's `rust` package, use `rustup update stable`
and make sure `rustc` and `cargo` select it. Vulkan also needs the driver for your
GPU, such as `vulkan-radeon`, `vulkan-intel` or your installed NVIDIA driver.
Keep your existing driver setup if it works.

Check the tools before configuring Discord:

```sh
rustc --version
cargo --version
pkg-config --modversion opus fontconfig wayland-client xkbcommon xkbcommon-x11 x11 xcb vulkan
yt-dlp --version
deno --version
```

## Set up Discord and start playback

### Create and invite the bot

1. Create an application in the [Discord Developer Portal](https://discord.com/developers/applications).
   Open Bot, create or reset its bot token, and keep it private. Use a bot account.
2. In OAuth2 > URL Generator, select the `bot` scope and the View Channels,
   Connect and Speak permissions. Open the generated invite URL and install the
   bot into your server. Administrator and application commands are unnecessary.
3. In Discord, enable User Settings > Advanced > Developer Mode. Right-click the
   server and choose Copy Server ID. Right-click an ordinary voice channel and
   choose Copy Channel ID. Stage channels are unsupported.
4. Check channel and category overrides: the bot needs View Channel, Connect and
   Speak in that channel. Do not server-mute it. Your network must allow outbound
   Discord HTTPS/WebSocket and voice UDP traffic.

Grajek requests only the non-privileged `GUILDS` and `GUILD_VOICE_STATES` gateway
intents. Leave Message Content, Server Members and Presence intents off.

### Configure and run

From the repository directory:

```sh
cp .env.example .env
```

Edit `.env` and replace all three placeholders:

```dotenv
DISCORD_BOT_TOKEN=your_bot_token
DISCORD_GUILD_ID=your_guild_id
DISCORD_VOICE_CHANNEL_ID=your_voice_channel_id
```

The IDs must be nonzero numbers. Environment variables take precedence over
`.env`. Keep the token private; `.env` is gitignored. Do not include it in bug
reports. Restart Grajek after changing configuration.

Launch from the repository directory so Grajek can load `.env`:

```sh
cargo run
```

Without valid configuration, the window shows an error and disables playback.
After startup:

1. Join the configured voice channel in Discord to listen.
2. Paste a public YouTube video URL into Grajek, for example
   `https://www.youtube.com/watch?v=aqz-KE-bpKQ` (Big Buck Bunny).
3. Select Play URL or press Enter in the URL field. Grajek connects the bot to
   voice and shows the title and elapsed time when playback starts.

Single-video actions accept `youtube.com/watch?v=...`,
`music.youtube.com/watch?v=...` and `youtu.be/...` links with an 11-character
video ID. They discard playlist, tracking and start-time parameters.
Playlist-only links require Import Playlist.

## Basic use

### Playback and queue

The Queue tab holds up to 100 pending entries in insertion order. With repeat
off, a track's natural completion starts the first pending entry. An empty queue
returns the player to Ready.

| Control | Behavior |
| --- | --- |
| Play URL / Enter | Replaces the playing or paused track immediately and keeps the queue. Invalid input leaves playback intact. |
| Add to queue / Ctrl+Enter | Appends a video without starting playback, joining voice or interrupting the current track. |
| Play Queue / Next | Starts the first pending entry. During playback or preparation it reads Next and cancels the current track or request first. Requires a pending entry. |
| Pause / Resume | Pauses or resumes the current track without advancing the queue. |
| Stop | Cancels audio preparation, playback and pending recovery. Keeps the queue and never advances it. Playlist import continues. |
| Leave voice | Stops audio and recovery, then disconnects. Keeps local data; a later Play URL or Play Queue reconnects. |
| Clear queue | Removes pending entries and cancels playlist import; leaves the current track playing. |

Move entries up or down, or remove them in the Queue tab. Titles load separately;
a title lookup failure does not prevent playback. The History tab holds the 50
most recently played unique videos, newest first. Play now replaces the current
track; Add to queue appends it. Clear history asks for confirmation.

Repeat cycles through Off, Repeat Track and Repeat Queue. Repeat Track restarts
a successfully completed track. Repeat Queue rotates successfully completed
queued tracks behind pending entries; a direct Play URL track never enters that
cycle. Next skips the interrupted track, and Stop never requeues it. Clear queue
can leave the active queued track repeating alone; use Stop or turn repeat off.

Continue on error defaults to Off. When enabled, it skips queued media failures
after a short delay and stops after three consecutive failures. It does not retry
failed URLs or skip direct Play URL, gateway, authorization or voice-connection
failures. Remaining queued entries stay available.

The bot leaves voice after ten idle minutes and on normal window close. Paused
playback counts as active; pending queue entries alone do not prevent idle leave.
The window stays open after idle leave. Play URL or Play Queue reconnects.

### Import a public playlist

Paste a `https://www.youtube.com/playlist?list=...` link and select Import Playlist.
Links on `youtube.com` or `music.youtube.com` and watch links with both `v` and
`list` also work. Import Playlist uses the playlist; Play URL and Add to queue
use only the selected video. Pasting alone does nothing.

Import reads metadata without joining voice or interrupting playback. It scans
at most the first 100 playlist entries, skips known unavailable entries and
appends valid entries in order, including duplicates, up to the queue's available
capacity when import finishes. Imported entries can still fail when played.
Use Cancel Import to cancel; a failed or cancelled import adds nothing.

### Seeking, volume and keyboard controls

Drag the progress slider to preview a timestamp, then release to seek. Seeking
preserves playing or paused state. It requires a known duration and a compatible
HTTP range source, including WebM/Opus and streamable M4A/AAC. Live streams and
tracks without a duration show elapsed time only. If a seek fails, Grajek disables
seeking for that track; Play URL reopens it.

Volume ranges from 0 to 100% and applies to current and subsequent tracks,
including while paused. It cannot override server mute or a listener's volume
for the bot in Discord.

Use Tab to focus controls and Enter or Space to activate buttons. Ctrl+1 and
Ctrl+2 select Queue and History. Slider shortcuts apply only while focused:

| Slider | Arrow keys | Page Down / Page Up | Home / End |
| --- | --- | --- | --- |
| Playback progress | Left/Right: back/forward 5 seconds | Back/forward 10 seconds | Start / one second before duration, or zero for shorter tracks |
| Volume | Left/Down: -5; Right/Up: +5 percentage points | -10 / +10 percentage points | 0% / 100% |

## Local data and settings

Grajek saves history, volume, repeat mode, Continue on error and the Remember queue
preference in `$XDG_DATA_HOME/grajek/state.json`, or
`$HOME/.local/share/grajek/state.json` if an absolute `XDG_DATA_HOME` is not set.
It stores canonical video URLs and metadata, not bot credentials, expiring stream
URLs or audio files.

Remember queue defaults to Off. Turning it on saves pending entries and their
titles. Turning it off removes the saved queue and keeps the current session's
queue. Restoration never starts playback or joins voice. The active track is not
saved; an abrupt exit can restore an older snapshot or lose recent queue changes.

Grajek writes through a temporary file and replaces the saved state atomically.
If it can set aside a malformed file, it renames it to
`state.json.corrupt-<unix time>` and starts empty with a warning. If it cannot
preserve the file, it leaves it intact and disables saving. Unavailable or
unwritable storage also produces a warning; playback can continue without saving.

## Troubleshooting and limits

| Problem | What to check |
| --- | --- |
| Missing configuration or rejected token | Replace placeholders with a bot token and numeric IDs, invite the bot to the server and restart. |
| Cannot join voice | Check server/channel IDs, permissions and overrides, channel capacity and outbound voice UDP access. |
| Playing but no audible output | Check server mute, Grajek's volume and your Discord volume for the bot. Playing confirms track progress, not what a listener hears. |
| YouTube extraction or challenge failure | Update `yt-dlp`, `deno` and `yt-dlp-ejs` together. Use the [standalone extraction check](docs/development.md#check-youtube-extraction) to isolate the failure. |
| No window | Use a graphical session and a working Vulkan driver. For X11/XWayland, try `env -u WAYLAND_DISPLAY cargo run`. An unreachable Wayland socket does not automatically fall back to X11. Leave `ZED_HEADLESS` unset. |
| Playback stops after a disconnect | Connectivity recovery does not restart interrupted audio. Once the gateway is ready, use Play URL or Play Queue. Fatal configuration/gateway startup failures require a restart. |

Grajek controls one bot in one server and one ordinary voice channel, with one
track at a time. It has no search, shuffle, playlist library, Discord commands or
drag and drop. Linux is the documented desktop platform; historical native-window
checks cover KWin Wayland and XWayland, not every compositor or display setup.

Grajek ignores user yt-dlp configuration and does not load browser cookies or
authentication options. Private, age-restricted or region-blocked videos,
CAPTCHA requests and HTTP 403 responses can prevent playback, even after
extraction succeeds. It does not bypass restrictions or generate PO tokens.
Use media only where access and playback are authorized by applicable service terms.

For diagnostics, run `RUST_LOG=grajek=debug,warn cargo run`. Detailed failures go
to stderr; the window shows shorter messages. Check logs for private information
before sharing them, and never share `.env`. The
[development guide](docs/development.md) has extraction and windowing checks;
[historical validation](docs/validation.md) records test scope and known gaps.

## Development

Run the same four checks as [CI](.github/workflows/ci.yml):

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
cargo build --locked
```

CI uses Rust 1.99.0 on Ubuntu 22.04. See the
[development guide](docs/development.md) for test prerequisites, optional network
checks and implementation notes, [PRODUCT.md](PRODUCT.md) for the behavior
contract, and [DESIGN.md](DESIGN.md) for the native interface design.

## License

[MIT](LICENSE).
