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
    fontSize: "22px"
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
  window-inset: "16px"
components:
  button-primary:
    backgroundColor: "{colors.brass}"
    textColor: "{colors.brass-ink}"
    typography: "{typography.body}"
    rounded: "{rounded.control}"
    padding: "0px 10px"
    height: "28px"
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
    width: "64px"
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
  transport-stamp:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.brass}"
    rounded: "{rounded.control}"
    size: "40px"
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

Grajek is precise, warm, quiet and independent. Premium comes from readable hierarchy, consistent geometry and native interaction quality. One useful transport stamp makes real playback state recognizable without artwork, meters or animation. The track is the visual anchor; the surrounding controls stay compact and familiar.

This is the lasting visual contract for a native Linux desktop application built with Rust and GPUI Kit 0.7.3. Product behavior belongs to `PRODUCT.md` and controller snapshots; the first player composition belongs to `.impeccable/surfaces/player.md`. The implementation is the evidence for these tokens. Frontmatter values are normative; `src/style.rs` is the runtime authority. Extend the existing native controls rather than introduce a web renderer or another component system.

**Key Characteristics:**

- One state-bearing signature, used sparingly.
- Warm charcoal surfaces with restrained brass emphasis.
- Prominent track type and monospaced measurements.
- Flat, ruled composition with visible native affordances.
- Compact controls that remain usable at the minimum window size.

### Direction selection

Three signature concepts were evaluated against the supplied brief; the direction was chosen independently as requested.

| Direction | Signature | Product fit | Limitation |
| --- | --- | --- | --- |
| Signal Ledger (chosen) | Transport stamp beside the title; measured time; compact ruled lists | Useful when idle, playing or failed; fits supported native primitives | Requires careful truncation and muted color balance |
| Audio console | Tactile transport strip and hardware-like panel | Familiar transport vocabulary | Adds weight and risks a retro hardware costume |
| Editorial playlist | Oversized track typography with a continuous queue narrative | Strong title hierarchy | Makes a small controller feel like a library and competes with link entry |

Signal Ledger wins on product fit, originality, compact usability and implementation feasibility. Its signature needs no new data, assets, dependencies or timers. The stamp is a native composition using existing Lucide transport assets.

## Colors

The palette combines warm charcoal, warm white and brass. Frontmatter colors are exact opaque sRGB values extracted from `src/style.rs`; preserve semantic roles rather than choosing new colors per component.

### Primary

- **Brass** (`brass`): Play URL, usable Resume, playing/paused stamp and phase, selected tab text/underline, focus, caret and volume thumb. Warning notices reuse this color with explicit words.
- **Brass hover / pressed**: native primary pointer states.
- **Brass ink**: dark text on filled brass controls.

### Semantic

- **Healthy**: the small Discord connection dot; healthy text stays muted so it does not compete with the track.
- **Danger**: failed connection, error phase/stamp, error text and destructive confirmation text.
- **Danger fill / hover / pressed**: error-callout backing and native destructive-button states. Stop stays ghost because stopping playback is reversible.

### Neutral

- **Canvas**: root window, lists, tabs and scrollbar backing.
- **Surface**: input, stamp, neutral controls and native popovers.
- **Hover / pressed**: neutral interaction, row hover and native selection fill.
- **Text / muted**: primary content versus connection, counts, measured time, empty guidance and placeholders.
- **Control border**: neutral-button/input boundaries, slider bar and scrollbar thumb.
- **Separator**: quiet structural rules, never the only actionable-control boundary.

**The One Signal Rule.** Brass communicates an action, selection, focus or real playback/warning state; do not spread it across decoration or repeat the stamp in headers and rows.

### State and contrast contract

Selected tabs use brass text and underline. Text selection uses `pressed`; list active border uses brass. Loading displays its real phase and ellipsis, without fabricated percentage. Error presence overrides stamp/phase color. Disabled availability remains controller-owned and uses kit subdued, noninteractive states.

Check actual background pairings: target at least 4.5:1 for normal text, 3:1 for large text and actionable boundaries. Do not treat disabled text or structural separators as active controls. Color accompanies words or a transport symbol. Keep a visible focus boundary or kit inner focus line; do not suppress focus for neatness.

### Theme strategy

Dark remains the existing default. `style::init` calls `Theme::change(ThemeMode::Dark, None, cx)` then `Theme::update(cx, ...)`; update palette, font size, radii and shadow policy through this synchronized API. Mutating `Theme::global_mut` alone can leave component tokens or the Base scrollbar theme out of sync.

Future light mode must retain these roles with warm paper, dark ink and a darker brass accent meeting contrast. Do not invert hex values or switch modes without a matching palette. Light mode and a theme toggle are not implemented or tested.

## Typography

**Sans family:** choose once at startup from installed `Noto Sans`, then `Inter`, then `DejaVu Sans`; otherwise retain the kit-resolved system family. These are native candidates, not a CSS comma-separated stack. Do not download fonts. Frontmatter omits a fixed `fontFamily` because the actual family depends on the Linux machine.

**Measurement family:** `theme.mono_font_family`, resolved by the kit to an installed family. The normal Linux default is DejaVu Sans Mono; the kit probes installed alternatives when needed. Preserve runtime resolution rather than passing an assumed family name.

The single sans hierarchy is direct and compact. Mono is reserved for measured time, volume percentage and queue positions, not control labels.

### Hierarchy

| Runtime role | Frontmatter token | Weight / source |
| --- | --- | --- |
| Grajek app name | `app-title` (18px) | Explicit semibold (600) |
| Track or “Nothing playing” | `track-title` (22px) | Explicit semibold (600); idle title muted |
| List titles and empty guidance | `body` (14px) | Native `text_sm()` |
| URL and default transport controls | `body` (14px) | Theme/native normal-size typography |
| Connection, phase, counts, errors, notices | `label` (12px) | Native `text_xs()` |
| Elapsed/duration, history duration, queue position | `measurement` (12px) | Native `text_xs()` plus installed mono |
| Volume percentage | `volume-measurement` (14px) | Native `text_sm()` plus installed mono |

Small underline tabs and extra-small utility buttons keep the kit's size typography. Phase and progress are 12px, not 14px. Do not invent line-height, tracking or display-family overrides where the source uses native defaults. Standard buttons retain kit relative 1.25 line height.

**The Measured Data Rule.** Use mono only where changing digits should align. Keep titles one line with an ellipsis and full-text tooltip; do not shrink body titles below 14px to solve density.

## Layout

Observed spacing: 4px for tight content, 8px for groups/row inset, 12px between stamp/title and phase/time, 16px around the window. Native normal buttons use 10px horizontal padding. Root is a vertical flex composition with an 8px gap, 16px inset and hidden outer overflow.

Default window: 520×560; minimum: 420×480. These are native constraints, not web breakpoints. Larger windows expose more list rows rather than enlarging controls. Fixed controls and flexible lists share aligned edges.

The first player surface orders name/connection header, full-width URL input, separate actions with a compact repeat selector, Now Playing, transport, optional error, volume, underline tabs and flexible list. Input/actions have a 12px separation (4px gap plus 8px bottom margin); do not squeeze a URL between two buttons. Preserve Enter to play and Ctrl+Enter to enqueue.

| Established geometry | Dimension |
| --- | --- |
| URL input | 32px high, full available width |
| Play URL / Add to queue | 28px high |
| Native transport controls | 32px high |
| Pause / Resume | Flexible width |
| Next (Play Queue while idle) / Stop | 116px / 64px wide |
| Stamp / internal icon | 40px square / 20px |
| List row / row icon button | 36px high / 24px square |
| Queue position column | 24px wide |
| History duration / volume percentage slot | 48px wide |
| Native small underline tab | 30px high |

Lists use `flex_1`, `min_h_0` and native vertical scrollbars; rows use `flex_none` so long queues cannot compress them. Titles grow/truncate; positions, duration and actions keep width. Input and transport never scroll away. Preserve at least two full rows in the worst-case minimum layout.

## Elevation & Depth

No decorative surface shadows: `theme.shadow = false`. Depth comes from tones, fine separators, control borders and native interaction states. No nested card shells, glass, gradients or glow. Native focus rendering remains allowed even when the kit internally represents a focus line as a shadow; this is accessibility feedback, not surface elevation.

**The Flat Surface Rule.** Separate content with alignment, tone and a quiet rule; do not add raised cards to manufacture hierarchy.

## Shapes

Control radius: 4px; native large-radius token: 6px. Stamp, list rows and error callout use control radius. Slider thumb and 6px connection dot stay round. No arbitrary per-component rounding.

Neutral buttons, input and stamp retain 1px boundaries; separators are 1px and quieter. Filled primary and ghost buttons retain native borderless geometry. Volume wrapper reserves a transparent 1px border at rest, switching to brass on focus without a layout shift.

## Components

### Buttons

Precise native actions with deliberate priority. Keep `Button`, theme variants, sizing, keyboard behavior, disabled logic and accessible labels.

- Play URL is primary; Add to queue is neutral with its native border. Both are 28px high with factual labels.
- Pause is flexible neutral; usable Resume is primary. Next is neutral; Stop ghost. Keep 32px native transport height and fixed Next/Stop widths.
- Native variants own hover/pressed colors and focus treatment. Do not replace the kit focus line with a web rule.
- Row actions are always-visible 24px native ghost icon buttons with tooltip and explicit accessible label. First/last queue moves retain disabled logic.
- Clear queue/history are extra-small ghost buttons. History retains two-step confirmation, danger “Yes, clear” and separate Cancel.

### Queue experience controls (v0.3.0)

Repeat uses one 120px native button in the existing URL action row. Its explicit
label cycles Off, Repeat Track and Repeat Queue; active repeat uses the brass
primary variant. Its tooltip explains cycling and the natural-completion rule.
Remember queue: Off/On is an extra-small native ghost button beside the section
tabs, visible in both sections. Both render saved controller preferences and keep
native Tab/Enter/Space handling. No new vertical row reduces compact list space.
The existing Next transport is 116px wide to fit Play Queue while idle; its label
and availability come from the controller snapshot. Pause remains flexible, Stop
64px. All controls retain full labels at 420×480, including long-error states and
two visible list rows. The theme, type, spacing, stamp and row actions are unchanged.

### Inputs / Fields

Full-width native `Input`, surface backing, contrasting border, brass focus/caret and native selection. Preserve placeholder “Paste YouTube URL…”, accessible label “YouTube video URL”, test-support ID, submit handling and focus recovery when transport becomes unavailable.

### Navigation

Use `TabBar::underline().small()`: canvas backing, muted inactive text, brass active text/underline. Keep Queue count and History labels. Kit tabs do not take keyboard focus; Ctrl+1/Ctrl+2 remain supported switching shortcuts. Do not claim arrow-key tab navigation was added.

### Volume

Existing speaker icon, native `Slider`, fixed-width right-aligned mono percentage. Bar uses control-border; thumb brass. Volume2/VolumeX follows the actual percentage. Focus uses the reserved wrapper border. Preserve Tab availability, custom keyboard support, live watch updates, release persistence and controller rollback.

### Lists and empty states

The Queue header includes a native extra-small ghost button, “Continue on error:
Off/On”. It renders the persisted controller preference, retains native Tab/Space
handling and explains the queued-only, three-failure limit in its tooltip. Recovery
messages use the existing error callout; no settings panel or visual system is added.

Rows use 8px horizontal inset/gaps, flexible one-line titles and full tooltips. Queue positions use two digits; history duration stays right-aligned in its reserved slot. Hover uses list-hover. Essential actions never appear only on hover.

Empty guidance is truthful and left-aligned near the top, with 8px horizontal and 12px top inset; no decorative icon or centered hero. Counts and persistent History notices use metadata type; notices clamp to two lines.

### Signature: transport stamp

The outlined 40px stamp is a status display, not a button. It appears once beside the title. Playing shows Play; Paused Pause; Initializing/Connecting/Resolving/Stopping Ellipsis; Ready/Error Square. Controller snapshots alone select symbol and explicit phase. Error presence takes precedence with danger color; playing/paused otherwise use brass, remaining phases muted.

Stamp/title separation is 12px. Now Playing has 8px vertical padding and one bottom rule. Keep the prominent title, explicit 12px phase and measured time readable without waveform or seeking affordance.

### Errors and micro-interactions

Error callout uses danger-fill, danger text, 4px/8px padding, three-line clamp and full-message tooltip. Healthy connection is a small dot plus muted “Discord voice connected” or “Discord ready · voice idle”; initializing/unavailable use actual connection text. Color never carries meaning alone.

No application animations or extra frame requests are introduced. The v0.2.1 idle deadline is controller scheduling and requests no idle frames. Native hover/focus is sufficient; existing kit loading behavior is retained. Do not claim default kit animation behavior changed. Future transitions must express real state, remain roughly 120–180ms, respect reduced motion and leave status readable without motion. Never animate the stamp as an audio meter. Existing elapsed snapshots and volume coalescing remain the performance model.

## Do's and Don'ts

### Do:

- **Do** use the centralized semantic palette through `Theme::update` and preserve native kit variants.
- **Do** use one controller-driven stamp beside the track, paired with explicit phase text.
- **Do** preserve the 16px root inset, 8px main gap, fixed controls and flexible 36px scrolling rows.
- **Do** preserve installed Linux font resolution, the 22px track anchor, 14px body and 12px metadata/measurements.
- **Do** retain visible keyboard focus, accessible icon labels, full-text tooltips and existing test-support identifiers.
- **Do** inspect both native window sizes and error/disabled/loading layouts before extending the surface.

### Don't:

- **Don't** invent artwork, waveform data, seeking, optimistic playback or animated loading percentages.
- **Don't** add music-note ornaments, a retro hardware skin, streaming-service navigation or SaaS dashboard vocabulary.
- **Don't** add nested rounded cards, gradients, glows, glass, arbitrary shadows or inconsistent radii.
- **Don't** repeat the stamp in rows or let healthy connection status compete with the title.
- **Don't** hide essential row actions until hover, shrink body titles below 14px or remove focus rings for aesthetics.
- **Don't** assume CSS font stacks or sidecar HTML/CSS previews are native implementation APIs.
- **Don't** claim light mode, live Discord playback, audio or fractional-scale proof from synthetic native captures.

### Evidence and drift

Reconciled against `src/style.rs`, `src/main.rs`, `src/app.rs`, `src/app/{controls,queue,history}.rs` and installed GPUI Kit/component 0.7.3 source. Provisional 14px phase/progress is corrected to shipped 12px. Unused 20px/24px spacing steps are not canonized as a layout scale. Danger pressed color and native neutral-versus-filled/ghost border geometry are recorded from source.

`.impeccable/review/` contains `before-default.png` and `before-minimum.png` from the incumbent executable, plus `after-default.png` and `after-minimum.png` from the rebuilt production executable at 520×560 and 420×480. The after frames show missing-configuration feedback, disabled playback actions, focused URL input and an empty queue. They do not show synthetic playback or populated history. Local integration tests cover the eight controller phases, populated queue/history, long titles/errors, scrolling and commands; the captures establish local rendering/layout only. Neither screenshots nor tests establish live Discord connectivity, audio, compositor integration or fractional scaling. Final strict Cargo validation must come from actual command results, not be inferred from screenshots.

`.impeccable/design.json` uses schemaVersion 2 for documentation-panel extensions and self-contained translated component previews. Synthesized tonal ramps are exploration metadata, not runtime colors. Native source/frontmatter remain authoritative. No craft defect was canonized; absent light mode and live-service proof remain explicit limitations.
