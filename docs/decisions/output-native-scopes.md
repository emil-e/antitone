# Output-native scopes with independent magnification

Status: accepted in the vectorscope/waveform planning discussion; implemented for the responsive proxy slice.

## Context

The user wants scopes to judge the rendered photograph and guide live controls. Display-referred scopes reveal the actual tone/gamut rendering; scene-linear scopes answer a different processing/diagnosis question. Resolve's reported small-trace annoyance concerned the trace occupying too little of the graticule, not widget size.

## Decision

Measure the selected ACES output in its native encoded sRGB/Display P3 coordinates before monitor conversion. Label the target and derive its luma/chroma coefficients/reference positions. Suspend outside positive-grade viewing. Keep waveform/vectorscope switchable above the controls and prioritize responsive whole-frame proxy updates.

Vectorscope amplification independently narrows the plotted chroma range about neutral, with explicit scale/reset and offscreen accounting. Share processing/sample/lifecycle code only where simpler; do not require a generic coordinate-system abstraction.

## Consequences

A common extended-sRGB basis was considered for stable cross-target coordinates, but would introduce negative/above-one coordinates for valid P3 colours and was not needed for the user's task. Output-native codes instead correspond to TIFF encoding before quantization. Changing target changes both rendering and coordinate meaning; identical native codes are not necessarily identical colours.

Magnification is not grading, image gamut conversion or trace brightness. Re-binning cached rendered samples retains useful zoom detail without rerunning ACES. Proxy coverage follows accepted composition through canonical geometry mappings and remains explicitly approximate. Full-source analysis requires separate approval; fresh combined photographic/onscreen acceptance remains open.

## Evidence

- [Scope contract](../contracts/scopes.md).
- [Completed implementation/integration](../plans/completed/vectorscope-and-waveform.md).
- [Unchanged rendering/presentation decision](sdr-output-and-presentation.md).
