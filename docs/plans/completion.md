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

[Thumbnail/resource follow-up](visual-feedback.md) owns cold scrolling, camera-preview replacement, live grading and actual OS-pressure/aggregate-memory assessment. Higher-resolution recent previews or additional cache tiers need a demonstrated quality/hit-rate benefit and measured cost; none is selected.

Next: reproduce a concrete issue or agree a measurement exercise. [Current preview/cache model](../editor.md#responsiveness-and-optional-caches).

## Roll assistance

[Analyze acceptance](roll-assistance.md) owns photographic and physical-use evaluation of the implemented [initializer](../roll-analysis.md). Statistical priors are not measured calibration truth.

## Photographic controls and B&W handling

Evaluate [contrast and saturation](../processing.md#photo-contrast-and-saturation) at full resolution on noisy shadows, skin and coloured lighting, corrected and uncorrected, with both targets. Assess colour/hue, separation and noise/texture. Physical use and older-binary interoperability remain unverified.

B&W scope is unresolved: monochrome film captured by an RGB camera, colour-negative desaturation, or both? Compare the relevant models before choosing one. No automatic film detection or calibration reinterpretation is implied.

## Operation feedback

Shared decision prompts and acknowledged flat-field/save/export failure messages are implemented; [the editor guide](../editor.md#decisions-are-not-operation-failures) owns their input and lifetime policy. Native keyboard/pointer, VoiceOver and display-scaling acceptance remain open. Other loading/import/analysis/background messages still use their existing surfaces, without a selected broader migration.

Next: select one remaining operation for feedback/progress design, or agree native acceptance of the implemented messages. Preserve accepted-state transactions, stale-result rejection, foreground priority and destination safety.

Loading/export progress remains a separate design slice. Inventory operation ownership and available phases before choosing determinate progress or cancellation; opaque native/GPU phases cannot be interrupted immediately, and animation must stop on completion. Coordinate with [thumbnail follow-up](visual-feedback.md#triaged-thumbnail-follow-up).

## Code readability and style

Authored-source cleanup remains separate work. Agree language/tooling, file scope and small behavior-preserving slices before editing; exclude generated/vendor files and samples. Preserve numerical, compatibility, ownership and unsafe preconditions. Do not turn formatting or comments into an architectural redesign or claim repository-wide compliance from a local check.

## Export and release work

- Design batch snapshots, reference-only exclusions, collision/per-photo failure handling, metadata and progress/cancellation.
- Assess aggregate resources, broader cameras/domains, actual macOS 14 and physical monitor matching/transitions; no minimum hardware target is established.
- Bundle native libraries and license notices with bundle-relative loading; sign/notarize and exercise clean-machine execution. Checkout-local runtime paths are not distributable packaging.
- Reopen outputs in Lightroom/Resolve without application automation; interoperability remains unverified.

Windows, HDR display output, custom RAW development, dust removal and catalog features remain outside current scope.
