# Playback components diagnosis and recovery

## Intent and approval
The user approved this scope on 2026-11-08: identify missing or broken playback components and download or repair them inside Grajek, including Windows. This supersedes the recovery behavior in `2026-11-08-components-notice-design.md`, not unrelated window or installer work.

## Approaches
- Selected: extend the existing Linux component downloader and native GPUI panel to Windows. This provides recovery without leaving the app and reuses checksums, staging, cancellation and publication safeguards.
- Rejected: offer only a Grajek Setup download. It leaves the repair outside Grajek and does not meet the selected in-app recovery flow.

## Experience
- Operate mode; preserve Signal Ledger colors, native installed fonts, English app copy, existing window and native controls.
- Replace the settings summary with **Playback components**, separate yt-dlp and Deno rows and a short explanation that unavailable components prevent YouTube playback. yt-dlp details explain its bundled EJS; EJS is not a third independently downloadable tool.
- Each row reports a factual state: **Missing**, **Not checked**, **Checking…**, **Ready**, or **Needs repair**, with a short reason for failure. A file's presence alone never establishes Ready. Ready requires a successful bounded version check. Managed versions must match their manifest; system Deno must satisfy the existing minimum of 2.3.
- One filled brass action: **Download missing components** when components are absent, or **Repair components** when broken. Keep **Check components** available in settings and repair available even when version checks pass but playback remains broken. The recovery flow may replace the complete verified pair to preserve atomic publication.
- **Details** discloses detected versions and locations, expected versions, official release sources, approximate platform-specific download size, checksums/licences and actionable diagnostics. Explain that nothing downloads without consent, files stay in Grajek's data directory and PATH stays unchanged.
- Startup absence opens the same panel. Playback/import failures caused by missing components reopen it through the existing controller request. Ordinary network, video or Discord failures do not become missing-component claims.
- Real download bytes and total drive progress. Verification, unpacking and component checks use named stages, not invented percentages. **Cancel download** cancels one owned job; **Keep downloading in background** closes only the panel.
- Failure retains an understandable reason, optional diagnostics and **Try again**. Only verified publication plus active resolver selection earns success. Success shows **Ready** and **Back to music** without automatically replaying a failed track.
- **Not now** and Escape return to the previous player/settings context with correct focus. Queue, history, Discord configuration and playback preferences remain intact.

## Engineering constraints
- Reuse current pinned artifacts in `packaging/components.json`, dependencies, download job, execution guards and controller snapshots. No second installer, new dependency, new window, web renderer, new animation, automatic download, commit, push or publishing.
- Windows downloaded tools live under LOCALAPPDATA/Grajek/tools. Preserve executable-adjacent tools as an existing installation fallback and PATH behavior. Honor GRAJEK_TOOLS=system; never claim downloaded components are active while that override prevents selection.
- Share execution/publication locking on both platforms. Checks must be asynchronous and bounded. Preserve previous usable components on failed or cancelled installation; keep startup crash recovery and cross-process locking.
- Preserve user-owned changes already present in app, tests, component resolver, main and Windows packaging. Do not modify vendored GPUI or installer behavior incidentally.

## Verification
- Exercise per-tool missing, invalid, timeout, unsupported version and success states without live Discord or external downloads.
- Exercise verified publication, corrupt/truncated archives, cancellation, retry, locked executables, concurrent instances and recovery; assert old usable tools survive failure.
- Exercise startup, settings and controller-error entry points, defer/reopen, background download, cancellation, completion, focus and keyboard controls without unintended controller commands or data loss.
- Inspect actual native rendering at 420×480 and 520×560 with long diagnostics, Details open, download, error and success states. Linux checks must remain green; Windows runtime evidence is required for Windows acceptance.
