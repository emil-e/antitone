# Matching SDR outputs and managed presentation

Status: accepted.

## Choice and rationale

Keep pinned OCIO 2.6.0 ACES 2 mathematics unchanged. Render to 100-nit sRGB/D65 or Display P3/D65, with matching TIFF transfer/ICC. A wider file container must not hide a narrower rendering gamut.

Share native definitions between CPU rendering and pinned generated GPU algorithms/tables. CPU/GPU agreement establishes consistency, not colour preservation; independent gamut, profile and presentation checks are also required.

Convert the selected linear target to explicitly tagged extended-sRGB `RGBA16Float` SDR presentation, with EDR disabled. Preserve P3's out-of-sRGB coordinates until OS monitor conversion. Encode once, handle ordinary UI separately and apply neither a second DRT nor an application monitor transform.

## Limits

Generated parameter tables are not a baked-LUT approximation. Output compatibility rejects unrecognized transform state rather than silently reinterpreting it. HDR or additional gamuts require their own colour-preservation gates. Software comparisons do not establish physical-monitor matching, broader-device/macOS-14 acceptance or complete CTL conformance.

[Output contract](../contracts/output.md) · [Pinned shaders](../../assets/ocio/README.md) · [Profiles](../../assets/profiles/README.md) · [Verification](../build-and-probes.md#rendering-gamut-and-icc)
