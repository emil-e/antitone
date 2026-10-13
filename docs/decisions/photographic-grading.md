# Simple grading that supports signed AP1

The working positive can have negative channels and values above one. Photographic operators must be defined there rather than assuming display RGB or masking difficult inputs with a clamp.

## Contrast

ACEScct supplies log-like contrast with a linear toe for signed values. Scaling it around a pivot changes contrast without introducing an extra S-curve.

An ordinary luminance-ratio method divides by Y and becomes singular for nonzero signed colours with zero luminance. Subtracting the curve's transformed black gives a finite black-relative ratio proxy with the same neutral response as RGB contrast. The black term is part of the curve, not an added pedestal.

Colour preservation blends these endpoints. It describes intent, not a guarantee of constant perceived saturation or exact proprietary Resolve behavior. [ACEScct](https://docs.acescentral.com/encodings/acescct/) is the encoding reference; the operator's stable evaluation belongs in [photographic processing](../../src/photographic.rs).

## Saturation

Constant-Y and maximum-channel anchoring offer a simple continuum between colour separation and separation with darkening. Maximum anchoring is motivated by [HSV-style density saturation](https://blog.frame.io/2023/04/03/cinematic-color-saturation-resolve-hsv/), not a physical dye model.

Its signed extension is algebraic, and constant Y is not constant perceived brightness. Nonlinear rendering can alter either endpoint's displayed appearance. Relative-density powers were not chosen because of hue-ratio changes, near-zero noise amplification and an inconsistent full-desaturation endpoint.

These operators remain after WB and before rendering. [Processing](../processing.md#photo-contrast-and-saturation) explains their use; [the independent reference tests](../../tests/photographic.rs) check identity, endpoint and blend behavior.
