# Antitone — current roadmap

## Current focus

Documentation/workflow reorganization is complete; [closure record](docs/plans/completed/documentation-structure.md). The user approved stabilizing the existing editor and making it saveable, with an initial commit/push first. The settled-preview buffer-limit fix is complete; [closure/reproduction](docs/plans/completed/preview-stability.md). Adapter-supported limits replace restrictive defaults; strip processing is deferred until observed problems justify it. Basic editor Save/Open is implemented; [closure and verification limits](docs/plans/completed/save-open.md). Recovery/relink/undo remain separate slices. Current approved sequence: navigation polish and source inspection are implemented; bounded thumbnails are implemented in the approved enlarged/resizable filmstrip; [execution plan](docs/plans/inspection-and-interaction.md#current-execution-plan).

Start here, then use [the documentation map](docs/README.md), [architecture](docs/architecture.md) and [repository workflow](AGENTS.md). Detailed contracts/decisions/evidence have their own homes; this is the only current cross-package status/order record.

## Implemented

The saveable editor develops Sony RAWs with fixed LibRaw settings, offers full-resolution calibration/neutral picks and scoped controls, renders interactive GPU edits plus tiled settled regional previews, presents managed SDR, and exports matching sRGB/P3 TIFF or signed/HDR AP0 EXR.

Completed feedback includes output-target diagnosis/replacement, RGB-only Dmin/Dmax, reusable precise controls, Dmin-first hard-clipping diagnosis and adjustable film response. New grades use model 3 toe 2.5/shoulder off; existing model-2 grades retain their behavior with explicit opt-in. One compact RGB16 production source path is integrated. The user reports successful real-use testing. [Completed response plan/evidence](docs/plans/completed/film-response.md).

The editor owns schema-3 project state through stable source IDs, restores saved development WB, offers transactional Open and atomic Save/Save As, and guards dirty New/Open/Close. File and Export menus replace the header action buttons without another row. Recovery, relink/source-change UI and undo remain unwired. [Project contract](docs/contracts/project-state.md).

## Remaining work

Recovery/relink/undo is already recorded, but the user does **not** consider it the highest priority now. Navigation polish and source inspection are implemented. Bounded thumbnails are implemented in the user-approved enlarged/resizable strip; cache/worker design and verification limits are recorded in the visual-feedback plan.

- **Scopes and remaining inspection/interaction polish** — scopes need their own basis/design; physical trackpad exercise and remaining layout complaints remain open; consult the user about viewer-intrusion cases. Dark red/cold toe-shadow appearance remains a separate reproducible investigation, not an assumed processing fix, unless it blocks useful grading. [Interaction package](docs/plans/inspection-and-interaction.md), [visual feedback package](docs/plans/visual-feedback.md).
- **Larger completion work** — roll/photo flips and photo quarter-turns, then crop/arbitrary rotation; selectable inversion bases and explicitly accepted/cancellable roll estimates; contrast/pivot needs design. [Package](docs/plans/completion.md).
- **Export/release acceptance** — progress/cancellation/batch snapshots/metadata, aggregate budgets, packaging/licensing and broader device/monitor/interoperability checks. [Scope](docs/plans/completion.md#export-and-release-work).

- **Later persistence work: recovery, relink and undo/redo** — separately scoped; basic Save/Open is complete. Ordered missing/changed/incompatible RAWs currently reject Open while retaining the current document. Never silently overwrite explicit saves or resample reference RGB. [Broader package](docs/plans/durable-projects.md).

Flat-field correction remains separate deferred work. Windows/HDR and custom RAW development are not part of the current pass.

## Limits and blockers

- Broader device/resource testing, macOS 14 and physical-monitor/monitor-transition validation remain open; finite M5 results do not establish universal acceptance. No minimum chip/RAM target is established.
- Original model-2 expanded gamma-3/high-EV GPU cancellation limits remain documented. Model-3 candidate cases were resolved without silently rewriting original grades. [Numerical record](docs/research/film-response-precision-gate.md).
- Lightroom/Resolve interoperability remains deferred; no application automation. Exact historical dog Photo/session settings are unknown.
- Native dependencies still use checkout-local builds/rpaths; no distributable packaging or universal numerical-domain proof.

## Handoff

Initial commit/push is complete; subsequent preview, Save/Open, navigation, inspection and thumbnail work remain uncommitted. Save/Open's 63 focused tests, strict all-target Clippy and release build pass; real-source state/WB/output checks pass, with an earlier 727-frame onscreen run and a final screen-locked visible-frame failure explicitly recorded. Navigation/source inspection checks pass: 73 focused tests, strict all-target Clippy, release build and a 722-frame unlocked real-source Save/Open smoke. Bounded thumbnails pass 79 focused tests, strict Clippy and release build; the initial four-thumbnail real-source Save/Open smoke submitted 906 frames with unchanged persistence/output checks. The final repeat passed those assertions but failed the visible-frame gate with the macOS session locked. [Thumbnail verification/limits](docs/plans/visual-feedback.md#progress--handoff). Scopes and remaining investigations need separate agreement; no next implementation is presumed. CPU-path/input-support feedback is recorded in the interaction plan; recovery/relink/undo is tracked for later. Do not restart completed black/toe/source work from stale historical checkboxes.

Current commands: [runbook](docs/build-and-probes.md). Original accumulated plans/checkpoints are preserved as [history](docs/research/history/README.md), not active queues. Samples remain read-only; commits, pushes and global installs require explicit user authorization.
