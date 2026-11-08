# Components Notice Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish the Windows missing-components recovery screen within the existing window.

**Architecture:** Refine `PlayerApp::components_notice_surface` using native GPUI layout and existing theme tokens. Preserve notice dismissal and controller behavior, adding only the focus handling required for keyboard access.

**Tech Stack:** Rust, GPUI Kit 0.7.3, native Windows.

**Spec:** `docs/superpowers/specs/2026-11-08-components-notice-design.md`

## Global Constraints
- Preserve Signal Ledger colors, native controls, English copy and installed fonts.
- Support 420×480 and 520×560.
- No dependencies, new window, downloads, automatic repair, unrelated cleanup or commit.
- Preserve unrelated working-tree changes and all playback/controller behavior.

## Review Focus
- Long error text must wrap without clipping the action.
- Initial keyboard focus must reach the visible action, not a hidden editor.
- Dismissal must restore a visible player/settings focus target.
- Continuing must not send a playback or repair command.
- Minimum-size layout must retain readable text and a fully visible action.

## Task 1: Finish and verify the recovery surface

**Files:** Modify `src/app.rs` and `src/app/tests/notice.rs`; touch existing focus helpers only if necessary.

**Interfaces:** Retain `components_notice: Option<String>` and `components_notice_surface(&self, cx: &Context<Self>) -> AnyElement`. Keep the existing notice/action identifiers for compatibility.

- [x] Extend notice tests to cover both window sizes, a long notice, initial action focus, keyboard/mouse dismissal, restored visible focus and no emitted controller commands. Run the focused tests before changing implementation and record the failures.
- [x] Read Impeccable's craft floor, then implement the centered, bounded native layout with heading, explanation, separate repair guidance and “Continue without playback”. Reuse existing button/focus conventions; preserve the existing dismissal semantics.
- [x] Run focused notice tests, `cargo fmt --all -- --check` and `cargo check --locked`. Resolve failures introduced by this change without unrelated edits.
- [x] Capture and inspect actual native Windows rendering at 420×480 and 520×560; verify mouse and keyboard paths. Record any unavailable checks rather than claiming visual verification from tests.
- [x] Review the final diff against the approved scope and report results. Do not commit or publish.
