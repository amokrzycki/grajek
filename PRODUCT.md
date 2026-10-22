# Grajek

<!-- impeccable:product-schema 1 -->

## Platform
Native Linux desktop (Rust / GPUI Kit 0.7.3). Impeccable's mobile/web platform enum does not describe this app; do not apply browser CSS or mobile conventions.

## Users
A user controlling a configured Discord music bot from a compact desktop window. No additional audience assumptions are confirmed.

## Product Purpose
Paste a YouTube video URL, play it in a Discord voice channel, and manage transport, volume, upcoming tracks and recent playback without a media-library interface.

## Capabilities and Constraints
- Existing Play URL, enqueue, Pause/Resume, Next, Stop, live volume and persisted history.
- Opt-in, persisted Continue on error skips queued media failures only, with a one-second cancellable delay and a three-failure limit; only actual playback resets that limit.
- Songbird owns transient retries. At most two fresh joins may follow eligible terminal voice failures; interrupted audio requires explicit Next/Play and never restarts automatically.
- Voice leaves after ten idle minutes; Playing, Paused and pending jobs are active. Queue and history survive, and the window stays open.
- Controller snapshots are the truth. No optimistic playback state, simulated waveform, artwork, seeking or discovery.
- Queue is FIFO, limited to 100 pending entries; optional Remember queue defaults off, saves pending-only canonical URLs/titles and restores without starting playback. Disabling erases the saved queue while keeping the session queue. History holds 50 unique successfully played videos.
- Repeat Off (default), Track (fresh extraction after confirmed success) and Queue (successful queued completions rotate behind pending entries) are saved preferences. Direct Play URL does not enter Queue repeat; a queued singleton cycles. The mode at completion applies. Stop, Next, cancellation and errors never recycle interrupted tracks; Next needs a pending successor. Clear removes pending entries only.
- Idle Next is labelled Play Queue and starts the first pending entry. The controller owns availability and serializes extraction/cancellation.
- Saves remain atomic and serialized. Normal close flushes the latest snapshot; abrupt termination may recover an older snapshot. In-flight/dequeued entries are not guaranteed to recover.
- Enter plays; Ctrl+Enter queues; Ctrl+1/2 selects Queue/History. Volume keyboard handling and focus recovery are preserved.
- Window defaults to 520×560 and permits 420×480. Input and transport stay fixed; lists scroll.
- Missing configuration still opens a usable window with an error and disabled playback actions. Restored queue/history edits are disabled if no controller started; local edits remain available after gateway loss with a running controller.
- No added dependencies, remote visual assets, web rendering, backend restructuring, commit or push in this task.

## Brand Commitments
Name: Grajek. Contemporary, elegant, premium, restrained and recognizable, with warmth from the Polish name. No retro hardware skin, cartoon, music-note ornament, SaaS dashboard or streaming-service clone.

## Evidence on Hand
README.md, src/app.rs, src/app/{controls,queue,history,tests}.rs and controller tests. No committed screenshots or artwork. Supplied design brief is authoritative; design decisions are delegated to the implementer.

## Product Principles
- Make the real track and transport state immediately readable.
- Preserve native controls and controller-owned behavior.
- Express personality with one useful detail and precise proportions.
- Keep compact layouts usable with long titles, failures and keyboard input.
