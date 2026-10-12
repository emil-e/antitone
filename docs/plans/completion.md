# Later completion work

Scope basis: product goals/remaining work, not one approved implementation change set. [Roadmap](../../PLAN.md) owns order. Split a selected item into a focused plan; settle its design/acceptance before editing.

## Geometry

[Flip/crop/rotate implementation](completed/flip-crop-rotate.md) is completed, user-accepted for now and integrated onto main. [Native-camera inversion](completed/camera-inversion.md) is also implemented and closed. Do not resume either package's historical steps.

Remaining work is separate from those implementation closures:

- **Photographic/native-diagnostic acceptance:** agree an unlocked-desktop real-photo exercise of straightening and native/settled agreement, rotated Gamma/WB picks, invalid/cancelled selections, Save/Open and TIFF/AP0 EXR reopen, including corrected and uncorrected sources. Synthetic/Metal/file checks already pass; the photographic exercise is not claimed as performed. Camera-model visual review is not calibrated stock/scene-colour acceptance.
- **Dense alignment guides:** optional visual-only follow-up; thirds are already implemented. Settle the overlay/control design before implementation; guides must not affect framing, measurements, processing or exports.
- Responsive scopes now follow accepted composition; [scope acceptance](#scopes) and [durable projects](durable-projects.md) remain separate, not missing geometry implementation.

Next action when selected: agree the photographic acceptance exercise or the smallest guide-design slice. Broader device/monitor/interoperability checks stay in [release work](#export-and-release-work).

## Scopes

[Responsive waveform/vectorscope implementation](completed/vectorscope-and-waveform.md) is closed and integrated with native-camera processing and accepted composition. [Current contract](../contracts/scopes.md) owns basis, approximation and lifecycle. Do not resume the old geometry-rebase or implementation steps.

- **Fresh visual acceptance:** inspect rim directions, approximate Skin guide and its dimmer stroke on an unoccluded desktop; exercise accepted turned/flipped/cropped/straightened photographs alongside source/reference/crop-draft suspension. Synthetic/Metal/layout integration checks pass, but no new combined real-photo/physical-control acceptance is claimed. Prior guide capture attempts were occluded; no gate bypass.
- **Full-source analysis:** deferred and requires separate approval. Decide requested versus automatic analysis from observed cost/use need. Use canonical exported coverage, bounded source tiles and valid-edge masks; process/render before accumulation, preserve original-source flat-field coordinates, yield to foreground work, cancel/reject stale generations and reconcile counts. No retained full rendered image, new RAW development or CPU interactive fallback. Magnifying full-source bins requires another tile pass to recover detail; never relabel proxy data Full source.
- Broader device/macOS-14/monitor checks remain in release work. Existing performance measurements are machine-specific reduction snapshots, not end-to-end gesture acceptance.

Next action when selected: agree an unoccluded real-photo acceptance exercise. No further scope implementation is selected.

## Rendering and thumbnail performance

[Rendering/cache/native-development implementation](completed/rendering-performance.md) is closed and integrated with accepted-composition scopes and geometry. [The output contract](../contracts/output.md#responsiveness-and-optional-caches) owns current cache/pressure policy; do not resume historical performance experiments as implementation steps.

- **Real-use/resource acceptance:** continue release-build feedback, physical rapid forward/reverse scrolling and control use; exercise actual OS-pressure transitions and assess aggregate worker/cache/buffer memory on available devices. Headless warm-roll, cancellation and Metal/onscreen lifecycle checks do not establish these acceptance gates.
- **Selected-image switching previews:** assessed, not implemented. After hash removal, measured backend readiness is still about 0.66–0.67 s, dominated by foreground RAW development. `Editor::load_source` clears the selected view; it does not reuse warm thumbnail proxies while awaiting `Message::Ready`. The next candidate is a separately owned pending preview, initially from existing 256 RGB16 proxies or a bounded recent higher-resolution tier. Measure click-to-first-new-image presentation, preview hit rate/quality and memory before choosing a tier. Keep pending display distinct from the authoritative selected Scan: no picks/refinement/scopes until full preparation completes. Retain current grading/composition and cache freshness; no whole-roll developed RAW cache or new identity/relink policy.
- **RAW hashing — removed outright.** [Removal scope](completed/remove-raw-hashing.md) and [accepted decision](../decisions/no-raw-content-hashing.md). No metadata identity, relinking or old-project compatibility is introduced. Path/stat keys are cache freshness only; camera/development compatibility remains. [Before/after import/switching measurements](../research/import-fingerprints.md); no hash-overlap optimization is queued.
- Full-source scopes stay in [scope follow-up](#scopes); native bundling, actual macOS-14 execution and broader hardware/monitor checks stay in [release work](#export-and-release-work).

Rendering acceptance next action when selected: reproduce an observed responsiveness issue or agree the smallest measurement/acceptance exercise. No new cache tier or scheduler redesign is approved. The hash-removal implementation/checks are recorded separately in the linked plan; capture-identity/relink design remains deferred.

## Inversion bases and roll assistance

The earlier selectable-inversion-bases proposal is superseded by the [accepted native-camera replacement](../decisions/native-camera-inversion.md). Current grades use one native-camera basis with explicit camera identity; no alternate AP0 path or calibration reinterpretation is planned. Any future alternative requires new approval, a versioned processing/compatibility decision and fresh floor/domain/fitting/CPU-GPU gates.

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

A repository-wide readability pass is requested as a behavior-preserving cleanup. Source scope and language tooling remain to be agreed; current priority is in [the roadmap](../../PLAN.md).

- Agree the tracked source scope/tooling first, excluding generated/vendor files, dependencies, build products, cached research outputs and samples. Inventory standard formatters for the included languages; do not install tools globally or hand-format generated shader assets.
- Run standard tooling, then manually inspect formatting, odd/opaque names, dense expressions, overly long functions and other readability problems. Work in small reviewable slices, production code before tests/tools; avoid generalizing or redesigning architecture as cleanup.
- Add brief descriptive documentation for each authored type and public function, covering purpose/behavior and important contracts rather than narrating implementation. Keep ordinary inline comments out unless clarity needs them; preserve non-obvious numerical, compatibility, ownership and concurrency constraints. Every Rust unsafe block needs a `SAFETY:` comment explaining its actual preconditions, not just the marker.
- Verify formatting, relevant lint/build checks and focused tests for any refactor; preserve numerical gates and original-grade behavior. Review diffs for accidental behavior changes and lost rationale. Choose independent review scope when this work is selected, rather than repeatedly reviewing the whole repository.
- After the pass is complete, encode the established formatting/readability/documentation/unsafe conventions in `AGENTS.md`. Do not declare the cleanup done or add unverified repository-wide compliance claims now.

Next action when selected: propose the source scope, language tooling, slice order and verification before changing code.

## Export and release work

- Shared progress/cancellation follows [operation feedback](#operation-feedback); batch settings snapshots, reference-only exclusions, destination collisions/per-photo failures and source metadata propagation remain export-specific work.
- Aggregate worker/cache/buffer memory and latency on available devices, prioritizing observed problems rather than a speculative minimum chip/RAM target; macOS-14 compatibility, monitor/reference matching and transitions; broader camera/metadata/domain tests.
- Bundle native dependencies, including pinned `libomp.dylib`, with bundle-relative loading; include Apache 2.0 with LLVM exceptions notices, review other dependency licenses/distribution layout, sign/notarize and test clean-machine/macOS-14 execution. Checkout-local rpaths and deployment-target inspection are not deployable packaging or OS acceptance.
- Lightroom/Resolve interoperability remains deferred; no application automation is authorized.

[Flat-field correction](completed/flat-field-correction.md) is completed and user-accepted, not folded into calibration/toe or these later requests. The [completed geometry integration](completed/flip-crop-rotate.md#baseline-and-integration-boundary) preserves its original-source coordinates and main's native-camera model. Windows/HDR output, custom RAW development and dust/catalog features are outside the current pass.

## Remaining scope

This package contains unfinished requirements, deferred guides and post-implementation acceptance, not one settled implementation design. The roadmap determines priority; selected items need focused scope and acceptance. Original requirements are preserved in the [historical feedback amendment](../research/history/feedback-plan.md) and [product plan](../research/history/product-plan.md). New observations remain in the feedback inbox until triaged.
