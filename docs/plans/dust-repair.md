# Dust-repair output and remaining acceptance

The provisional manual workflow, neutral-AP1 working-view integration and project persistence are implemented. [Projects](../projects.md#source-owned-dust-repairs) owns saved instructions and compatibility; [processing](../processing.md#ungraded-positive-and-repair-placement) owns the stage; [editor](../editor.md#provisional-dust-cleanup) owns gestures, cache lifetime and user-visible limits. The [research guide](../research/dust-removal.md) records evidence for the `ap1-boundary-ranked-score10-grow4-v1` solver, not a production-quality guarantee. [PLAN.md](../../PLAN.md) owns priority.

## Preserve

- Corrected camera samples are inverted and converted to signed/HDR ungraded AP1 before healing. Photographic Roll/Photo exposure/WB and all creative/local grades remain downstream. Fixed development WB, gamma balance and inversion response remain upstream.
- Every independent repair reads immutable unrepaired input. Manual masks bypass detector thresholds, separate supports cannot overlap, and known f32 values are preserved without clamping signed/HDR data.
- Full-resolution evaluation stays asynchronous and bounded to four million pixels. Source/interpretation/repair identities govern acceptance, not viewport generations; cancellation is not an unchanged healing failure. Downstream range rejection retains valid neutral evaluations for readmission under a safe grade, rather than memoizing a solver failure.
- Retain physical-view detail/underlay, shared viewer/scope output, explicit original reference, source leases, submission gate and numerical/preflight checks. Ordinary grading reuses neutral repairs and working detail; original reconstruction is explicitly requested.
- Apply absolute repairs to original neutral samples before footprint filtering; do not subtract corrections from an already rounded proxy. Cache sparse immutable repair content and view-sized derived copies, not a full-resolution repaired photograph. Source/correction/inversion changes may take time to reevaluate; ordinary grading and navigation reuse valid heals.
- Keep direct canvas selection, release-to-commit creation/edits, Shift-add, Option-remove, donor dragging, Delete and contextual reset/Done. No per-repair Apply/Cancel mode, list, numeric donor panel or local-only undo.
- Preserve the [saved-instruction contract](../projects.md#source-owned-dust-repairs): output work must not replace editable intent with cached pixels or pin automatic donors.

## Next slice: repaired output

**TIFF/EXR and thumbnails remain unrepaired.** Keep that omission visible until the corresponding consumers are implemented. Repaired output is separate from thumbnail work and undo.

The authoritative original-sample export path needs an immutable repair snapshot and explicit invalidation/acceptance tied to its source and interpretation. Apply repairs before original-sample photographic grading and geometry reconstruction, never by exporting a working-view bitmap. Preserve scalar/OCIO diagnostic references, RGB16 source storage, AP0 EXR and destination safety. Settle pending/no-donor export behavior and cancellation before implementation; do not silently omit an enabled saved repair or label unrepaired output repaired.

## Remaining acceptance

1. Exercise pre-grade placement on representative photographs, including corrected scans, signed/HDR regions, strong Roll/Photo grades, fine texture and seams. Compare working detail, coarse navigation coverage and original reference at matching positions. The full-grade AP1 studies do not establish every downstream grade's appearance after pre-grade repair.
2. Broaden normal-use residuals, donor failures, support rejections and complete click/stroke/donor-to-display latency beyond the initial accepted native exercise. Do not retune the retained corpus or change solver constants without a concrete new failure. Inversion changes are uncommon and may reevaluate asynchronously; failure feedback must remain visible.
3. Measure concurrent CPU/GPU ownership during high-volume cleanup, navigation, target changes and source retirement. Sparse GPU records retain coordinates and repaired values; neutral bases, repaired physical rasters, bounded original/region reconstruction and encoded/in-flight older versions add mandatory work. Declared bytes, Arc tests and bounded project fixtures are not aggregate memory or device acceptance.
4. Broaden physical pointer/trackpad, normal/narrow inspector, fullscreen suspension/restoration and modifier/cursor acceptance. Headless input does not establish native interaction or monitor correctness.
5. Preserve regression coverage for source/grade/repair invalidation, cancellation versus genuine failure, reset/delete, crop/turn/flip/straightening, shader initialization only after submission and viewer/scope identity.

## Deliberately deferred

- **Undo:** settle gesture coalescing and shared/Photo scope with the existing undo plan, rather than introducing a repair-only stack.
- **Thumbnails:** separately scope authoritative repair snapshots, reconstruction and invalidation; do not promote a working-view image into the filmstrip cache.
- **Automatic proposals:** present editable masks independently from healing; detector scores/provenance must not change reconstruction. Precision/recall needs independently sampled exhaustive labels.
- **Algorithm questions:** clean-truth accuracy, real dust crossing fine in-focus wires, diagnostic calibration, grain fidelity and broader cameras/materials remain unestablished. No further Lightroom comparison or neural downloads.

## Checks and next action

Use the [normal development checks](../development.md#dust-persistence), including production and reference-feature builds. Run composed editor/UI suites after shared-control or lifecycle changes. The [experiment checks](../../scripts/dust-experiments/README.md#inspect-and-check) reproduce standalone numerical behavior without publishing private images.

Next: scope repaired output when requested. Collect additional photographic/resource evidence only for a concrete symptom.
