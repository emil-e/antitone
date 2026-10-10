# Unchanged ACES with matching SDR outputs and managed presentation

Status: accepted; replaces the original ProPhoto output choice and rejected SDR Rec.2020 candidate.

## Context

ProPhoto/100-nit and Rec.2020/100-nit ACES parameterizations produced yellow-to-gray collapse before profiles or presentation. CPU/GPU consistency alone reproduced the defect rather than establishing colour preservation. Changing ACES equations, hiding a narrower render inside a wider container or switching to CPU-only interaction was not approved.

## Decision

Keep pinned OCIO 2.6.0 ACES 2 mathematics unchanged. Use 100-nit sRGB/D65 and Display P3/D65 with matching TIFF encoding/ICC. Share native definitions between CPU rendering and pinned generated GPU algorithms/tables. Convert the selected target to explicitly tagged extended-sRGB `RGBA16Float` SDR presentation, with OS monitor conversion and EDR disabled.

## Consequences

Output contract v2 rejects obsolete output state rather than silently reinterpreting it. Preserve P3 out-of-sRGB coordinates until managed presentation; encode once, handle UI separately, and do not apply a second DRT/monitor transform. Generated tables are not permission for a baked-LUT/approximate ACES replacement. Broader gamut/HDR policies require separate approval/colour gates. Software checks and user appearance feedback do not establish physical-monitor/broader-device/macOS-14 acceptance or complete CTL conformance.

## Evidence

- [Output contract](../contracts/output.md), [pinned shaders](../../assets/ocio/README.md), [profiles](../../assets/profiles/README.md).
- [Collapse mechanism/reference limits](../research/aces-yellow-collapse.md), [ProPhoto](../research/prophoto-gamut-diagnosis.md), [SDR Rec.2020](../research/rec2020-rendering-gate.md).
- [Approved P3 implementation](../research/p3-output-update.md), [selected numerical evidence](../research/evidence/README.md).
