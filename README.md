# Grajek

Grajek is a native Linux and Windows desktop app that plays YouTube audio in a selected
Discord voice channel through your own bot. Paste a video URL and select Play URL
to start a single audio stream.

- Pause, resume, skip tracks, seek in compatible streams and adjust output volume.
- Manage up to 100 pending tracks or import a public YouTube playlist.
- Repeat a track or the queue, keep playback history and optionally save the queue.
- Stop playback or leave voice without closing the window.
- Use Linux MPRIS or Windows system media controls for the bot's Discord playback.

Version 0.5.0 adds in-app bot setup, server/channel discovery and native tray controls.
See [release evidence and limitations](docs/releases/v0.5.0.md). Native Windows
execution and live Discord acceptance must not be inferred from cross-builds.

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

### Windows build and runtime

Use Windows 10 (version 1703 or later) / Windows 11 x64, Rust's `x86_64-pc-windows-msvc` toolchain and Visual
Studio 2022 C++ Build Tools with a Windows 10/11 SDK. Install CMake on `PATH`;
libopus is built from the locked crate's bundled source. GPUI uses Direct3D 11,
DirectWrite and DirectComposition; use current graphics drivers with Direct3D
feature level 10.1 or better. The import table includes Windows' ICU library
(`icuuc.dll`, present from Windows 10 version 1703) and `VCRUNTIME140.dll`.
Portable users need Microsoft's [x64 Visual C++ v14 runtime](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist)
if it is not already installed. Debug GUI builds may use the optional Windows
Graphics Tools debug layer when available. Release builds need SDK `fxc.exe`
(the pinned GPUI build script searches the SDK; `GPUI_FXC_PATH` can override it).

Install `yt-dlp.exe` and `deno.exe` on your user `PATH`, with Deno 2.3 or newer
and compatible EJS scripts. Reopen your terminal/Explorer session after changing
`PATH`. Grajek invokes the executables directly and does not install them.
`--remote-components ejs:npm` allows yt-dlp to retrieve compatible challenge
scripts if local EJS scripts are missing; it does not install executables.
FFmpeg is unnecessary for the supported streams.

From an x64 Developer PowerShell/Command Prompt:

```powershell
rustup show active-toolchain
cmake --version
yt-dlp --version
deno --version
cargo build --locked --release
```

The portable executable is `target\release\grajek.exe`. No administrator
permissions are needed to run it. An unsigned cross-compiled release candidate is produced in this workspace's
`target/dist` when available; native Windows acceptance remains pending. Debug
builds use source-tree shaders and are not portable distributions.

### Linux desktop installation and application icons

Build and install the app for your user (no sudo):

```sh
cargo build --locked --release
make install
```

This installs `~/.local/bin/grajek`, the application menu entry and the Grajek
icons in `~/.local/share/icons/hicolor`. Ensure `~/.local/bin` is on your desktop
session's `PATH`. For a system package, use `PREFIX=/usr` and optionally
`DESTDIR=/path/to/staging`; `DATADIR` can override the shared data location.

Windows builds embed the multiresolution ICO in `grajek.exe`; no separate icon
file is needed at runtime. Linux matches the installed desktop entry to the
window on both Wayland and X11. The idle tray icon uses the Grajek logo; playing,
paused and error retain their state icons. See [icon assets](assets/brand/README.md).

## Set up Discord and start playback

### Create and invite the bot

1. Create an application in the [Discord Developer Portal](https://discord.com/developers/applications).
   Open Bot, create or reset its bot token, and keep it private. Use a bot account.
2. In OAuth2 > URL Generator, select the `bot` scope and the View Channels,
   Connect and Speak permissions. Open the generated invite URL and install the
   bot into your server. Administrator and application commands are unnecessary.
3. Check channel and category overrides: the bot needs View Channel, Connect and
   Speak in an ordinary voice channel. Stage channels are unsupported. Do not
   server-mute the bot. Discord HTTPS/WebSocket and voice UDP must be reachable.

Grajek requests only the non-privileged `GUILDS` and `GUILD_VOICE_STATES` intents.
Leave Message Content, Server Members and Presence intents off.

### Configure in Grajek

1. Launch Grajek (or `cargo run` from a development checkout). No `.env` is needed.
2. Paste the **bot token** into the masked field in Settings. Choose **Connect and
   remember** for OS-protected storage, or **Use for this session** without saving it.
   Never enter a personal Discord user token or password.
3. Choose a server where the bot is installed, then an eligible voice channel.
   Refresh if the bot was just invited or permissions changed. Disabled channels
   explain missing permissions; no IDs need copying.
4. Return to the player. The header shows the selected destination. Join that
   voice channel in Discord to listen, paste a YouTube video URL, and press Play URL.

Linux credentials use Secret Service (including KWallet's Secret Service support).
Windows uses Credential Manager. If secure storage fails, Grajek explains the
failure; explicitly choose session-only to continue. Tokens never enter state.json.
An older saved token remains unchanged when you use a different session-only token.
Remove saved token removes the local credential, not the token on Discord.

Settings can be reopened at any time. Retry connection reauthenticates without
restarting. Switching while playing, paused or preparing requires confirmation,
stops the current track and leaves the old voice connection. Pending queue,
history and preferences remain. A switch or restored selection never starts audio.

### Tray and closing

The native tray provides Show, Hide, Play/Pause/Resume, Next/Play Queue, Stop,
Leave Voice and Quit. Controls follow the controller's actual capabilities.
**Close to tray** is opt-in and defaults off, including for migrated installations.
When enabled with an available tray, Close hides the existing window and keeps
playback and OS media controls running. Otherwise Close quits. **Quit Grajek**
always terminates the bot and flushes state.

Hide removes the window from the desktop and taskbar without closing it. Show
restores the same window, including from minimized state. The pinned GPUI Linux
backend has a scoped visibility patch; Windows uses the existing HWND. Wayland
restore may let the compositor choose a new placement. Focus requests remain
subject to desktop policy. KDE Plasma needs a StatusNotifier host; if the host
disappears while Grajek is hidden, Grajek restores its window and Close falls
back to Quit. Native Windows and live Discord acceptance remain pending; see
[the v0.5.0 release record](docs/releases/v0.5.0.md).

### Advanced environment compatibility

Existing `DISCORD_BOT_TOKEN`, `DISCORD_GUILD_ID` and
`DISCORD_VOICE_CHANNEL_ID` settings still work. Explicit environment variables
win over `.env`; an external token wins over secure storage and is labelled
externally managed. The UI cannot replace or remove it. No `.env` is deleted or
migrated. A valid legacy guild/channel pair takes precedence over saved selection;
malformed/missing IDs instead lead to destination selection after authentication.
Saved selections are associated with the authenticated bot and revalidated.

Linux retains dotenv lookup from the working directory and ancestors. Windows
checks `%LOCALAPPDATA%\Grajek\.env`, then `.env` beside `grajek.exe`; Explorer's
working directory is irrelevant. Change externally managed credentials at their
source and restart to reload the process environment. For advanced diagnostics,
Discord Developer Mode exposes Copy Server ID / Copy Channel ID; these are not
needed by the normal setup flow. `.env.example` documents this optional route.

Single-video actions accept `youtube.com/watch?v=...`,
`music.youtube.com/watch?v=...` and `youtu.be/...` links with an 11-character
video ID. They discard playlist, tracking and start-time parameters.
Playlist-only links require Import Playlist.

## Basic use

### Playback and queue

The Queue tab holds up to 100 pending entries in insertion order. With repeat
off, a track's natural completion starts the first pending entry. An empty queue
returns the player to Ready.

| Control                   | Behavior                                                                                                                                               |
| ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Play URL / Enter          | Replaces the playing or paused track immediately and keeps the queue. Invalid input leaves playback intact.                                            |
| Add to queue / Ctrl+Enter | Appends a video without starting playback, joining voice or interrupting the current track.                                                            |
| Play Queue / Next         | Starts the first pending entry. During playback or preparation it reads Next and cancels the current track or request first. Requires a pending entry. |
| Pause / Resume            | Pauses or resumes the current track without advancing the queue.                                                                                       |
| Stop                      | Cancels audio preparation, playback and pending recovery. Keeps the queue and never advances it. Playlist import continues.                            |
| Leave voice               | Stops audio and recovery, then disconnects. Keeps local data; a later Play URL or Play Queue reconnects.                                               |
| Clear queue               | Removes pending entries and cancels playlist import; leaves the current track playing.                                                                 |

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

| Slider            | Arrow keys                                    | Page Down / Page Up         | Home / End                                                     |
| ----------------- | --------------------------------------------- | --------------------------- | -------------------------------------------------------------- |
| Playback progress | Left/Right: back/forward 5 seconds            | Back/forward 10 seconds     | Start / one second before duration, or zero for shorter tracks |
| Volume            | Left/Down: -5; Right/Up: +5 percentage points | -10 / +10 percentage points | 0% / 100%                                                      |

## Native system media controls

Linux registers `org.mpris.MediaPlayer2.grajek` on the user's session D-Bus.
MPRIS clients and compatible desktop media widgets can discover it. Windows uses
SMTC attached to Grajek's actual native window. Both represent audio sent to
Discord, not local speaker playback. Controls work independently of window focus.
Unknown-duration tracks clear the Windows timeline rather than inventing a total.
Windows shell presentation and seeking affordances depend on the OS; native
Windows discovery has not yet been verified for this release.

External Play resumes a paused track or starts the pending queue from Ready/Error.
It does nothing when already playing, resolving, or when no media is available.
Pause, PlayPause (MPRIS), Next and Stop use the existing controller semantics;
Stop keeps pending entries. Previous is unsupported. MPRIS relative Seek and
SetPosition, and SMTC timeline requests where exposed, use the current track ID
and existing seek validation. Old track IDs cannot seek a successor. Loading and
pending seeks never publish unconfirmed playback or timestamps.

Title and known duration come from actual track metadata; no artwork, artist or
album is invented. Metadata clears when no track is playing/paused. Unseekable
streams do not advertise seek. MPRIS reports the applied Discord bot volume as a
read-only value; volume writes and playback-rate changes are unsupported. Adjust
bot volume in Grajek. OS integration failures log a warning and leave the player
usable. A second Linux instance keeps playback but cannot take the first
instance's MPRIS name. No reconnect loop or system service is installed.

## Local data and settings

Grajek saves history, volume, repeat mode, Continue on error and the Remember queue
preference in `$XDG_DATA_HOME/grajek/state.json`, or
`$HOME/.local/share/grajek/state.json` if an absolute `XDG_DATA_HOME` is not set.
On Windows the same JSON format lives at `%LOCALAPPDATA%\Grajek\state.json`;
it never goes in the executable directory. Existing v0.3.2 files stay compatible.
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

| Problem                                 | What to check                                                                                                                                                                                                                                                                                                 |
| --------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Missing configuration or rejected token | Replace placeholders with a bot token and numeric IDs, invite the bot to the server and restart.                                                                                                                                                                                                              |
| Cannot join voice                       | Check server/channel IDs, permissions and overrides, channel capacity and outbound voice UDP access.                                                                                                                                                                                                          |
| Playing but no audible output           | Check server mute, Grajek's volume and your Discord volume for the bot. Playing confirms track progress, not what a listener hears.                                                                                                                                                                           |
| YouTube extraction or challenge failure | Update `yt-dlp`, `deno` and `yt-dlp-ejs` together.                                                                                                                                                                                                                                                            |
| No window                               | On Windows check graphics drivers/Direct3D support and use a native release build. On Linux use a graphical session and a working Vulkan driver. For X11/XWayland, try `env -u WAYLAND_DISPLAY cargo run`. An unreachable Wayland socket does not automatically fall back to X11. Leave `ZED_HEADLESS` unset. |
| Playback stops after a disconnect       | Connectivity recovery does not restart interrupted audio. Once the gateway is ready, use Play URL or Play Queue. Fatal configuration/gateway startup failures require a restart.                                                                                                                              |

Grajek controls one bot in one server and one ordinary voice channel, with one
track at a time. It has no search, shuffle, playlist library, Discord commands or
drag and drop. Historical Linux native-window checks cover KWin Wayland and
XWayland, not every compositor or display setup. Current platform verification
and its outstanding Windows checks are recorded in the release report.

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

`vendor/gpui-pre-linux/` is a required, patched source dependency for native
Hide/Restore. Include the whole directory in version control. Its large initial
diff mostly copies upstream code; [the maintenance note](vendor/gpui-pre-linux/README.grajek.md)
explains the focused patch, verification and eventual removal. Agent guidance is
in [AGENTS.md](AGENTS.md).

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
