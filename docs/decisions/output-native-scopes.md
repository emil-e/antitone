# Output-native scopes with independent magnification

Status: accepted.

## Choice and rationale

Scopes judge the rendered positive photograph, so measure the selected ACES output's native encoded sRGB/Display P3 coordinates before monitor conversion. Label the target and derive its luma/chroma coefficients and references. These coordinates correspond to TIFF encoding before quantization; identical codes across targets need not represent identical colours.

Use the whole accepted-composition proxy for responsive updates, explicitly labelled Approximate. Regional settled viewer images cannot supply whole-frame statistics. Suspend scopes outside positive-grade viewing.

Vectorscope amplification narrows chroma range around neutral independently of widget size or grade. Re-bin cached rendered samples for detail, with explicit scale/reset and offscreen accounting, without rerunning ACES. Magnification changes neither colour processing nor measured values.

Full-source analysis is a separate feature with different cost and coverage requirements.

[Scope contract](../contracts/scopes.md) · [Rendering/presentation](sdr-output-and-presentation.md)
