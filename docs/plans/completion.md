# Later completion work

Scope basis: product goals/remaining work, not one approved implementation change set. [Roadmap](../../PLAN.md) owns order. Split a selected item into a focused plan; settle its design/acceptance before editing.

## Geometry

Start with independent roll/photo horizontal/vertical flips and photo quarter-turns. Define composition, dimensions and one shared source↔display transform across viewer, full-resolution pickers, thumbnails, scopes and exports. Calibration RGB stays unchanged. Use asymmetric corner fixtures for every composition and exact picker mapping.

Then add non-destructive crop/arbitrary rotation with explicit source-coordinate storage, pivot/order, resampling and edge/transparent-area policy. Verify preview/export geometry, valid transformed picks, scope coverage, undo/reopen and untouched RAWs. These are larger processing/state changes, not a viewer-only cosmetic transform.

## Inversion bases and roll assistance

Selectable linear inversion bases are independent of AP1 photography/AP0 EXR. Declare/version reference units/conversion and gamma-fit behavior; recompute floor/domain checks and verify anchors/fitting/CPU-GPU agreement. Reject unusable combinations rather than silently reinterpreting calibration or clipping it.

Start roll assistance with a cancellable proposed Dmax estimate: robust full-source confirmation after bounded candidate discovery, clipping/floor masks and outlier handling. Show uncertainty/coverage and require explicit acceptance; insufficient exposure evidence must be reportable rather than labelled true Dmax. Later gamma/WB suggestions are model/scene-dependent proposals, not physically known automatic calibration.

Contrast/pivot is also requested. Its stage, signed/HDR semantics, defaults and versioning are not decided; it needs a separate processing/UX design.

## Export and release work

- Export progress/cancellation, batch settings snapshots, reference-only exclusions, destination collisions/per-photo failures and source metadata propagation.
- Aggregate worker/cache/buffer memory and latency on M1 Pro/16 GB; macOS-14 compatibility, monitor/reference matching and transitions; broader camera/metadata/domain tests.
- Bundle native dependencies, review dependency licenses/notices and distribution layout. Checkout-local rpaths are not deployable packaging.
- Lightroom/Resolve interoperability remains deferred; no application automation is authorized.

Flat-field correction remains separately deferred, not folded into calibration/toe work. Windows/HDR output, custom RAW development and dust/catalog features are outside the current pass.

## Handoff

No implementation in this package. Next action: choose one work item after the preceding feedback/persistence sequence and design its smallest verifiable slice. Original requirements/approval context: [historical feedback amendment](../research/history/feedback-plan.md), [historical product plan](../research/history/product-plan.md). The feedback inbox is triaged; the remaining requirements above are the persistent record.
