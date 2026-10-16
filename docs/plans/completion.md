# Remaining acceptance and release work

[The roadmap](../../PLAN.md) orders this work. These are open requirements and questions, not selected implementation designs. Focused packages own their detailed scope; this page keeps the other acceptance and release needs visible.

## Geometry

Exercise straightening, rotated neutral picks, rejection/cancellation, Save/Open and TIFF/AP0 EXR reopening on real photographs, corrected and uncorrected. Numerical tests do not establish appearance or physical interaction. Dense alignment guides still need a design; thirds are already available.

Next: agree a photographic acceptance exercise or a guide-design slice. [Current geometry](../geometry.md).

## Scopes

Inspect target directions, the heuristic Skin guide and composed photographs on a visible desktop, including source/reference/draft suspension. Physical interaction and photographic interpretation remain open.

Full-source scopes are deferred, not an implemented mode. Any design must distinguish them from proxy statistics, use actual exported coverage, preserve processing-before-accumulation and source correction, and yield to foreground work without a retained full rendered image.

Next: agree visual acceptance before designing another analysis mode. [Current scopes](../scopes.md).

## Rendering and thumbnail performance

Full-source reuse, persistent CPU/GPU optional byte limits, one native priority worker and eager whole-roll CPU/disk thumbnails are implemented. [Thumbnail/resource follow-up](visual-feedback.md) owns real-use scrolling/grading, normal-budget roll residency and physical pressure/aggregate-memory acceptance. Higher-resolution previews, persistent full-source storage and additional development pools are not selected.

Next: agree bounded essential working-set/resource acceptance. Cache byte limits do not cap process footprint or replace minimum RAM/GPU requirements. No hardware floor is established. [Current preview/cache model](../editor.md#responsiveness-and-optional-caches).

## Roll assistance

The [initializer and sampling-area review](../roll-analysis.md) are accepted for current use. [Analyze follow-up](roll-assistance.md) owns broader corrected/uncorrected roll evaluation, physical review/cancellation and manual Roll/Photo WB, preview retention, project/TIFF/AP0 EXR appearance and deferred dense-tail/sampling work. These are not missing estimator stages. Statistical priors are not measured calibration truth; manual finishing remains expected.

## Photographic controls and B&W handling

Evaluate [contrast and saturation](../processing.md#photo-contrast-and-saturation) at full resolution on noisy shadows, skin and coloured lighting, corrected and uncorrected, with both targets. Assess colour/hue, separation and noise/texture. Physical use and older-binary interoperability remain unverified.

[Local linear/radial adjustments](local-adjustments.md) are implemented. Their photographic, physical-interaction and broader-device acceptance is separate from acceptance of the Global operators.

B&W scope is unresolved: monochrome film captured by an RGB camera, colour-negative desaturation, or both? Compare the relevant models before choosing one. No automatic film detection or calibration reinterpretation is implied.

## Operation feedback

Shared decisions, acknowledged flat-field/save/export failures and usability-based working feedback are implemented; the [editor guide](../editor.md#working-feedback-follows-what-remains-usable) owns the current policy. Native keyboard/pointer, VoiceOver, display scaling and real-operation appearance/latency acceptance remain open. Component/controller checks and composed working-state captures do not establish these.

Loading/import/picker/Analyze failures still use passive status. Broader acknowledged-failure coverage, additional native/export progress and additional cancellation remain unselected; do not infer capabilities from activity animation. Import/Open now report completed-file work, not whole-job percentages.

Next: exercise document work, export inspection and Analyze cancellation in the native editor. Preserve accepted-state transactions, stale-result rejection, foreground priority and destination safety. Coordinate optional background presentation with [thumbnail follow-up](visual-feedback.md#triaged-thumbnail-follow-up).

## Code readability and style

Authored-source cleanup remains separate work. Agree language/tooling, file scope and small behavior-preserving slices before editing; exclude generated/vendor files and samples. Preserve numerical, compatibility, ownership and unsafe preconditions. Do not turn formatting or comments into an architectural redesign or claim repository-wide compliance from a local check.

## Export and release work

- Design batch snapshots, reference-only exclusions, collision/per-photo failure handling, metadata and progress/cancellation.
- Measure essential cold/steady working sets and validate published minimum RAM/GPU requirements across supported cameras/workloads; no numerical hardware floor is established yet. Broader domains, actual macOS 14 and physical monitor matching/transitions remain separate acceptance.
- Bundle native libraries and license notices with bundle-relative loading; sign/notarize and exercise clean-machine execution. Checkout-local runtime paths are not distributable packaging.
- Reopen outputs in Lightroom/Resolve without application automation; interoperability remains unverified.

Windows, HDR display output, custom RAW development, dust removal and catalog features remain outside current scope.
