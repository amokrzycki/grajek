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
- Controller snapshots are the truth. No optimistic playback state, simulated waveform, artwork, seeking or discovery.
- Queue is FIFO, limited to 100 and not persisted; history holds 50 unique successfully played videos.
- Enter plays; Ctrl+Enter queues; Ctrl+1/2 selects Queue/History. Volume keyboard handling and focus recovery are preserved.
- Window defaults to 520×560 and permits 420×480. Input and transport stay fixed; lists scroll.
- Missing configuration still opens a usable window with an error and disabled playback actions.
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
