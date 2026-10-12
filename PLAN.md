# Antitone — current roadmap

## Current focus

Native-camera inversion, geometry, responsive scopes and rendering/thumbnail performance are implemented and integrated on main; their implementation plans are closed. The user reports improved responsiveness from the earlier performance release build. Remaining photographic, real-use and release acceptance is separately scoped; no next implementation is selected. [Performance closure/checks](docs/plans/completed/rendering-performance.md), [active follow-up](docs/plans/completion.md).

For developer setup, use [the README](README.md); for implementation and design, use [the documentation map](docs/README.md) and [architecture](docs/architecture.md). This roadmap records current cross-package status/order; details live in contracts, plans and evidence.

## Implemented

The saveable editor develops RAWs with fixed LibRaw settings, offers full-resolution calibration/neutral picks and scoped controls, renders interactive GPU edits plus bounded tiled settled previews, presents managed SDR, and exports matching sRGB/P3 TIFF or signed/HDR AP0 EXR. One compact RGB16 production source path is retained.

- Native-camera calibration/inversion/fitting and post-inversion camera→AP1 use model/schema 4. New grades retain toe 2.5/shoulder off; AP0 schemas/models reject explicitly, without migration. [Camera closure/photographic limits](docs/plans/completed/camera-inversion.md), [processing contract](docs/contracts/processing-model.md).
- Stable-ID Save/Open, fixed development WB, atomic saves and dirty New/Open/Close guards are implemented. Recovery/relink/source-change UI and undo remain unwired. [Project contract](docs/contracts/project-state.md), [Save/Open closure](docs/plans/completed/save-open.md).
- Navigation, source-inspection brightness and bounded/resizable thumbnails are implemented. [Interaction package](docs/plans/inspection-and-interaction.md), [visual feedback](docs/plans/visual-feedback.md).
- Roll-level flat-field correction is implemented and user-accepted: editable rectangular support, native-camera gains, corrected processing/picks/thumbnails/exports and durable map state. [Closure/evidence limits](docs/plans/completed/flat-field-correction.md).
- Responsive positive-grade scopes reuse the bounded whole-source proxy through accepted composition, independently of viewer navigation. [Scope contract](docs/contracts/scopes.md), [implementation/checks](docs/plans/completed/vectorscope-and-waveform.md); full-source analysis is deferred.
- Shared roll/photo turns/flips, source-anchored crop/straightening, actual-coverage picks, bounded rotated reconstruction and composed exports are implemented. Tab-scoped Scan orientation/Composition panels, action flips, stable-source crop UX and thirds guides are user-accepted for now. [Geometry contract](docs/contracts/geometry.md), [implementation closure](docs/plans/completed/flip-crop-rotate.md).
- Shared/prewarmed renderer resources, restricted rectangular averaging, selected proxy reuse, pressure-responsive/persistent caches and bounded parallel development/lazy diagnostics are implemented on main, preserving accepted-composition scopes and original-source correction. [Performance closure/evidence limits](docs/plans/completed/rendering-performance.md).

## Remaining work

Recovery/relink/undo is recorded but is not the user's highest priority.

1. **Rendering/scopes real-use feedback** — shared-resource ownership, source revision/whole-frame scope reuse and independent thumbnail/refinement checks pass. Physical scrolling/resource acceptance, higher-resolution switching caches and import fingerprint optimization remain separately selected follow-ups. [Scope](docs/plans/completion.md#rendering-and-thumbnail-performance).
2. **Camera/geometry photographic acceptance** — native/settled straightening agreement, rotated Gamma/WB picks, Save/Open and TIFF/EXR reopen on real photographs; synthetic combined checks pass. Dense guides remain separate. [Active acceptance/follow-up](docs/plans/completion.md#geometry).
3. **Scope acceptance and inspection/interaction polish** — fresh scope-guide/composed-photo visual checks, physical trackpad exercise and remaining viewer-intrusion/layout complaints remain open. [Scope acceptance](docs/plans/completion.md#scopes). Dark-red/cold-toe appearance is a reproducible investigation, not an assumed processing fix. [Interaction](docs/plans/inspection-and-interaction.md), [visual feedback](docs/plans/visual-feedback.md).
4. **Larger completion work** — explicitly accepted/cancellable roll estimates, photographic controls/B&W design, general operation feedback and separately scoped readability work. [Package](docs/plans/completion.md).
5. **Export/release acceptance** — progress/cancellation/batch snapshots/metadata, observed aggregate-resource needs, packaging/licensing and broader device/monitor/interoperability checks. [Scope](docs/plans/completion.md#export-and-release-work).
6. **Later persistence** — recovery, relink/source-change UI and undo/redo. Never silently overwrite explicit saves or resample reference RGB. [Package](docs/plans/durable-projects.md).

## Limits and blockers

- Camera and geometry photographic feedback remains distinct from finite scalar/Metal checks. Historical AP0/model-2/3 precision results are research evidence, not current compatibility gates.
- Broader device/resource testing, macOS 14 and physical-monitor/monitor-transition checks remain open. No minimum chip/RAM target is established.
- Lightroom/Resolve interoperability remains deferred; no application automation. Exact historical dog Photo/session settings are unknown.
- Native dependencies use checkout-local builds/rpaths; distribution and universal numerical-domain coverage are not established. Windows/HDR and custom RAW development are outside this pass.

Current checks: [development guide](docs/development.md) and [specialist verification](docs/build-and-probes.md). Earlier execution/checkpoint records are [archived](docs/research/history/README.md), not active work queues. Samples remain read-only.
