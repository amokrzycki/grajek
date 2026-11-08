# Playback components recovery screen

## Intent
Make the Windows missing-components screen a finished recovery surface, not text stranded in a corner. The user approved the existing-window approach on 2026-11-08.

## Approaches
- Selected: a compact, vertically centered recovery block in the existing window. Preserve native window management and Signal Ledger styling.
- Rejected: a separate small dialog, which introduces window lifecycle and return-to-player handling for the same information.

## Design
- Operate mode. Preserve native GPUI controls, English application language, existing palette and installed fonts.
- Use a bounded, left-aligned content block with generous outer space, a clear heading, readable explanation, and a distinct installer-repair instruction.
- Replace ambiguous “OK” with “Continue without playback”. Keep the existing dismissal behavior; do not claim an automatic repair or introduce downloads.
- Maintain usable composition at 420×480 and 520×560, including longer text.
- Make the visible action keyboard reachable and restore appropriate player/settings focus after dismissal.

## Scope and constraints
Change the components notice and its tests only, with narrowly necessary focus handling. Preserve controller ownership, the player, settings, existing unrelated edits and installer behavior. No dependencies, animation, web renderer, new window, commit or publishing.

## Verification
Exercise visibility, mouse/keyboard dismissal, focus recovery and absence of unintended controller commands. Inspect actual native rendering at both sizes. State any unavailable runtime checks explicitly.
