# Antitone — current roadmap

## Current focus

Documentation/workflow reorganization is complete; [closure record](docs/plans/completed/documentation-structure.md). The user approved stabilizing the existing editor and making it saveable, with an initial commit/push first. The settled-preview buffer-limit fix is complete; [closure/reproduction](docs/plans/completed/preview-stability.md). Adapter-supported limits replace restrictive defaults; strip processing is deferred until observed problems justify it. Basic editor Save/Open is implemented; [closure and verification limits](docs/plans/completed/save-open.md). Recovery/relink/undo remain separate slices. Navigation, source inspection and bounded/resizable thumbnails are implemented. Roll-level flat-field correction is user-accepted and complete; [closure](docs/plans/completed/flat-field-correction.md). Current work in this isolated checkout is the user-approved [native-camera inversion replacement](docs/plans/camera-inversion.md), integrated onto the finalized flat-field commit and verified after rebase. No old-grade compatibility is required.

Start here, then use [the documentation map](docs/README.md), [architecture](docs/architecture.md) and [repository workflow](AGENTS.md). Detailed contracts/decisions/evidence have their own homes; this is the only current cross-package status/order record.

## Implemented

The saveable editor develops Sony RAWs with fixed LibRaw settings, offers full-resolution calibration/neutral picks and scoped controls, renders interactive GPU edits plus tiled settled regional previews, presents managed SDR, and exports matching sRGB/P3 TIFF or signed/HDR AP0 EXR.

Completed feedback includes output-target diagnosis/replacement, RGB-only Dmin/Dmax, reusable precise controls, Dmin-first hard-clipping diagnosis and adjustable film response. The camera replacement uses model 4, with new grades retaining toe 2.5/shoulder off; AP0 grades/schemas reject explicitly. One compact RGB16 production source path is integrated. The user reports successful real-use testing. [Completed response plan/evidence](docs/plans/completed/film-response.md).

The camera-replacement editor owns schema-4 project state through stable source IDs, restores saved development WB, offers transactional Open and atomic Save/Save As, and guards dirty New/Open/Close. File and Export menus replace the header action buttons without another row. Recovery, relink/source-change UI and undo remain unwired. [Project contract](docs/contracts/project-state.md).

Roll-level flat-field correction is implemented and user-accepted: automatic/editable rectangular support, three-channel neutral-gray correction preserving centre luminance, corrected processing/picks/thumbnails/exports and durable map state. [Closure/evidence limits](docs/plans/completed/flat-field-correction.md).

## Remaining work

Recovery/relink/undo is already recorded, but the user does **not** consider it the highest priority now. Navigation polish and source inspection are implemented. Bounded thumbnails are implemented in the user-approved enlarged/resizable strip; cache/worker design and verification limits are recorded in the visual-feedback plan.

- **Camera inversion (implemented/verified; photographic feedback pending)** — native-camera calibration/fitting and scalar/GPU inversion, post-inversion camera→AP1, schema/model/development replacement; no AP0 migration. [Plan and exact next action](docs/plans/camera-inversion.md).
- **Scopes and remaining inspection/interaction polish** — scopes need their own basis/design; physical trackpad exercise and remaining layout complaints remain open; consult the user about viewer-intrusion cases. Dark red/cold toe-shadow appearance remains a separate reproducible investigation, not an assumed processing fix, unless it blocks useful grading. [Interaction package](docs/plans/inspection-and-interaction.md), [visual feedback package](docs/plans/visual-feedback.md).
- **Larger completion work** — roll/photo flips and photo quarter-turns, then crop/arbitrary rotation; explicitly accepted/cancellable roll estimates; contrast/pivot needs design. [Package](docs/plans/completion.md).
- **Export/release acceptance** — progress/cancellation/batch snapshots/metadata, aggregate budgets, packaging/licensing and broader device/monitor/interoperability checks. [Scope](docs/plans/completion.md#export-and-release-work).

- **Later persistence work: recovery, relink and undo/redo** — separately scoped; basic Save/Open is complete. Ordered missing/changed/incompatible RAWs currently reject Open while retaining the current document. Never silently overwrite explicit saves or resample reference RGB. [Broader package](docs/plans/durable-projects.md).

Windows/HDR and custom RAW development are not part of the current pass.

## Limits and blockers

- Broader device/resource testing, macOS 14 and physical-monitor/monitor-transition validation remain open; finite M5 results do not establish universal acceptance. No minimum chip/RAM target is established.
- Historical AP0/model-2/3 numerical limits remain [research evidence](docs/research/film-response-precision-gate.md), not a current compatibility gate. Model-4 checks establish finite numerical agreement, not calibrated photographic accuracy or universal domain coverage.
- Lightroom/Resolve interoperability remains deferred; no application automation. Exact historical dog Photo/session settings are unknown.
- Native dependencies still use checkout-local builds/rpaths; no distributable packaging or universal numerical-domain proof.

## Handoff

Camera work is isolated in `.worktrees/camera-inversion` on `impl/camera-inversion`, integrated onto main's finalized flat-field commit `c0b664a`. Camera implementation is committed locally on this branch; main is untouched and no push is authorized or made. [Verified checks, remaining limits and next action](docs/plans/camera-inversion.md). Prior editor/navigation/thumbnail outcomes remain in their closure plans. Flat-field is closed and accepted, with native/physical limits retained as historical evidence rather than active blockers; [closure](docs/plans/completed/flat-field-correction.md). Do not restart historical investigations/PCA or broaden into scopes/recovery/geometry from stale checklists.

Current commands: [runbook](docs/build-and-probes.md). Original accumulated plans/checkpoints are preserved as [history](docs/research/history/README.md), not active queues. Samples remain read-only; commits, pushes and global installs require explicit user authorization.
