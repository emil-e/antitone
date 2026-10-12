# Basic editor Save/Open — completed

Status: completed and integrated on main. [Roadmap](../../../PLAN.md) owns next priority; [durable projects](../durable-projects.md) holds separately scoped recovery/relink/undo. The implementation/verification below records the original schema-3 checkpoint, not current compatibility: [native-camera replacement](camera-inversion.md) superseded it with schema/model 4, and [geometry integration](flip-crop-rotate.md) added accepted composition. The old uncommitted handoff is obsolete.

## Scope and decisions

Reuse schema 3, existing project validation/runtime contracts and atomic writers. Persist accepted roll/photo/calibration/output/development state through stable source IDs. No migration, recovery/autosave, relink/changed-byte acceptance UI, undo, geometry, thumbnails/scopes or numerical/rendering redesign.

Approved UI: replace all top action buttons with **File** (New, Open, Add RAWs, Save, Save As) and **Export** (TIFF, EXR half/full) within the existing row; keep single-line document/Modified status and add no toolbar row. Cmd-N/O/S and Cmd-Shift-S run after accepted control edits.

Initial availability policy: missing/changed/incompatible **ordered** RAWs reject Open with the path and preserve the current document. Unavailable nonordered records remain stored; calibration remains RGB-only and is never resampled. Open selects the first ordered photo at Fit; empty filmstrips have no selected scan.

`.antitone.json` is the default filename convention, not a substitute for content/schema/runtime validation. The editor accepts other JSON filenames; no new naming-based migration/compatibility rule is introduced.

## Implementation

- `src/editor_document.rs`: authoritative `Project`, path and saved snapshot; stable-ID photo ownership; background header/fingerprint registration; runtime/GPU/source identity checks; validated atomic snapshots and relative Save As rebasing.
- `src/editor.rs`: document-aware explicit WB loads, staged Import/Open/New, worker Save, monotonic request IDs and stale-message suppression. Candidate source/preview preparation succeeds before replacing active state. No worker-global WB or competing path-keyed photo-settings map remains.
- `src/editor_files.rs`: File actions, overwrite confirmation, accepted-state dirty tracking and Save/Discard/Cancel protection. Save failure/cancellation prevents destructive continuation; failed Open preserves old state even after Discard. Document work keeps the old display and blocks unrelated persistent mutations.
- `src/scan.rs`: shared precision-floor calculation for header/developed identity capture; arithmetic unchanged.
- `src/bin/antitone.rs`: document launch and Save/Open smoke entry point; ordinary window title is Antitone.
- Input RAWs cannot be project save destinations; Export cannot replace the currently open project. Save As retains relative locators relative to the new physical directory while preserving identified files. Unvisited imports retain their default photo settings without whole-roll development.
- Picker rectangles/source return, viewport/selection, bypass, worker IDs and derived GPU resources remain transient. Source-only picker navigation does not register reference provenance.

## Verification

Historical original-slice results, not a fresh run during this plan audit:

- **63 focused tests passed:** 49 library tests, 7 project, 3 film-project and 4 viewer tests. This includes 37 editor/control/document/file tests, source/RAW/viewport checks and existing compact-source/large-Fit regressions. No fresh full integration/export suite claim.
- New coverage: stable-ID edits/order/locators; exact settings and legal out-of-slider values; relative Save As and symlink directory bases; failed-write preservation; JSON-named RAW destination protection; unavailable nonordered records/runtime rejection; save snapshot flushing; dirty guards; stale/failed results; New resetting WB; clean view/navigation; actual Cancel/Discard clicks; viewport-close veto; header menus; Cmd-S after pending numeric text; current-project export protection.
- Strict **all-feature/all-target Clippy**, root formatting and release editor build passed.
- Real-source `--smoke-project` uses read-only DSC04397/DSC04378/DSC04416/DSC04393 and temporary JSON documents. It preserves source order/IDs and unvisited defaults; saves a deliberately distinct fixed WB, starts New, imports a different-as-shot-WB source, then restores saved WB. Exact model-3/model-2 settings and a full-resolution working/sRGB/P3 reference pixel reproduce after reopen. Live Fit/native and reopened settled GPU paths run.
- The first unlocked smoke passed and submitted **727 onscreen frames**. After final edge-case additions, the smoke passed all persistence assertions, including changed/missing/**incompatible** ordered-source rejection, but exited with **“editor smoke acquired no visible frame.”** Native logs remained unfocused; `CGSessionCopyCurrentDictionary` reported `CGSSessionScreenIsLocked=1`. The visible-frame gate was retained, not bypassed. The final build therefore has assertion coverage but not a successful final onscreen run under that locked session.
- One independent state/data-loss review identified absolute-only Save As rebasing and pending-input/protection coverage gaps. Relative rebasing and those focused checks were added. The low-priority filename restriction suggestion was not adopted: content/runtime validation is authoritative and `.antitone.json` remains the default convention.

## Limits and handoff

Native file-picker cancellation/overwrite panels have not been externally driven end-to-end; focused egui tests exercise menu/guard/numeric/close dispatch and worker failure preservation. The final unlocked onscreen repeat remains unverified because the session locked. M5 Pro results are observations, not minimum-hardware or physical-monitor acceptance. No RAWs, ACES mathematics, source storage or numerical tolerances changed.

At closure, recovery/relink/undo and fresh feedback were left for separate selection. The user subsequently deferred persistence work and selected navigation/source inspection, then thumbnails; the root roadmap owns current priority. CPU-path/input questions are now recorded in the [interaction plan](../inspection-and-interaction.md#recorded-supportarchitecture-questions), not duplicated in the inbox. A subsequent unlocked real-source Save/Open smoke passed with 722 onscreen frames; the historical locked-session failure above remains an accurate record.
