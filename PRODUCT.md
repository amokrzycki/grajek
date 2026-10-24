# Grajek

<!-- impeccable:product-schema 1 -->

## Platform
Native Linux desktop (Rust / GPUI Kit 0.7.3). Impeccable's mobile/web platform enum does not describe this app; do not apply browser CSS or mobile conventions.

## Users
A user controlling a configured Discord music bot from a compact desktop window. No additional audience assumptions are confirmed.

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
- Controller snapshots are the truth. No optimistic playback state, simulated waveform, artwork or discovery.
- Native accessible seek actions use controller intents and disabled semantics. Delayed polls retain the last confirmed position for a bounded 15-second liveness window; range failures never qualify as successful natural completion or repeat rotation.
- Queue is FIFO, limited to 100 pending entries; optional Remember queue defaults off, saves pending-only canonical URLs/titles and restores without starting playback. Disabling erases the saved queue while keeping the session queue. History holds 50 unique successfully played videos.
- Repeat Off (default), Track (fresh extraction after confirmed success) and Queue (successful queued completions rotate behind pending entries) are saved preferences. Direct Play URL does not enter Queue repeat; a queued singleton cycles. The mode at completion applies. Stop, Next, cancellation and errors never recycle interrupted tracks; Next needs a pending successor. Clear removes pending entries only.
- Idle Next is labelled Play Queue and starts the first pending entry. The controller owns availability and serializes extraction/cancellation.
- Saves remain atomic and serialized. Normal close flushes the latest snapshot; abrupt termination may recover an older snapshot. In-flight/dequeued entries are not guaranteed to recover.
- Enter plays; Ctrl+Enter queues; Ctrl+1/2 selects Queue/History. Volume keyboard handling and focus recovery are preserved.
- Window defaults to 520×560 and permits 420×480. Input and transport stay fixed; lists scroll.
- Missing configuration still opens a usable window with an error and disabled playback actions. Restored queue/history edits are disabled if no controller started; local edits remain available after gateway loss with a running controller.
- No added crates, remote visual assets, web rendering, backend restructuring, commit or push in this task.

## Brand Commitments
Name: Grajek. Contemporary, elegant, premium, restrained and recognizable, with warmth from the Polish name. No retro hardware skin, cartoon, music-note ornament, SaaS dashboard or streaming-service clone.

## Evidence on Hand
README.md, src/app.rs, src/app/{controls,queue,history,tests}.rs and controller tests. No committed screenshots or artwork. Supplied design brief is authoritative; design decisions are delegated to the implementer.

## Product Principles
- Make the real track and transport state immediately readable.
- Preserve native controls and controller-owned behavior.
- Express personality with one useful detail and precise proportions.
- Keep compact layouts usable with long titles, failures and keyboard input.
