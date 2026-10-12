# Completion and acceptance work

These are unfinished requirements, not one settled implementation design. [Roadmap](../../PLAN.md) owns priority; select and scope a concrete slice before implementation.

## Geometry

- Exercise native/settled straightening agreement, rotated Gamma/WB picks, invalid/cancelled selections, Save/Open and TIFF/AP0 EXR reopen on real photographs, with and without correction. Numerical agreement does not establish photographic appearance or physical interaction.
- Dense alignment guides need a visual/control design. Thirds are available; any additional guides must not change framing, measurements, processing or export.

[Geometry contract](../contracts/geometry.md). Next: agree a real-photo acceptance exercise or a guide-design slice.

## Scopes

- Inspect rim directions, approximate Skin guide and turned/flipped/cropped/straightened photographs on an unoccluded desktop. Exercise source/reference/crop-draft suspension. Physical controls and photographic appearance remain unverified.
- Full-source analysis is deferred. Decide requested versus automatic operation from actual cost/use need. Use canonical exported coverage, bounded source tiles and valid-edge masks; process/render before accumulation, preserve source-coordinate correction, yield to foreground work and reject stale generations. No extra RAW development or retained full rendered image. Magnification needs another tile pass; proxy data must never be labelled Full source.

[Scope contract](../contracts/scopes.md). Next: agree visual acceptance; no new analysis mode is selected.

## Rendering and thumbnail performance

- Exercise rapid forward/reverse scrolling, live controls and actual OS-pressure transitions; assess aggregate worker/cache/buffer memory on available devices. Headless lifecycle checks do not establish physical responsiveness.
- Measure switching cost/retention before selecting a higher-resolution cache tier. A pending cached preview is not a verified selected Scan: picks, refinement and analysis wait for identity/development acceptance. No whole-roll developed RAW cache.

[Cache contract](../contracts/output.md#responsiveness-and-optional-caches). Next: reproduce a concrete issue or agree a measurement exercise; no new cache tier or capture-identity policy is selected.

## Roll assistance

Start with a cancellable proposed Dmax estimate: bounded candidate discovery, robust full-source confirmation, clipping/floor masks and outlier handling. Show uncertainty/coverage and require explicit acceptance. Insufficient exposure evidence is an outcome, not true Dmax. Gamma/WB suggestions remain model/scene-dependent proposals, not physically known calibration.

## Photographic controls and B&W handling

- Photo contrast/pivot, saturation and related controls need a concrete operator, stage, signed/HDR semantics, identity defaults, scopes and persistence/versioning design. Preserve supported grades, CPU/GPU/pick/export agreement and unchanged ACES.
- B&W needs clarification: monochrome film captured by an RGB camera, colour-negative desaturation, or both. Compare RGB inversion plus a declared mix with a dedicated monochrome response, considering base colour, channel weighting/noise, calibration and tint. No automatic film detection or implicit calibration reinterpretation.

## Operation feedback

Design shared progress for RAW opening/export and development spinners for the viewer/thumbnails. Inventory import, Open/Save, reference processing, refinement, thumbnails and export; declare ownership, phases, completion/errors and stale-result handling through existing worker boundaries.

Use determinate progress only for measurable work and indeterminate feedback for opaque phases. LibRaw and submitted GPU work cannot be interrupted mid-phase. Decide cancellation boundaries separately; preserve transactional document state and safe export replacement. Selected-image work stays ahead of thumbnails. Test success/failure/switching/cancellation; animations stop repainting at completion.

Next: agree the smallest operation-state/presentation slice, starting with RAW loading and export. [Thumbnail follow-up](visual-feedback.md#triaged-thumbnail-follow-up).

## Code readability and style

Agree tracked authored-source scope, language tooling, slice order and checks, excluding generated/vendor files, dependencies, products and samples. Use standard formatters, then inspect naming, dense expressions and long functions in small behavior-preserving slices. Do not redesign architecture as cleanup or install tools globally.

Document authored types/public functions briefly, preserving contracts rather than narrating implementation. Keep inline comments only for non-obvious intent or constraints. Every Rust unsafe block needs its actual preconditions in a `SAFETY:` comment. Preserve numerical, compatibility, ownership and concurrency rationale. Verify focused tests/lints and diff behavior, then record established conventions in `AGENTS.md`; do not claim repository-wide compliance in advance.

## Export and release work

- Batch settings snapshots, reference-only exclusions, destination collisions/per-photo failures and source metadata; shared progress/cancellation follows operation feedback.
- Aggregate resource assessment, broader camera/metadata/domain checks, actual macOS-14 execution and physical monitor matching/transitions. No minimum chip/RAM target is established.
- Bundle native dependencies, including pinned `libomp.dylib`, with bundle-relative loading and required license notices; review distribution layout, sign/notarize and test clean-machine execution. Checkout-local rpaths are not deployable packaging.
- Lightroom/Resolve interoperability remains unverified; no application automation. Windows/HDR display output, custom RAW development and dust/catalog features are outside current scope.
