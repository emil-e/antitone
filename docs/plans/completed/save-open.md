# Basic editor Save/Open — completed

Status: implemented. The original feature used schema 3; [native-camera replacement](camera-inversion.md) and [geometry integration](flip-crop-rotate.md) establish current schema/model 4. Recovery, relink/source-change UI and undo remain separate [work](../durable-projects.md).

## Scope and result

The editor owns authoritative document state, path and saved snapshot with stable source-ID photo edits. Explicit Save/Open persists accepted roll/photo/calibration/output/development state; transient picker/view/worker/GPU state remains outside the project.

- Existing-row File/Export menus, document/Modified status and Cmd-N/O/S/Cmd-Shift-S actions after accepted control edits.
- Background source registration, explicit saved WB on loads, staged Open/New/Import, stale-request rejection and candidate source/preview validation before replacement.
- Atomic snapshots and relative Save As rebasing; no conflicting path-keyed photo-state map or worker-global WB.
- Save/Discard/Cancel dirty guards. Failed/cancelled Save prevents destructive continuation; failed Open preserves the old document even after Discard.
- Missing/changed/incompatible ordered RAWs reject Open. Unavailable nonordered records remain stored; calibration RGB is never resampled. Open selects the first ordered photo at Fit; empty filmstrips have no selected scan.
- RAW inputs cannot be project destinations, and Export cannot replace the open project. `.antitone.json` is a filename convention, not a substitute for content/runtime validation.

The [project contract](../../contracts/project-state.md) defines current persistence and compatibility; [architecture](../../architecture.md#persistence-boundary-and-known-gaps) describes ownership and remaining gaps.

## Verification and limits

Recorded checks cover stable IDs/settings/locators, relative/symlink Save As, unavailable sources, failed writes, dirty guards, stale results, pending numeric saves, destination protection, explicit WB and real-source Save/Open/output reproduction. The [archived implementation record](../../research/history/implementation/save-open.txt) retains the original schema-3 environment and results, including a locked-session zero-visible-frame failure and a later successful unlocked repeat.

Native file-picker cancellation/overwrite panels were not externally exercised end-to-end. Headless guard tests and scripted visible frames do not establish physical-input, monitor or broader-device acceptance. Recovery/autosave, missing-source/relink UI and undo remain unwired; basic Save/Open completion does not imply those features.
