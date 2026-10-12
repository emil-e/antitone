# Durable editor projects

Status: active recovery/relink/undo package; not a pending basic Save/Open implementation. Scope basis: retained product work. [Roadmap](../../PLAN.md) owns order/status. [Basic Save/Open](completed/save-open.md) and stable-ID editor ownership are implemented. Recovery/relink/undo remain separate slices, not automatically authorized by this package.

## Goal and non-goals

Extend the saveable stable-ID editor with recovery, source-change UI and undo/redo, preserving supported current grades, full-resolution fitting and RGB-only references. No catalog, automatic migration, reference provenance revival, derived images in JSON or silently overwritten explicit saves.

## Existing contracts and gaps

Read [project-state contract](../contracts/project-state.md), [architecture](../architecture.md#persistence-boundary-and-known-gaps) and `src/project.rs`/tests. Current schema/model 4 stores native-camera identity, explicit classic/Film response, accepted composition and optional flat-field state. AP0 schemas/models 2/3 reject without migration; future recovery/undo must not reinterpret them. [Camera implementation closure](completed/camera-inversion.md). `load()` structural validation is not the same as `require_runtime()`.

The editor owns photo edits by stable source ID and passes document WB explicitly to loads. Save/Open restores recorded development policy and accepted settings; dirty transitions are guarded. Recovery, relink/source-change UI and undo are not implemented.

## Proposed incremental sequence and acceptance

1. **Complete:** map authoritative editor roll/photo/output/source state to `Project` with stable IDs/order, and keep transient picker/view/worker state separate. Specify dirty transitions and unsaved-open/close behavior before wiring dialogs.
2. **Complete:** wire explicit Save/Open with runtime and GPU-domain checks before committing a replacement document. Preserve current state after failed/cancelled load/save. Current model-4 settings, accepted geometry/flat-field state and rendered output must remain exact after reopen, including legal out-of-slider values; the original schema-3 checks are historical, not a compatibility requirement.
3. Wire missing/matching/changed-source UI using existing fingerprint/relink/explicit-acceptance primitives. Matching relink retains IDs/photo edits; source changes/removal never invalidate or resample stored calibration RGB.
4. Add separate recovery/autosave with explicit saved-document identity and stale/recovery handling. Never silently overwrite the last explicit save; test interrupted/failed writes and corrupt/incompatible records. Do not claim complete power-loss durability without evidence.
5. Add undo/redo for shared/photo edits, reset and successful picks, coalescing one drag/gesture into one operation. Invalid/cancelled picks create no undo entry; view/navigation and derived resources do not dirty processing state unless deliberately specified.
6. Verify state/output reopen, document transitions while workers are active, stale-result rejection, recovery decisions and independent undo scopes. Retain narrower state tests before a real editor Save/Open smoke.

Likely files: `src/{editor,editor_tests,project}.rs`, worker commands, dialogs/host close behavior, `tests/{project,film_project}.rs`, new focused recovery/undo tests. Include already-implemented shared/photo orientation, accepted framing and roll flat-field changes in any agreed undo/recovery scope; picker/crop drafts and derived caches remain transient. Future scopes do not belong in saved processing state.

## Implementation status

Basic Save/Open is complete: stable-ID ownership, background registration, explicit WB, atomic snapshots, staged transitions and File/Export menus. [Closure and verification limits](completed/save-open.md) link the original state/data-loss and real-source checks. Camera/geometry integration established the current schema and persistence paths; recovery/relink/undo are the remaining implementation scope.

Next action: select/design one missing-source/relink, recovery or undo slice with the user before implementation.
