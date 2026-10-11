# Separate inversion, photography and interchange spaces

Status: superseded by [native-camera inversion](native-camera-inversion.md). The separation of photographic/interchange/rendering roles is retained; the AP0 inversion choice below is historical rationale, not the current contract.

## Context

The nonlinear inversion basis, photographic controls and file interchange serve different purposes. Selecting one must not implicitly choose the others. Camera-native channels alone are not a standard colour-space definition; AP0 channels are not physical film-dye measurements.

## Decision

Develop/convert into AP0/D60 for calibration, inversion and roll-neutral fitting. Convert the signed inversion result into AP1/D60 for photographic exposure/Bradford WB. Convert processed AP1 back to AP0/D60 for scene-linear EXR. Rendering targets/encoding remain separate downstream choices.

## Consequences

Preserve signed and above-one working values; no intermediate sRGB/working-space clamp. AP0/AP1 share D60, so this boundary requires no adaptation. The rendering transform's input limit may blacken all-nonpositive AP1 vectors, but it must not destroy working/EXR values. Selectable inversion bases remain separately designed/versioned work, not permission to reinterpret existing calibration units.

## Evidence

- [Processing contract](../contracts/processing-model.md#color-space-roles), [output contract](../contracts/output.md).
- [Dmin/black boundary trace](../research/dmin-shadow-trace.md#where-variation-is-lost).
- `src/color.rs`, `src/processing.rs`, `tests/processing.rs`; [historical working-space update](../research/history/execution-plan.md#progress-evidence).
