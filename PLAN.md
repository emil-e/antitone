# Antitone — current roadmap

## Current focus

Documentation/workflow reorganization is complete; [closure record](docs/plans/completed/documentation-structure.md). The user approved stabilizing the existing editor and making it saveable, with an initial commit/push first. No feature implementation is currently in progress; the next bounded task is the settled-preview dimensions/GPU-budget failure.

Start here, then use [the documentation map](docs/README.md), [architecture](docs/architecture.md) and [repository workflow](AGENTS.md). Detailed contracts/decisions/evidence have their own homes; this is the only current cross-package status/order record.

## Implemented

The unsaved editor develops Sony RAWs with fixed LibRaw settings, offers full-resolution calibration/neutral picks and scoped controls, renders interactive GPU edits plus tiled settled regional previews, presents managed SDR, and exports matching sRGB/P3 TIFF or signed/HDR AP0 EXR.

Completed feedback includes output-target diagnosis/replacement, RGB-only Dmin/Dmax, reusable precise controls, Dmin-first hard-clipping diagnosis and adjustable film response. New grades use model 3 toe 2.5/shoulder off; existing model-2 grades retain their behavior with explicit opt-in. One compact RGB16 production source path is integrated. The user reports successful real-use testing. [Completed response plan/evidence](docs/plans/completed/film-response.md).

`src/project.rs` provides tested stable-ID/identity/schema/atomic-save primitives, **not editor Save/Open**. Editor photo state is still path-keyed/in-memory; recovery and undo are unwired. [Project contract](docs/contracts/project-state.md).

## Remaining order

1. **Stabilize settled preview** — diagnose/fix the reported area-average-dimensions/GPU-budget failure and verify fit/zoom/resize behavior as a bounded bug fix. [Package](docs/plans/inspection-and-interaction.md).
2. **Basic editor Save/Open** — integrate stable-ID project state and preserve grades, photo edits and development settings across restart. Then add recovery, relink and undo/redo in separate slices; never silently overwrite explicit saves. This now precedes thumbnails/scopes. [Package](docs/plans/durable-projects.md).
3. **Remaining inspection/interaction polish, then bounded thumbnails/scopes** — dark-source inspection, natural trackpad/click zoom/pan/cursors, `z` Fit and remaining layout complaints; consult the user about viewer-intrusion cases. Dark red/cold toe-shadow appearance remains a separate reproducible investigation, not an assumed processing fix, unless it blocks useful grading. [Interaction package](docs/plans/inspection-and-interaction.md), [visual feedback package](docs/plans/visual-feedback.md).
4. **Larger completion work** — roll/photo flips and photo quarter-turns, then crop/arbitrary rotation; selectable inversion bases and explicitly accepted/cancellable roll estimates; contrast/pivot needs design. [Package](docs/plans/completion.md).
5. **Export/release acceptance** — progress/cancellation/batch snapshots/metadata, aggregate budgets, packaging/licensing and broader device/monitor/interoperability checks. [Scope](docs/plans/completion.md#export-and-release-work).

Flat-field correction remains separate deferred work. Windows/HDR and custom RAW development are not part of the current pass.

## Limits and blockers

- M1 Pro/16 GB, macOS 14 and physical-monitor/monitor-transition validation remain unavailable/open; finite M5 results do not establish that baseline.
- Original model-2 expanded gamma-3/high-EV GPU cancellation limits remain documented. Model-3 candidate cases were resolved without silently rewriting original grades. [Numerical record](docs/research/film-response-precision-gate.md).
- Lightroom/Resolve interoperability remains deferred; no application automation. Exact historical dog Photo/session settings are unknown.
- Native dependencies still use checkout-local builds/rpaths; no distributable packaging or universal numerical-domain proof.

## Handoff

After the requested commit/push, the next approved product action is to diagnose the settled-preview dimensions/GPU-budget failure, then make Save/Open the next feature. Do not restart completed black/toe/source work from stale historical checkboxes.

Current commands: [runbook](docs/build-and-probes.md). Original accumulated plans/checkpoints are preserved as [history](docs/research/history/README.md), not active queues. Samples remain read-only; commits, pushes and global installs require explicit user authorization.
