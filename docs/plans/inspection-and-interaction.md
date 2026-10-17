# Interaction and colour diagnosis

These are unfinished fixes and design questions. [The roadmap](../../PLAN.md) owns priority; [editor](../editor.md) and [geometry](../geometry.md) explain current behavior.

## Decisions and open questions

- Reproduce DSC04524 dark reds with the exact RAW and grade, tracing camera RGB through inversion, AP1 grading and rendering. The cause is not established.
- Investigate cold near-base Vision3 500T shadows. Greater blue sensitivity is a hypothesis, not permission to warm shadows or add a pedestal.
- Reproduce stale fitted-resolution symptoms across Fit/native/resize/switching. Preserve refinement and existing numerical/visibility gates.
- Exercise physical pointer/trackpad/keyboard/VoiceOver behavior, numeric/popup precedence, view restoration and normal-app preference relaunch. Headless and scripted window checks cannot establish these.

## Triaged interaction follow-up

### Focused fixes

- **Busy tool identity:** investigate Apply/Cancel auto-ID changes when the tool icon becomes a spinner. Verify focus and in-progress input ownership across the transition before choosing a correction.
- **Decision-dialog acceptance:** exercise the shared Save/Discard, project/output replacement and Analyze prompts, plus acknowledged flat-field/save/export failures and working modal/inline/activity feedback, with native keyboard/pointer input, VoiceOver and display scaling. Component/event/float-capture checks establish layout and transaction boundaries, not physical appearance or native chooser ownership. [Current policy](../editor.md#decisions-are-not-operation-failures).
- **Wheel accessibility:** settle a named pointer-only surface with adjacent keyboard-editable numeric controls versus a dedicated wheel keyboard interface. Preserve the numerical mapping; do not introduce an inert focus stop. Physical/VoiceOver acceptance remains separate from card/title semantic and input checks.

### Roll workflow

- **RAW drag/drop:** settle supported-file feedback, busy drops, duplicates and all-or-nothing failure through staged import/compatibility checks.
- **Selection/removal acceptance:** exercise [multi-selection and confirmed project-only removal](../editor.md#select-and-remove-roll-photos) with physical Cmd/Shift input, focused cards, context menus, VoiceOver and display scaling. Controller/authored-RAW checks cover same-index fallback, empty Save/Open/reimport, retained correction/calibration and input/transaction gates; they do not establish native interaction or appearance. Undo remains separate.

### Layout and controls

- **Application-shell acceptance:** exercise the [native menu and Settings workflow](../editor.md#application-menus-and-settings) with physical keyboard/pointer input, VoiceOver and display scaling. Include focused numeric/text commands, native file-panel editing and return focus, disabled commands during tools/work, Settings relaunch and inspection transitions. Scripted AppKit checks cover installation, state, accelerators, activation wakeup and native-panel Edit restoration—not those physical/lifecycle boundaries. Windows/Linux host/backend work and photographic undo remain separate.
- **Read-only text selection:** extend native Copy availability to selectable-label selections, not only focused text inputs, while preserving egui's existing keyboard/context-menu copying.
- **Native termination acceptance:** exercise the [guarded native termination workflow](../editor.md#native-quit-negotiation) with physical Dock input, native chooser cancellation/return focus, pending saves, inspection and actual logout/shutdown negotiation. Isolated-process checks use ordinary termination messages only, not a real system logout/shutdown; forced termination cannot promise a save opportunity. Keep the pinned winit delegate/entry-hook compatibility check explicit when upgrading the windowing stack.
- **Operation feedback:** coordinate progress/errors with [the feedback slice](completion.md#operation-feedback), preserving transactions and cancellation boundaries.
- **Fullscreen inspection acceptance:** exercise quick F/Escape entry/return, auto-hide discovery, zoom/source controls, suspended drafts and exact working-view restoration with physical input and VoiceOver. Include pre-existing native fullscreen, focus loss/return, closing while inspection is active, scaling and monitor/Space changes; verify retained system menu/Dock access and presentation restoration. The [bounded current-display/native paint gate](../development.md#explicit-native-host-checks) does not establish these broader boundaries.
- **Crop cursor acceptance:** exercise the native rotation cursor around edges/corners, through straightening/move/resize, and across popup, canvas/window-exit and monitor transitions. Headless checks establish bitmap/output mechanics, not native tracking, DPI appearance or photographic acceptance.
- **Wheel feel/mapping:** exercise normal/Shift-fine dragging and modifier transitions with physical input; reproduce any directional asymmetry before changing mapping. Gamma and Bradford WB have different processing responses despite similar artwork; [processing](../processing.md#curve-balance-is-not-white-balance) explains why. Further sensitivity/keyboard changes must not quietly change numerical mapping.

## Next action and acceptance

Select one concrete symptom or design question, reproduce it in the current composed editor, and verify the focused fix. Preserve full-resolution picks, source anchors, draft/accepted separation, input precedence and exact working-context restoration. Physical feel and monitor appearance need real-use evidence.

[Thumbnail follow-up](visual-feedback.md) · [Photographic and release work](completion.md).
