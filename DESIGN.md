---
name: Grajek — Signal Ledger
description: A precise, warm native desktop controller anchored by real playback state.
colors:
  canvas: "#191B18"
  surface: "#232620"
  hover: "#353A30"
  pressed: "#42493B"
  text: "#EFEDE5"
  muted: "#ADAEA3"
  control-border: "#777D6D"
  separator: "#3D4338"
  brass: "#DDC28B"
  brass-hover: "#ECD4A4"
  brass-pressed: "#C6A76B"
  brass-ink: "#1E231A"
  healthy: "#ACC69B"
  danger: "#EDA99B"
  danger-fill: "#592F2A"
  danger-hover: "#743C32"
  danger-pressed: "#482721"
typography:
  app-title:
    fontSize: "18px"
    fontWeight: 600
  track-title:
    fontSize: "24px"
    fontWeight: 600
  body:
    fontSize: "14px"
  label:
    fontSize: "12px"
  measurement:
    fontSize: "12px"
  volume-measurement:
    fontSize: "14px"
rounded:
  control: "4px"
  large: "6px"
spacing:
  tight: "4px"
  group: "8px"
  button-inset: "10px"
  detail: "12px"
  section: "16px"
  window-inset: "16px"
components:
  button-primary:
    backgroundColor: "{colors.brass}"
    textColor: "{colors.brass-ink}"
    typography: "{typography.body}"
    rounded: "{rounded.control}"
    padding: "0px 10px"
    height: "32px"
  button-primary-hover:
    backgroundColor: "{colors.brass-hover}"
  button-primary-active:
    backgroundColor: "{colors.brass-pressed}"
  button-neutral:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.text}"
    typography: "{typography.body}"
    rounded: "{rounded.control}"
    padding: "0px 10px"
    height: "32px"
  button-neutral-hover:
    backgroundColor: "{colors.hover}"
  button-neutral-active:
    backgroundColor: "{colors.pressed}"
  button-ghost:
    textColor: "{colors.text}"
    typography: "{typography.body}"
    rounded: "{rounded.control}"
    height: "32px"
  button-danger:
    backgroundColor: "{colors.danger-fill}"
    textColor: "{colors.danger}"
    rounded: "{rounded.control}"
    height: "20px"
  url-input:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.text}"
    typography: "{typography.body}"
    rounded: "{rounded.control}"
    height: "32px"
  section-tabs:
    backgroundColor: "{colors.canvas}"
    textColor: "{colors.muted}"
    height: "30px"
  playback-state-symbol:
    typography: "{typography.label}"
    size: "12px"
  list-row:
    typography: "{typography.body}"
    textColor: "{colors.text}"
    rounded: "{rounded.control}"
    padding: "0px 8px"
    height: "36px"
  error-callout:
    backgroundColor: "{colors.danger-fill}"
    textColor: "{colors.danger}"
    typography: "{typography.label}"
    rounded: "{rounded.control}"
    padding: "4px 8px"
---

# Design System: Grajek — Signal Ledger

## Overview

**Creative North Star: "Signal Ledger"**

Grajek is precise, warm, quiet and independent. Premium comes from readable hierarchy, consistent geometry and native interaction quality. The dominant track title, inline playback state and measured time express the existing identity without artwork, meters or animation. The player refinement corrects native scale, spacing and component rendering within the established palette and composition.

This is the visual contract for a native Linux and Windows desktop application built with Rust and GPUI Kit 0.7.3. Frontmatter tokens are normative; `src/style.rs` supplies the runtime palette and theme. Per-control geometry and type also live in the player source. Controller snapshots own behavior; `.impeccable/surfaces/player.md` records the player composition. Extend existing native controls rather than adding a web renderer or another component system.

**Key Characteristics:**

- Warm charcoal surfaces with restrained brass emphasis.
- Prominent track type and monospaced measurements.
- Informational playback symbols paired with explicit state text.
- Flat composition grouped by proximity, with visible native affordances.
- Compact controls, disclosed secondary actions and flexible lists.

### Evidence and documentation boundary

The player refinement is implemented in `src/app.rs`, `src/app/{controls,queue,history,progress}.rs`, `src/style.rs`. The Signal Ledger palette and native font preferences are preserved.

Native evidence comes from `native_player_redesign_evidence` in `src/app/tests/native_ui.rs`. Matching baseline and refined captures live in `.impeccable/review/refinement/{before,after}/{state}-{420,520}.png`; the suffixes mean 420×480 and 520×560. The base matrix includes idle, playing with populated/empty queue, focused/unfocused URL, paused, connecting, resolving, importing with an error, playback error, disconnected, full queue, History, Queue options, More and setup. `GRAJEK_CAPTURE_DIR` selects the destination. `GRAJEK_CAPTURE_INTERACTIONS=1` adds input/button hover, pressed button and keyboard focus on transport/queue actions. The fixture uses realistic long titles and Unicode, applies production snapshot subscriptions, and verifies the native Pause intent/acknowledgement and typing/Ctrl+Enter path. These captures replace the rejected visual conclusion of the previous review; tests alone do not establish craftsmanship.

Linux captures use the actual GPUI X11 window through KDE XWayland. `scripts/capture-native-window.py` resolves only the fixture PID/caption to its KWin UUID and uses ScreenShot2 CaptureWindow directly, validating dimensions before saving compositor pixels. This avoids the desktop-focus race in the older Spectacle active-window driver. GPUI `render_to_image` is unsupported on Linux. The capture helper requires Python D-Bus, GLib and Pillow; it changes no desktop focus and is not an application runtime dependency. Offline snapshots establish only the states and local layout they show. They do not establish live Discord audio, the native Wayland backend, Windows runtime behavior, assistive-technology behavior or fractional scaling.

Verification covers the full Cargo suite, strict Clippy, formatting, native build and the separate opt-in capture test. Regression tests cover keyboard-only disclosures, explicit playlist import, focus recovery, phase-specific headlines and real paused transport combined with a full queue, long Discord context, import feedback and errors at both supported sizes. An HTML/CSS detector does not apply to this native Rust/GPUI surface.

`.impeccable/design.json` remains stale and intentionally unchanged under the requested write boundary. Its translated HTML/CSS previews are not native implementation APIs. `PRODUCT.md` also retains older player wording and GPUI minimize-only claims; those claims are not adopted here. The local surface brief remains ignored by the existing repository rule, which is unchanged.

The previous theme used a 14px rem, making `p_4` 14px, `text_sm` 12.25px, `text_xs` 10.5px and kit medium buttons 28px despite the documented targets. The theme now uses a 16px rem and the player explicitly selects 14px body text, so utility sizes match their intended pixels.

## Product experience and native interaction contract

The 2026-10-30 experience pass supersedes the earlier player-only composition and unchanged-setup scope above. Signal Ledger colors, fonts, type roles and native integration remain unchanged.

Configured settings lead with a compact factual bot status and **Playback destination**: one labeled native server Select, then one voice-channel Select. Refresh belongs beside each field. An explicit selected-destination Status remains controller-owned while browsing another server. Permission and unsupported-stage reasons stay inside the affected option; disabled names use one ellipsized line so their reason cannot collide with the next row, while the accessible name contains the full name and reason. Enabled names can wrap to two lines. Menus have a 224px maximum list height and use the kit's window-aware flip/clamp. Credential editing and Preferences are disclosures. Settings has an explicit scroll handle, scrollbar gutter, and focus-revealing wrappers for offscreen controls. Changing a live destination still invokes the controller's Stop-and-switch confirmation, with explicit focus on the confirmation and Escape/Keep playing cancellation.

First run keeps token entry visible, masked, and labeled, with OS-vault versus session-only semantics and an optional creation/invite guide. No tutorial storage or new wizard is added. Authentication reveals destination selection without joining voice. The validated returning destination keeps the existing startup bypass. Empty playback invites a link; a pending queue retains a usable Play Queue action. Idle has no disabled Pause/Stop or repeated Ready phase line. Idle volume sits beside the link label. The active track, timeline and transport retain their existing ownership and geometry. Stop remains available while an accepted Play/Next request awaits its controller snapshot.

Help is now application-owned using Base TooltipOverlay/TooltipRequest and its anchor-aware Positioner. A 300px maximum-width surface wraps within viewport-minus-32px dimensions. Open/close transitions cancel both visible and delayed help, and help is suppressed while menus or settings are active. Pointer ownership prevents an old trigger's leave event from hiding a newer tooltip; keyboard F1 help is not dismissed by pointer modality changes. Escape, ordinary key navigation, pointer actions and window activation changes dismiss help. Required Queue options and More explanations are inline. No dependency or priority/z-order workaround was added.

Semantic rhythm: 4px title/status and label/control pairs, 8px related controls, 12px idle support, 16px conceptual separation and root inset. At 480px height the link group uses 4px gaps; only stacked playback-error/import feedback also compresses root section gaps to 4px, retaining two History rows and full-size controls. Queue insertion uses the existing confirmed count/list update: an extra status row was rejected because it moved scrolled content on routine snapshots. No optimistic playback/selection or extra animation was introduced.

Evidence and the independent Impeccable/UI UX Pro Max assessments are in `.impeccable/review/experience/REPORT.md`; the implementation specification is `docs/superpowers/plans/2026-10-30-product-experience.md`. The image gallery shows compositor captures, not an HTML implementation. Native XWayland evidence does not establish native Wayland, Windows runtime, live Discord audio, assistive technology or fractional scaling acceptance.

## Colors

The palette combines warm charcoal, warm white and brass. Frontmatter colors are the established opaque sRGB values from `src/style.rs`; preserve semantic roles rather than choosing new colors per component.

### Primary

- **Brass**: the current primary action, playing/paused state, selected tab text/underline, focus, caret and slider thumbs. Active repeat receives subtle text emphasis on its ghost disclosure trigger, with its mode available in the tooltip and accessible label.
- **Brass hover / pressed**: native primary pointer states.
- **Brass ink**: dark text on filled brass controls.

### Semantic

- **Healthy**: the small Discord connection dot; healthy connection text stays muted.
- **Danger**: failed connection, error state and error text.
- **Danger fill / hover / pressed**: error-callout backing and native destructive-button states. Stop stays ghost because stopping playback is reversible.

### Neutral

- **Canvas**: root window, lists, tabs and scrollbar backing.
- **Surface**: input, neutral controls and native popovers.
- **Hover / pressed**: neutral interaction, row hover and native selection fill.
- **Text / muted**: primary content versus connection, counts, measured time, empty guidance and placeholders.
- **Control border**: neutral-button/input boundaries, slider bar and scrollbar thumb.
- **Separator**: quiet structural rules, never the only actionable-control boundary.

**The One Primary Rule.** The main player has at most one filled brass action: usable Pause/Resume while Playing/Paused; Play now in other phases. Add to queue, Next, Stop and disclosure triggers retain their native neutral or ghost treatment. State, focus and selection may use brass without becoming competing filled actions.

### State and contrast contract

Selected tabs use brass text and underline. Text selection uses `pressed`; list active border uses brass. Playback error presence overrides playback-state color. Otherwise Playing/Paused use brass and other phases muted text. Disabled availability remains controller-owned and uses kit subdued, noninteractive states.

Check actual background pairings: target at least 4.5:1 for normal text and 3:1 for large text and actionable boundaries. Color accompanies explicit words or a playback symbol. Preserve a visible focus boundary or kit inner focus line on interactive controls; informational state symbols have no focus treatment.

### Theme strategy

Dark remains the existing default. `style::init` calls `Theme::change(ThemeMode::Dark, None, cx)` then `Theme::update(cx, ...)`; update palette, font size, radii and shadow policy through this synchronized API. Mutating `Theme::global_mut` alone can leave component tokens or the Base scrollbar theme out of sync. Light mode and a theme toggle are not implemented or verified.

## Typography

**Sans family:** Linux chooses once at startup from installed `Noto Sans`, then `Inter`, then `DejaVu Sans`. Windows prefers installed `Segoe UI`, then `Segoe UI Variable Text`, then `Arial`. Otherwise retain the kit-resolved system family. These are native candidates, not a CSS stack. Do not download fonts. Frontmatter omits a fixed family because resolution depends on the machine.

**Measurement family:** preserve `theme.mono_font_family` and its installed-family resolution. Mono is reserved for measured time, volume percentage and queue positions, not ordinary control labels.

### Hierarchy

- **App title:** `app-title`, semibold, for Grajek in the compact header.
- **Track title:** `track-title`, semibold, for the dominant single-line title or factual no-track headline. A no-track headline uses muted text.
- **Body:** `body`, for row titles, empty guidance and default controls.
- **Label:** `label`, for connection, phase, counts, errors and notices.
- **Measurement:** `measurement`, with mono, for elapsed/duration, history duration and queue positions.
- **Volume measurement:** `volume-measurement`, with mono, in a reserved right-aligned slot.

The track title is 24px semibold on a 30px line; body/control/list titles and volume are 14px. Connection and playback metadata use 12px on a 16px line; errors use the same 12/16 rhythm. Measurements and queue positions retain the native mono family. Kit sizing uses a 16px rem, independently of the player's 14px body role. Preserve the same hierarchy at minimum size rather than shrinking text.

**The Measured Data Rule.** Align changing digits with mono. Keep titles on one line with an ellipsis and full-text tooltip; do not shrink body titles below their established size to solve density.

## Layout

The root is a vertical flex composition with a true 16px inset and 16px section gaps. At a viewport height of 480px or less, section gaps compress to 8px while type and controls keep their sizes. `style::space` names the shared 4px tight, 8px group, 12px detail and 16px section/inset values; kit utilities use the same 16px rem. Title/state/timeline/transport use a 4px internal cadence, the input/actions use 8px, and transport/volume retain at least 12px separation. Outer overflow stays hidden. Default window: 520×560; minimum: 420×480. These are native constraints, not web breakpoints. Larger windows reveal more list rows rather than enlarging controls.

The player orders: compact header; dominant title/state/time and conditional seek slider; transport with bounded volume on the same row; optional playback/seek error; full-width YouTube input and link actions; independent import status when present; underline Up next/History navigation with Queue options; flexible scrolling list. Spacing separates the player from URL entry; no player separator is needed. Input and transport remain fixed while lists scroll. An absent timeline creates no empty sibling or residual gap.

Established player geometry:

- Input, transport, Play now, Add to queue and More share a 32px height and 4px radius. Native buttons retain 10px horizontal padding, 14px text and 16px icons.
- Pause/Resume reserves 96px; Next/Play Queue reserves 112px; Stop is a 32px square ghost icon with an explicit accessible label and tooltip. Play Queue omits the skip icon so its full label fits.
- Volume occupies a bounded 120px region beside transport. The transport group retains its width instead of stretching Pause.
- State symbol is 12px and unboxed; no square stamp or reserved decorative tile.
- Rows are 36px high; three always-visible 24px native icon buttons form one compact group. Queue action ink is muted, with disabled actions further subdued. Queue positions reserve 24px; history duration reserves 48px.
- Underline tabs retain native small height. Popover content is 240px wide.

Lists use flexible height, zero minimum height and native vertical scrollbars. Titles grow and truncate; positions, durations and actions retain their slots. Preserve two full rows in the stressed minimum-size layout, including error and import feedback. Avoid a redundant pending-count line or an additional queue toolbar.

Windows retains the native titlebar and GPUI DPI-aware window/input APIs, the same palette and compact dimensions, and installed platform typography. Native Windows interaction and mixed-DPI behavior require runtime verification.

## Elevation & Depth

No decorative surface shadows: `theme.shadow = false`. Depth comes from tones, fine separators, control borders and native interaction states. Popovers use the existing surface tone. The kit's `theme.focus_ring = false` selects its supported single-line focus rendering: bordered controls tint their boundary; filled buttons draw a contrasting inset line; ghost buttons draw an edge line. Keyboard focus stays visible without an outer halo. Slider wrappers keep their reserved focus boundary.

**The Flat Surface Rule.** Separate content through alignment, tone and a quiet rule. Do not add raised cards, glass, gradients or glow to manufacture hierarchy.

## Shapes

Use the frontmatter control radius and existing large-radius token. List rows and error callouts use the control radius; slider thumbs and the small connection dot stay round. Native neutral buttons and inputs retain their boundaries. Filled primary and ghost buttons retain native borderless geometry.

The playback-state symbol has no background, border, button geometry, hover behavior or focus target. Volume and seek wrappers reserve a transparent border at rest and switch to brass on focus without shifting layout.

## Components

### Buttons and priority

Keep native `Button`, theme variants, keyboard handling, disabled logic and accessible labels. Pause/Resume is the filled primary while usable in Playing/Paused; Play now supplies that emphasis in other phases. Play now remains available according to the existing controller gate even while playback controls carry the visual priority. Add to queue and Next remain neutral; Stop stays ghost.

Row actions remain always visible native ghost icon buttons with explicit accessible labels and tooltips. First/last queue moves retain disabled logic. Clear history keeps its existing two-step confirmation, danger Yes, clear and separate Cancel.

### Header and Discord context

Keep Grajek, factual connection status and destination context distinct. The connection dot and text report actual availability/voice state; a selected server/channel does not imply connection. Safely truncate server/channel names separately from connection status and retain full-text tooltips. Leave voice remains a visible small ghost action. Settings uses a 32px gear icon with a Discord settings accessible label/tooltip; it becomes a back arrow with Back to player semantics in settings. Both remain keyboard reachable at both sizes.

Leave voice stops audio and leaves voice while preserving pending entries. Stop only stops audio. Controller snapshots own leaving/success/failure states. Losing the focused control restores focus to the URL input. Do not encode tray behavior in this header contract.

### Playback identity, time and seek

The dominant title precedes an inline state row: a 12px informational symbol, explicit phase and monospaced elapsed/duration. Playing uses Play; Paused uses Pause; Ready/Error use Square; transitional phases use Ellipsis. The symbol is not interactive and is never rendered as an outlined transport stamp. Existing Preview and Seeking… status temporarily occupy the phase text slot during seek interaction.

Without a visible track, the headline follows actual phase: Ready for a link, Starting Grajek…, Joining voice…, Preparing your track…, Stopping playback…, Leaving voice… or Playback stopped. Unavailable Ready/Error instead shows Connect to Discord; unavailability must not erase a real leaving/starting transition. A visible track lacking a title uses Untitled track.

Known nonzero duration exposes the existing native progress slider beneath title/state/time. Unknown or zero duration and live media retain elapsed time without a fabricated seek range. Drag previews remain local; release sends one controller-owned seek and returns to confirmed position until the result arrives. Preserve track identity, disabled capability checks, native accessible seek actions and focus recovery. Focused Left/Right, Page Down/Up, Home and End retain their existing meanings.

### Volume

Keep the speaker icon, native slider and right-aligned mono percentage inside the bounded transport-row region. Volume2/VolumeX follows the actual percentage. Bar uses control-border; thumb uses brass. Preserve Tab availability, the reserved focus border, arrow/Page/Home/End handling, live watch updates, release persistence and controller rollback. This is a composition change; volume ownership remains unchanged.

### Playback and seek errors

The danger callout sits immediately below the playback group, before link entry. It displays the playback error or, when absent, the seek failure. Keep native Alert semantics, a maximum two-line clamp, full-message tooltip, existing padding and control radius. Independent import feedback must not replace this error or alter playback phase.

### Link input, More and playlist import

Use the existing full-width native input with the current Paste YouTube link placeholder and accessible label. Preserve input handling, selection, focus/caret, controller gates and test-support identifier. Enter invokes Play now; Ctrl+Enter adds a single video to the queue. A watch link containing a playlist still keeps single-video behavior for those actions. Pasting alone never starts playback or import.

The URL frame is the sole owner of its 1px boundary; the embedded native `Input` uses `bordered(false)` and keeps editing, selection and accessibility. The kit otherwise paints both its input border and an outer focus ring. A neutral boundary becomes slightly brighter on hover and brass on focus, without changing geometry. This frame supplies hover styling because kit Input does not expose that builder API. Do not add another wrapper border or suppress the replacement focus indicator.

URL focus retention is intentional: startup/Play prepare the next link, native mouse buttons preserve typing focus, and disabled-control recovery returns here. Snapshot updates do not automatically blur the field. Capture focused and unfocused states explicitly.

Below the input, group Play now, Add to queue and the ghost More disclosure together with 8px gaps. More uses the native `Popover`, a trailing native chevron, a left-aligned Import playlist action and concise aligned helper text. Selecting import uses the current link, closes More and focuses the input; it never starts or interrupts playback.

Import feedback is an independent native Status region with a full accessible message and tooltip. During an import, Cancel import remains visibly available beside that status without reopening More. Preserve cancellation, pending-command protection, capacity rules and focus recovery when import finishes. No fabricated progress percentages or automatic paste action are introduced.

### Queue options and navigation

Use native underline tabs labelled Up next · N and History. The pending count appears once in Up next, with · Full at capacity. Queue options sits alongside the tabs and is available from History too. No pending-count header or separate queue-preferences toolbar remains.

Queue options is a native `Popover` with left-aligned Repeat, Remember queue, Continue on error and Clear queue actions and a trailing native chevron. Its small ghost trigger uses subtle brass text when repeat is active; tooltip and accessible label expose the actual repeat mode. It does not become a second filled primary. The repeat row labels and cycles Off → Repeat Track → Repeat Queue, preserving saved controller semantics: only successful natural completion repeats, and Next/Stop bypass repeat. Remember queue and Continue on error render saved preferences without optimistic state.

Clear queue removes pending entries and cancels an active import while preserving the current track. It is disabled until the controller is ready and whenever both queue and import are absent. Keep its access from either tab. History retains its own saved count, notices and clear confirmation.

Native popover triggers retain keyboard activation. Scoped `PlayerMenuControl` Enter/Space activation keeps menu controls from losing activation to the popover binding. Escape closes a popover and returns focus to its trigger. Selecting import closes More and focuses the input. Section changes close Queue options; opening Settings or losing gateway availability closes both menus. Gateway loss also recovers input focus. Ctrl+1/Ctrl+2 switches Up next/History; do not claim arrow-key tab navigation was added to kit tabs.

### Lists and empty states

Rows retain one-line titles, full-title or explanatory URL tooltips, mono positions/durations and always-visible actions. Loading title… and Title unavailable remain factual and playable according to controller gates. Hover uses the native list-hover color. Queue/history scrolling, keyboard row navigation and accessible labels remain unchanged.

Empty guidance is truthful and left-aligned near the list top, with existing row inset and top padding. Avoid decorative icons or a centered hero. Persistent History notices remain metadata-sized and clamp to two lines.

### Discord setup and settings

Settings retain their existing flow and controls; shared rem, focus and header refinements also apply there. Keep compact native settings, masked token entry, explicit secure-storage/session-only actions, progressively disclosed help and named server/channel choices. Disabled choices explain recovery; full-name tooltips and visible keyboard focus remain. Confirm active-playback interruption before changing destination. Setup captures are regression context, not evidence of a new settings design or verified tray wording.

### Motion and rendering

No new application animation, polling or frame-request loop belongs to this composition. Native hover/focus and existing kit loading behavior remain. Preserve elapsed snapshots, controller-owned seek behavior and volume coalescing. Never animate the state symbol as an audio meter or infer changes to kit animation defaults from the redesign.

## Do's and Don'ts

### Do:

- **Do** preserve Signal Ledger colors, native controls and installed platform font resolution.
- **Do** lead with the title and factual playback state, then transport and bounded volume.
- **Do** retain one filled primary, visible small header actions and disclosed secondary actions.
- **Do** preserve the root inset, fixed controls, flexible lists and 36px row rhythm.
- **Do** keep explicit state text, visible focus, accessible labels and full-text tooltips.
- **Do** verify both native sizes, popovers, long text, loading, errors and disabled states against the final source.

### Don't:

- **Don't** restore the outlined stamp, a separate volume row, inline preference controls or redundant pending count.
- **Don't** introduce artwork, waveforms, invented playback positions, optimistic phase changes or loading percentages.
- **Don't** add a retro hardware skin, music-note ornament, streaming-service navigation or SaaS dashboard vocabulary.
- **Don't** hide essential row actions until hover or remove focus feedback for aesthetics.
- **Don't** treat selected destination context as proof of connection or offline captures as live-service/platform acceptance.
- **Don't** treat stale sidecar previews or older PRODUCT tray claims as authority for this redesigned player.
