# Dust-repair acceptance and output boundaries

The transient manual workflow and its neutral-AP1 working-view integration are implemented. [Processing](../processing.md#ungraded-positive-and-repair-placement) owns the repair stage; [editor](../editor.md#provisional-dust-cleanup) owns gestures, cache lifetime and user-visible limits. The [research guide](../research/dust-removal.md) records evidence for the unchanged `ap1-boundary-ranked-score10-grow4-v1` solver, not a production-quality guarantee. [PLAN.md](../../PLAN.md) owns priority.

## Preserve

- Corrected camera samples are inverted and converted to signed/HDR ungraded AP1 before healing. Photographic Roll/Photo exposure/WB and all creative/local grades remain downstream. Fixed development WB, gamma balance and inversion response remain upstream.
- Every independent repair reads immutable unrepaired input. Manual masks bypass detector thresholds, separate supports cannot overlap, and known f32 values are preserved without clamping signed/HDR data.
- Full-resolution evaluation stays asynchronous and bounded to four million pixels. Source/interpretation/repair identities govern acceptance, not viewport generations; cancellation is not an unchanged healing failure. Downstream range rejection retains valid neutral evaluations for readmission under a safe grade, rather than memoizing a solver failure.
- Retain main's physical-view detail/underlay, shared viewer/scope output, explicit original reference, source leases, submission gate and numerical/preflight checks. Do not restore an idle sharpen-after-edit pipeline or grade precomputed post-grade patches twice.
- Apply absolute repairs to original neutral samples before footprint filtering; do not subtract corrections from an already rounded proxy. Cache sparse immutable repair content and view-sized derived copies, not a full-resolution repaired photograph. Source/correction/inversion changes may take time to reevaluate; ordinary grading and navigation reuse valid heals.
- Keep direct canvas selection, release-to-commit creation/edits, Shift-add, Option-remove, donor dragging, Delete and contextual reset/Done. No per-repair Apply/Cancel mode, list, numeric donor panel or local-only undo.

## Remaining acceptance

1. Exercise the new pre-grade placement on representative photographs, including corrected scans, signed/HDR regions, strong Roll/Photo grades, fine texture and seams. Compare working detail, coarse navigation coverage and original reference at matching positions. The historical full-grade AP1 studies do not automatically establish every downstream grade's appearance.
2. Collect normal-use residuals, donor failures, support rejections and complete click/stroke/donor-to-display latency. Do not retune the retained corpus or change solver constants without a concrete new failure. Inversion changes are uncommon and may reevaluate asynchronously; updating/failure feedback must remain honest.
3. Measure concurrent CPU/GPU ownership during high-volume cleanup, navigation, target changes and source retirement. Sparse GPU records retain coordinates and repaired values; neutral bases, repaired physical rasters, bounded original/region reconstruction and encoded/in-flight older versions add mandatory work. Declared bytes, Arc tests and a small fixture are not aggregate memory or device acceptance.
4. Broaden physical pointer/trackpad, normal/narrow inspector, fullscreen suspension/restoration and modifier/cursor acceptance. Headless input does not establish native interaction or monitor correctness.
5. Preserve regression coverage for source/grade/repair invalidation, cancellation versus genuine failure, reset/delete, crop/turn/flip/straightening, shader initialization only after submission and viewer/scope identity. Rebaseline against current main rather than carrying historical test exclusions.

## Deliberately deferred

- **Persistence and undo:** repairs remain outside the project schema/dirty state; Save omits them and New/Open/exit discards them. Durable state should express support, donor policy and algorithm/domain identity, not cached repaired pixels. Settle global/local undo together.
- **TIFF/EXR and thumbnails:** these remain unrepaired and the UI explicitly says repairs are unsaved/unexported. GPU export now exists on main, but independent authoritative original-sample consumers still need explicit repair snapshots and invalidation. Never export a working-view bitmap. Retain scalar/OCIO diagnostic references, RGB16 source storage, AP0 EXR and destination safety.
- **Automatic proposals:** present editable masks independently from healing; detector scores/provenance must not change reconstruction. Precision/recall needs independently sampled exhaustive labels.
- **Algorithm questions:** clean-truth accuracy, real dust crossing fine in-focus wires, diagnostic calibration, grain fidelity and broader cameras/materials remain unestablished. No further Lightroom comparison or neural downloads.

## Checks and next action

Use the [normal development checks](../development.md), including both production and reference-feature builds. Focused coverage lives in `dust_repair`, `editor::dust`, `gpu::view::tests::repairs` and `editor::kittest_tests::dust`; run the composed editor/UI suites after changes to shared controls or lifecycle. The [experiment checks](../../scripts/dust-experiments/README.md#inspect-and-check) reproduce standalone numerical behavior without publishing private images.

Next: assess normal cleanup with the integrated pre-grade working view and collect only evidence tied to a concrete latency, rendering or photographic symptom. Scope persistence/output consumers separately after that workflow is accepted. Do not turn remaining acceptance into another corpus-tuning programme.
