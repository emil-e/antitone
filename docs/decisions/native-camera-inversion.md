# Invert in native-camera RGB, then map positives to AP1

Status: accepted by the user; outright replacement, no old-grade preservation/migration required. Supersedes the inversion choice in [colour-space boundaries](colour-space-boundaries.md), and the grade-compatibility/version clauses of [adjustable response](versioned-film-response.md) and [compact source](rgb16-gpu-source.md). Their earlier rationale is retained.

## Context

Colour matrices and channel-wise nonlinear negative inversion do not commute. AP0 channels are colour-mixed coordinates, not physical film-dye measurements. Train-red comparisons established basis-dependent brightness before ACES, not that the brighter rendering is more accurate. Native developed camera channels are the most direct available measurements of the negative, although Bayer spectral overlap and film/scene information loss remain.

A generic baseline is preferable to requiring stock-by-stock user calibration. Shared printing compatibility motivates that direction but does not identify independent dye spectra or prove a universal reconstruction. Roll-density PCA was descriptive, not an inversion transform; the user stopped that research angle.

## Decision

Calibrate, floor, fit channel gamma and invert developed native-camera RGB. Apply optional flat-field gains there first. Map inverted positives through the metadata-derived camera→AP1 matrix, then keep AP1 photographic exposure/Bradford WB and unchanged rendering/output processing. AP0 is retained for EXR interchange and internally in ACES only.

Roll neutrality must correspond to AP1 white, not equal native channels: target the positive camera direction `inverse(camera_to_AP1) * [1,1,1]` while evaluating the actual response. Keep existing response mathematics and new-grade toe 2.5/shoulder off. Keep one retained RGB16 source and accurate shader temporaries; no alternate AP0 inversion path.

Replace processing/project identities with model/schema 4 and a camera-inversion development contract. Old AP0 projects reject explicitly; missing response rejects. Retain camera model/matrix identity with the roll and reject different native-camera source/reference bases rather than share incompatible anchors. Persisted base floors must equal one normalized code; corrected bounds remain derived, not separately authored values. The user does not need personal-grade preservation, so migration machinery would introduce misleading calibration interpretation without a present need. No commits/pushes are implied by approval.

## Consequences and limits

Native coordinates avoid colour mixing before nonlinear inversion; they do not make the response physically calibrated or undo spectral overlap. Post-inversion use of the positive camera matrix is pragmatic and replaceable only with evidence/new approval. No brightness maximization, PCA primaries, hidden pedestal, demosaicer change, per-stock repair or approximate ACES.

Flat-field's physical neutral/centre-luminance target is unchanged, expressed directly through camera→AP1→XYZ. Its inversion precision policy becomes one native code amplified by maximum gain, not an AP0 row-sum bound. Output formats/targets, signed/HDR working data and numerical tolerances remain unchanged.

## Evidence and implementation

- [Processing contract](../contracts/processing-model.md), [project identity](../contracts/project-state.md), [unchanged output](../contracts/output.md).
- `src/{raw,scan,processing,sampling,gpu}.rs` and owned shaders.
- `tests/camera_inversion.rs`: noncommuting order, AP1-neutral camera direction, actual-response fitting and camera-channel floor flags.
- [Implementation closure/checks/limits](../plans/completed/camera-inversion.md). Finite numerical checks are not stock colour calibration or photographic/physical-monitor acceptance.
