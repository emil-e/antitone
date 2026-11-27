# Pinned output profiles, version 1

Original generated matrix-shaper ICC payloads (v4.3 SDR, v4.4 HDR), constructed with Little CMS through `lcms2 6.2.0` / `lcms2-sys 4.0.7` (bundled source reports `LCMS_VERSION=2190`). No Apple, Adobe or third-party profile payload was copied. Embedded copyright tag permits free use.

`src/profiles.rs` contains the generation definition; production uses `include_bytes!` of validated artifacts rather than regenerating timestamp/profile-ID-dependent bytes. Regeneration is intentional versioned work, not a startup action. ICC generation uses the target chromaticities in `src/rendering.rs`; the native OCIO reference verifies them independently.

- **sRGB/D65:** standard sRGB primaries.
- **Display P3/D65:** primaries `(0.68, 0.32)`, `(0.265, 0.69)`, `(0.15, 0.06)`; white `(0.3127, 0.3290)`.
- **SDR transfer:** standard sRGB segmented curve. Decode `E/12.92` for `E ≤ 0.04045`, otherwise `((E + 0.055)/1.055)^2.4`. Encode `12.92L` for `L ≤ 0.0031308`, otherwise `1.055L^(1/2.4) − 0.055`. P3/D65 is not cinema P3's gamma 2.6 or white point.

- **Rec.2020/D65:** all HDR peaks share one ICC profile. Its `cicp` tag identifies full-range Rec.2020 RGB with ST 2084 PQ; its sampled TRCs decode to fractions of 10,000 nits.

ICC fixed-point parameters/colorants are quantized; conformance tolerances include that representation precision. Relative-colorimetric PCS uses ICC D50; Little CMS writes the required Bradford adaptation from the specified D65 white. Transfer functions apply after ACES target rendering/limiting, not instead of rendering.

SHA-256:

```text
a430f5acd1e9b8b9e99097b6e6091b26d89b606d462978fbe00bd8a56d6c8865  antitone-display-p3-v1.icc
3df81ab2782ccfb1267dcb11a0d0bd1e01b6bd0b10445b3d84a8d8be09eda280  antitone-srgb-v1.icc
b7c8732124a673576b5304f148d4b869d7acdeca612392dcfe78f5bf9dcb46dc  antitone-rec2020-pq-v1.icc
```

SDR tests validate all 65,536 code values per channel, monotonicity, serialized toe, endpoints, matrix colorants and RGB-to-XYZ/D50 conversion for pinned and regenerated definitions. The HDR TIFF test checks reopened PQ codes and the RGB signal tag. `scripts/probe-rendering.sh` checks pinned payloads through macOS ColorSync/Core Image against analytic references; ImageIO checks full-resolution diagnostic TIFFs and exact ICC bytes. [Commands](../../docs/diagnostics.md#rendering-gamut-and-icc). These checks validate the profile definitions and payload bytes, not what any physical monitor displays.
