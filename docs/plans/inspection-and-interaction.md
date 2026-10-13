# Interaction and colour diagnosis

These are unfinished fixes and design questions. [The roadmap](../../PLAN.md) owns priority; [editor](../editor.md) and [geometry](../geometry.md) explain current behavior.

## Decisions and open questions

- Reproduce DSC04524 dark reds with the exact RAW and grade, tracing camera RGB through inversion, AP1 grading and rendering. The cause is not established.
- Investigate cold near-base Vision3 500T shadows. Greater blue sensitivity is a hypothesis, not permission to warm shadows or add a pedestal.
- Reproduce stale fitted-resolution symptoms across Fit/native/resize/switching. Preserve refinement and existing numerical/visibility gates.
- Exercise physical pointer/trackpad/keyboard/VoiceOver behavior, numeric/popup precedence, view restoration and normal-app preference relaunch. Headless and scripted window checks cannot establish these.

## Triaged interaction follow-up

### Focused fixes

- **Save prompt:** settle owner-relative placement and modality without changing Save/Discard/Cancel behavior.
- **Selection/accessibility:** cards lack accessible filename/selection metadata; pinned section titles lack label semantics; wheels lack a named keyboard interface beyond adjacent numeric controls. Rejected filmstrip selection actions still expose enabled card responses. Scope fixes with cold-load/error/disabled/AccessKit tests; paint equivalence does not close these gaps.

### Roll workflow

- **RAW drag/drop:** settle supported-file feedback, busy drops, duplicates and all-or-nothing failure through staged import/compatibility checks.
- **Selection/removal:** clarify selection modes, active/last-photo removal, next selection and in-flight work. Removal changes the project, never deletes RAWs or resamples calibration; undo is separate.

### Layout and controls

- **Scopes disclosure:** decide whether disclosure-style hiding improves the persistent visibility control without changing canvas/resource behavior.
- **Native shell:** app menus and owner-relative dialogs need design; no framework choice is established.
- **Operation feedback:** coordinate progress/errors with [the feedback slice](completion.md#operation-feedback), preserving transactions and cancellation boundaries.
- **Wheel feel/mapping:** reproduce the specific gesture first. Gamma and Bradford WB have different processing responses despite similar artwork; [processing](../processing.md#curve-balance-is-not-white-balance) explains why. Precision/keyboard changes must not quietly change numerical mapping.

## Next action and acceptance

Select one concrete symptom or design question, reproduce it in the current composed editor, and verify the focused fix. Preserve full-resolution picks, source anchors, draft/accepted separation, input precedence and exact working-context restoration. Physical feel and monitor appearance need real-use evidence.

[Thumbnail follow-up](visual-feedback.md) · [Photographic and release work](completion.md).
