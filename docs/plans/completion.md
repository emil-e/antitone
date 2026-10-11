# Later completion work

Scope basis: product goals/remaining work, not one approved implementation change set. [Roadmap](../../PLAN.md) owns order. Split a selected item into a focused plan; settle its design/acceptance before editing.

## Geometry

Start with independent roll/photo horizontal/vertical flips and photo quarter-turns. Define composition, dimensions and one shared source↔display transform across viewer, full-resolution pickers, thumbnails, scopes and exports. Calibration RGB stays unchanged. Use asymmetric corner fixtures for every composition and exact picker mapping.

Then add non-destructive crop/arbitrary rotation with explicit source-coordinate storage, pivot/order, resampling and edge/transparent-area policy. Verify preview/export geometry, valid transformed picks, scope coverage, undo/reopen and untouched RAWs. These are larger processing/state changes, not a viewer-only cosmetic transform.

## Inversion bases and roll assistance

Selectable linear inversion bases are independent of AP1 photography/AP0 EXR. Declare/version reference units/conversion and gamma-fit behavior; recompute floor/domain checks and verify anchors/fitting/CPU-GPU agreement. Reject unusable combinations rather than silently reinterpreting calibration or clipping it.

Start roll assistance with a cancellable proposed Dmax estimate: robust full-source confirmation after bounded candidate discovery, clipping/floor masks and outlier handling. Show uncertainty/coverage and require explicit acceptance; insufficient exposure evidence must be reportable rather than labelled true Dmax. Later gamma/WB suggestions are model/scene-dependent proposals, not physically known automatic calibration.

## Photographic controls and B&W handling

- **Common Photo controls:** contrast/pivot, saturation and other common grading controls are requested at least in Photo view; current Photo controls are WB and exposure only. Settle the concrete control set, stage, signed/HDR semantics, identity defaults, scopes and persistence/versioning before implementation. Preserve existing grades, CPU/GPU/pick/export agreement and unchanged ACES; a familiar label does not establish the processing operator.
- **Black-and-white negatives:** discussion required. Clarify whether this means monochrome film captured by an RGB camera, colour-negative desaturation, or both; these need not share the same calibration/inversion path. Compare retaining RGB inversion followed by a declared monochrome mix against a dedicated neutral/monochrome response, including stock/base colour, channel weighting/noise, calibration and output tint. No automatic film detection, calibration reinterpretation or processing change is approved.

## Operation feedback

The user requests a **general progress framework**, most visibly for RAW opening and export, plus spinners while the main view and thumbnails develop. Current workers expose busy/status text; thumbnails use a static ellipsis. This is broader than export-only feedback and needs a focused design before implementation.

- Inventory RAW import/development, project Open/Save, reference processing, preview refinement, thumbnails and exports. Declare operation ownership, phase/completion/error messages, foreground versus background visibility and stale-result handling using existing worker boundaries.
- Use determinate progress only for measurable work; show an indeterminate spinner for opaque phases such as LibRaw development. Do not imply submitted GPU work or LibRaw can be interrupted mid-phase. Separate spinner feedback from any new cancellation API.
- Decide which operations support cancellation and where; cancelled/failed document work must retain transactional state, and exports must retain safe destination replacement. Keep selected-image work ahead of thumbnails.
- Verify success, failure, switching and cancellation transitions; animated indicators must stop requesting repaints when work finishes. Thumbnail-specific reproduction stays in [visual feedback](visual-feedback.md#triaged-thumbnail-follow-up).

Next action when selected: inventory worker events and agree the smallest shared operation-state/presentation slice, starting with RAW loading and export. Batch export remains a separate extension.

## Code readability and style

The user will read the code and requests a repository-wide humanization pass. This is a substantial behavior-preserving cleanup, recorded here rather than executed during feedback triage. Current focus/order remains in [the roadmap](../../PLAN.md).

- Agree the tracked source scope/tooling first, excluding generated/vendor files, dependencies, build products, cached research outputs and samples. Inventory standard formatters for the included languages; do not install tools globally or hand-format generated shader assets.
- Run standard tooling, then manually inspect formatting, odd/opaque names, dense expressions, overly long functions and other readability problems. Work in small reviewable slices, production code before tests/tools; avoid generalizing or redesigning architecture as cleanup.
- Add brief descriptive documentation for each authored type and public function, covering purpose/behavior and important contracts rather than narrating implementation. Keep ordinary inline comments out unless clarity needs them; preserve non-obvious numerical, compatibility, ownership and concurrency constraints. Every Rust unsafe block needs a `SAFETY:` comment explaining its actual preconditions, not just the marker.
- Verify formatting, relevant lint/build checks and focused tests for any refactor; preserve numerical gates and original-grade behavior. Review diffs for accidental behavior changes and lost rationale. Choose independent review scope when this work is selected, rather than repeatedly reviewing the whole repository.
- After the pass is complete, encode the established formatting/readability/documentation/unsafe conventions in `AGENTS.md`. Do not declare the cleanup done or add unverified repository-wide compliance claims now.

Next action when selected: propose the source scope, language tooling, slice order and verification before changing code.

## Export and release work

- Shared progress/cancellation follows [operation feedback](#operation-feedback); batch settings snapshots, reference-only exclusions, destination collisions/per-photo failures and source metadata propagation remain export-specific work.
- Aggregate worker/cache/buffer memory and latency on available devices, prioritizing observed problems rather than a speculative minimum chip/RAM target; macOS-14 compatibility, monitor/reference matching and transitions; broader camera/metadata/domain tests.
- Bundle native dependencies, review dependency licenses/notices and distribution layout. Checkout-local rpaths are not deployable packaging.
- Lightroom/Resolve interoperability remains deferred; no application automation is authorized.

[Flat-field correction](completed/flat-field-correction.md) is completed and user-accepted, not folded into calibration/toe or these later requests. Windows/HDR output, custom RAW development and dust/catalog features are outside the current pass.

## Handoff

No implementation in this package. New feedback has been classified through read-only code/plan inspection, not runtime testing or design approval. Next action: select one work item through the roadmap and design its smallest verifiable slice. Original requirements/approval context: [historical feedback amendment](../research/history/feedback-plan.md), [historical product plan](../research/history/product-plan.md). The feedback inbox is triaged; the remaining requirements above are the persistent record.
