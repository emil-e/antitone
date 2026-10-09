# Pinned output profiles, version 1

Original generated matrix-shaper ICC v4.3 payloads, constructed with Little CMS through `lcms2 6.2.0` / `lcms2-sys 4.0.7` (bundled source reports `LCMS_VERSION=2190`). No Apple, Adobe or third-party profile payload was copied. Embedded copyright tag permits free use.

`src/profiles.rs` contains the generation definition; production uses `include_bytes!` of validated artifacts rather than regenerating timestamp/profile-ID-dependent bytes. Regeneration is intentional versioned work, not a startup action. Native OCIO rendering, probes and ICC generation share target chromaticities in `native/rendering_transform.h`.

- **sRGB/D65:** standard sRGB primaries.
- **Display P3/D65:** primaries `(0.68, 0.32)`, `(0.265, 0.69)`, `(0.15, 0.06)`; white `(0.3127, 0.3290)`.
- **Both transfers:** standard sRGB segmented curve. Decode `E/12.92` for `E ≤ 0.04045`, otherwise `((E + 0.055)/1.055)^2.4`. Encode `12.92L` for `L ≤ 0.0031308`, otherwise `1.055L^(1/2.4) − 0.055`. P3/D65 is not cinema P3's gamma 2.6 or white point.

ICC fixed-point parameters/colorants are quantized; conformance tolerances include that representation precision. Relative-colorimetric PCS uses ICC D50; Little CMS writes the required Bradford adaptation from the specified D65 white. Transfer functions apply after ACES target rendering/limiting, not instead of rendering. The retired ProPhoto/ROMM payload is not used or shipped in the active target set.

SHA-256:

```text
a430f5acd1e9b8b9e99097b6e6091b26d89b606d462978fbe00bd8a56d6c8865  antitone-display-p3-v1.icc
3df81ab2782ccfb1267dcb11a0d0bd1e01b6bd0b10445b3d84a8d8be09eda280  antitone-srgb-v1.icc
```

Tests validate all 65,536 code values per channel, monotonicity, the serialized toe, endpoints, matrix colorants and RGB-to-XYZ/D50 conversion for both pinned and regenerated definitions. `scripts/probe-rendering.sh` independently checks the pinned payloads using macOS ColorSync/Core Image against analytic extended-linear-sRGB references (200 combinations per target; maximum error `5.9604645e-8` sRGB / `6.2599778e-5` P3). ImageIO reopens the full-resolution diagnostic TIFFs with exact ICC bytes. These checks do not establish physical-monitor or Lightroom interoperability correctness.
