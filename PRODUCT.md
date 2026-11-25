# Grajek

<!-- impeccable:product-schema 1 -->

## Platform
Native Linux and Windows 10/11 x64 desktop (Rust / GPUI Kit 0.7.3). Impeccable's mobile/web platform enum does not describe this app; do not apply browser CSS or mobile conventions.

## Users
A user configuring and controlling their Discord music bot from a compact desktop window. No additional audience assumptions are confirmed.

## Product Purpose
Paste a YouTube video URL, import a public playlist into the pending queue, play it in a Discord voice channel, and manage transport, volume, upcoming tracks and recent playback without a media-library interface.

## Capabilities and Constraints
- Existing Play URL, enqueue, Pause/Resume, Next, Stop, live volume and persisted history.
- Opt-in, persisted Continue on error skips queued media failures only, with a one-second cancellable delay and a three-failure limit; only actual playback resets that limit.
- Songbird owns transient retries. At most two fresh joins may follow eligible terminal voice failures; interrupted audio requires explicit Next/Play and never restarts automatically.
- Leave voice explicitly stops/cancels audio and automatic recovery, disconnects without clearing data, and reports teardown success or failure. Only a subsequent manual Play URL/Next reconnects.
- New pending titles resolve through one independent metadata-only yt-dlp job at a time, with a 20-second timeout, duplicate sharing and stable-ID results. Loading/failure labels remain playable; Remember queue retains resolved titles.
- Explicit Import Playlist uses the existing URL field; watch links with a list import the playlist, while Enter/Play URL and Ctrl+Enter/Add to queue keep single-video behavior. Pasting does nothing. Metadata-only import never joins voice, starts or interrupts audio.
- One owned cancellable import scans the first 100 playlist entries with a 45-second subprocess deadline and bounded output. Valid video IDs/titles become one ordered batch; duplicates remain distinct. Available queue slots are recalculated at completion. Invalid/unavailable entries and known capacity omissions are reported; the scan boundary reports that more may remain without an exact fabricated count.
- Import status is separate from playback. Cancel Import, Clear queue and shutdown invalidate pending work; Stop/Next and per-entry queue edits remain independent. Playlist titles use existing sanitization; missing titles share the existing worker. Remember queue and repeat apply normally after insertion.
- Voice leaves after ten idle minutes; Playing, Paused and pending jobs are active. Queue and history survive, and the window stays open.
- Native playback progress follows the existing 250 ms Songbird position poll. Drag previews stay local; release sends one controller-owned seek and reconciles with its actual result. WebM/Opus and streamable M4A/AAC use bounded HTTP ranges with the existing extraction/headers. Unknown/zero duration and live streams show elapsed time only; incompatible sources and failed seeks disable seeking honestly.
- Focused progress keys: Left/Right ±5 seconds, Page Down/Up ±10 seconds, Home zero, End one second before duration (zero for subsecond tracks). One owned seek task and one latest target preserve transport responsiveness; per-track IDs cancel stale work/gestures. Pause state, volume, queue/repeat, history, recovery and playlist import remain independent.
- Controller snapshots are the truth. No optimistic playback state, simulated waveform, artwork, recommendations or unbounded discovery.
- Native accessible seek actions use controller intents and disabled semantics. Delayed polls retain the last confirmed position for a bounded 15-second liveness window; range failures never qualify as successful natural completion or repeat rotation.
- Pending entries stay FIFO and are limited to 100. Queue next inserts a new entry at the front of the pending queue (refused when full); Move to next moves an existing entry to the front, keeps its identity and works when full. Neither interrupts, stops or replaces the current track or joins voice while idle. With Repeat Track on, the current track keeps repeating until Next or Play Queue. Repeat Queue picks its successor when the track ends, so later insertions do not jump ahead of it; optional Remember queue defaults off, saves pending-only canonical URLs/titles and restores without starting playback. Disabling erases the saved queue while keeping the session queue. History holds 50 unique successfully played videos.
- Repeat Off (default), Track (fresh extraction after confirmed success) and Queue (successful queued completions rotate behind pending entries) are saved preferences. Direct Play URL does not enter Queue repeat; a queued singleton cycles. The mode at completion applies. Stop, Next, cancellation and errors never recycle interrupted tracks; Next needs a pending successor. Clear removes pending entries only.
- Favorites is a local, newest-first collection of deliberately saved videos, capped at 500 unique entries. Save and Remove change membership only: they never join voice, prepare or start audio, reorder the queue, change repeat or record History. Play now remains the explicit action that replaces playback.
- Save/Remove is available for the current playing or paused track, the exact known requested video while Joining/Preparing, pending queue entries, History entries and the selected Find Music result. It needs a running controller, not Discord availability, queue capacity or playback components. Membership renders only after controller confirmation; disk-write failures use the existing persistent notice.
- Canonical video URL identity means different supported URL forms for one video produce one favorite. Repeated Save keeps the existing entry and its position. A later explicit Save may fill missing title, channel, duration or live facts from metadata the controller already knows, without overwriting known facts, reordering or starting a lookup; missing facts stay absent.
- Favorites survives queue consumption, History eviction or clearing, queue clearing, disabling Remember queue, destination changes and restart, and restoration never autoplays. Saving an existing member still succeeds at capacity; adding another video is refused with "Favorites full (500 tracks). Remove a favorite and try again." Nothing is evicted. Favorites Play now/Add to queue/Queue next use the existing capability checks: Add to queue remains requestable at a full queue and receives controller refusal, while Queue next is disabled; Save/Remove stays available in either case and while Discord is unavailable.
- An invalid or oversized favorites list drops only the offending entries. Recovery to the first 500 valid unique entries preserves an exact backup of the original file before any bounded replacement is written. A failed backup keeps the original untouched, disables persistent writes for the session with an explanation and still permits local management. An older release that rewrites state can discard favorites; downgrade is not lossless.
- Idle Next is labelled Play Queue and starts the first pending entry. The controller owns availability and serializes extraction/cancellation.
- Saves remain atomic and serialized. Normal close flushes the latest snapshot; abrupt termination may recover an older snapshot. In-flight/dequeued entries are not guaranteed to recover.
- Enter plays; Ctrl+Enter queues; Ctrl+1/2/3 selects Up next/History/Favorites. Volume keyboard handling and focus recovery are preserved.
- Window defaults to 520×560 and permits 420×480. Input and transport stay fixed; lists scroll.
- First run opens Discord Settings with a persistently labeled masked bot-token input, explicit secure storage or session-only use, retry and a concise bot-creation guide. Authentication leads to native Server → Voice channel selectors; validated returning destinations bypass setup. Configured settings disclose credential changes/removal and background-playback preferences. Idle leads with a link or Play Queue instead of inactive transport. Local queue/history operations run before authentication. Bot token credentials never enter snapshots or JSON.
- Guild/channel choices come from the authenticated bot via non-privileged gateway events and rate-limited REST. Effective View Channel, Connect and Speak are checked before selection and joining; stage channels are disabled. Bot-associated selection restores only after validation, never joining or playing automatically.
- Runtime reconfiguration is serialized by the controller. Playing/paused/preparing changes require confirmation; acceptance cancels extraction/seek, stops the interrupted track and disconnects before activating a destination. Queue, history and playback preferences survive, with no repeat rotation or autoplay.
- The Linux StatusNotifierItem/D-Bus menu and Windows notification icon issue controller commands. Close-to-tray defaults off and requires an available host. GPUI 0.3.9 only supports minimize/restore here, so the same window and taskbar entry remain; full hiding is not claimed. Quit flushes state and tears down sessions.
- Native sessions observe controller snapshots and submit bounded intents; they never own queue/playback state. Linux uses session D-Bus MPRIS; Windows uses HWND-based SMTC. Previous is unsupported, metadata is factual, and failed OS registration cannot disable playback.
- External Play resumes Paused or starts a pending queue from Ready/Error. Loading/empty Play is a no-op; Stop retains pending items. Seeks validate the current track identity and capabilities at controller consumption. Confirmed seek serials support discontinuity signals without polling.
- Linux keeps existing XDG storage/dotenv behavior. Windows uses LOCALAPPDATA/Grajek for compatible JSON state and first-choice .env, then .env beside the executable. Explicit environment variables win; no token enters state or distribution artifacts.
- Signal Ledger stays shared across platforms; Windows selects installed Segoe UI, Linux retains its font preference. Native titlebar, 520×560 default and 420×480 minimum, controls, focus and shortcuts remain.
- Only target-specific native dependencies are added. No remote visual assets, web rendering, installer, commit, push or release publishing. No app self-updater. Only managed playback components may update, from an Ed25519-signed manifest, after an explicit click. Windows native runtime acceptance remains pending; cross-compilation and Wine tests do not establish it.

## Brand Commitments
Name: Grajek. Contemporary, elegant, premium, restrained and recognizable, with warmth from the Polish name. No retro hardware skin, cartoon, music-note ornament, SaaS dashboard or streaming-service clone.

## Evidence on Hand
README.md, src/app.rs, src/app/{controls,queue,history,tests}.rs and controller tests. No committed screenshots or artwork. Supplied design brief is authoritative; design decisions are delegated to the implementer.

## Product Principles
- Make the real track and transport state immediately readable.
- Preserve native controls and controller-owned behavior.
- Express personality with one useful detail and precise proportions.
- Keep compact layouts usable with long titles, failures and keyboard input.

## Bounded YouTube search

The shared composer accepts a YouTube link or search query. Explicit text submission returns at most five transient video candidates; typing and paste do nothing. Select a result, then explicitly Play now (replaces playback, keeps the queue), Add to queue (appends within the 100-entry bound) or Queue next (inserts at the front within the same bound). A search session is transient and never saves itself: selection, search, cancellation and errors never join voice, change playback or persist results. Only an explicit Save adds a selected result to Favorites. No recommendations, artwork, folders, named playlists, shuffle or automatic favorites.
