# Responsive vectorscope and waveform — completed

Status: implemented and integrated with native-camera processing and accepted composition. Fresh photographic/physical acceptance and full-source analysis remain separate [follow-up](../completion.md#scopes).

## Scope and result

Positive-grade feedback uses selected-output-native encoded sRGB/P3 from the existing bounded whole-source proxy, sampled through accepted composition in final-frame order. It does not change calibration, picks, processing, exports or project state.

- One collapsible switchable slot above Roll/Photo controls, width-responsive height/divider, vectorscope default and independent amplification.
- Shared viewer processing/ACES preparation, RGB/luma bin reuse and re-binning of existing rendered samples for vector amplification.
- One reduction/16-byte asynchronous diagnostic readback outstanding. Compatible grading traces remain Updating; incompatible source/revision/composition/target/projection/scale traces cannot paint.
- Source/reference views and crop drafts suspend scopes. Regional settled images never supply scope coverage.
- Stable colour/rim and approximate Skin guides; amplification does not rotate guides or imply full-source statistics.

Canonical formulas, approximation and lifecycle: [scope contract](../../contracts/scopes.md). Rationale: [output-native scope decision](../../decisions/output-native-scopes.md).

## Verification and limits

[Integration evidence](../../research/evidence/scopes-integration.txt) covers both targets, source origins, turns/flips/exact and angled crops, scalar frame-centre bins/counts, native-camera parameters, geometry invalidation/reuse, retained traces and layout/Save behavior. [Initial](../../research/evidence/scopes-checkpoint.txt) and [layout/flicker](../../research/evidence/scopes-layout-and-flicker.txt) evidence preserve earlier timing/onscreen snapshots; they are not fresh integrated photographic acceptance.

Prior guide capture attempts were occluded. Composed-photo/guide appearance, physical controls, full-source statistics, real-photo export reopen, other devices/macOS 14 and physical monitors remain unclaimed. No histogram, automatic correction, full-source analysis, extra RAW/wide retained image, CPU interactive fallback or new colour transform was added.

[Archived implementation record](../../research/history/implementation/vectorscope-and-waveform.txt) preserves the original scope and checkpoints. [Completion work](../completion.md#scopes) owns photographic acceptance and any future full-source design.
