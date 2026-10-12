# Recovery, source-change UI and undo

Status: unfinished persistence work. [Roadmap](../../PLAN.md) owns priority. Save/Open, stable-ID settings, explicit development WB and dirty-transition guards are available; [project-state contract](../contracts/project-state.md) defines the supported document.

## Goal and boundaries

Add recovery, source-change UI and undo/redo without reinterpreting grades or resampling RGB calibration. No catalog, automatic migration, derived images in JSON or silently overwritten explicit saves. `load()` structural validation and `require_runtime()` are distinct gates.

## Work and acceptance

1. Design missing-source/relink and explicit source-change UI without assuming byte immutability or a capture-identity scheme. Settle matching/candidate/acceptance policy from workflow needs before adding primitives. Relinking retains IDs/photo edits; source changes/removal never resample stored calibration RGB.
2. Add separate recovery/autosave with explicit saved-document identity and stale/recovery handling. Preserve the last explicit save; test interrupted/failed writes and corrupt/incompatible records. Do not claim complete power-loss durability without evidence.
3. Add undo/redo for shared/photo edits, reset and successful picks, coalescing one gesture into one operation. Include accepted orientation, framing and flat-field changes in the selected scope. Invalid/cancelled picks create no entry; drafts, view/navigation and derived caches stay transient.
4. Verify exact settings/output reopen, worker-active document transitions, stale-result rejection, recovery decisions and independent undo scopes. Legal out-of-slider values must survive. Use narrow state tests before an editor Save/Open smoke.

Likely boundaries: `src/{editor,editor_document,editor_files,project}.rs`, worker commands, close/dialog handling and focused project/recovery/undo tests. [Architecture](../architecture.md#persistence-boundary-and-known-gaps).

Next: select and design one missing-source/relink, recovery or undo slice.
