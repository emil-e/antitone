# Simple signed-AP1 photographic grading

The operator families and blend semantics are implemented. Defaults/ranges remain subject to photographic acceptance.

## Choice and rationale

Photo contrast scales ACEScct about a positive pivot, without adding an S-curve. Blend channelwise RGB contrast with a black-offset-ratio proxy sharing the neutral tone response. The proxy deliberately promises black-relative RGB direction, not perceived saturation or proprietary Resolve HDR behavior.

Photo saturation blends constant AP1 XYZ-Y anchoring with HSV-like maximum-channel anchoring. These provide a useful continuum between colour separation and colour separation with darkening. Each blend interpolates independent endpoint outputs in linear AP1 before unchanged ACES. Amount 1 remains exact identity for all blends; both blend defaults are 0: ordinary RGB contrast and constant-Y saturation, with preservation/density opt-in. The contrast control is named Colour preservation, describing intent without guaranteeing unchanged displayed saturation.

## Why these families

[ACEScct](https://docs.acescentral.com/encodings/acescct/) provides log-like contrast with an explicit linear toe for signed values. Scaling an encoded log signal is not affine scene-linear RGB scaling. The Resolve 18.5 manual documents [linear Primaries contrast with the S-curve disabled](https://ltbits.github.io/davinci-resolve-manuals/DR18/DR18-RM10-Color.pdf#page=110), but describes [HDR contrast as preserving perceived saturation](https://ltbits.github.io/davinci-resolve-manuals/DR18/DR18-RM10-Color.pdf#page=127) without enough equations to reproduce it. These are version-specific prior art, not evidence of exact proprietary or current-version equivalence.

Direct luminance-ratio scaling is singular at nonzero signed RGB vectors with zero Y. Subtracting the tone curve's transformed black gives a finite divided difference and analytic zero-Y derivative while sharing the RGB endpoint's entire neutral response. The black term is a decomposition of that curve, not an added pedestal; increased contrast can still make black negative. Stable evaluation across the decode toe must preserve continuity rather than conceal cancellation with an epsilon or gain cap.

[Cullen Kelly's HSV saturation technique](https://blog.frame.io/2023/04/03/cinematic-color-saturation-resolve-hsv/) motivates the density-like direction. Maximum anchoring explains saturation-induced darkening on positive RGB without a dye model or division by the maximum channel. Its signed extension is algebraic, not perceptual; constant Y is not constant perceived brightness. Neither pre-DRT invariant guarantees unchanged displayed saturation after nonlinear ACES rendering.

Relative-density power can change hue ratios and amplify near-zero noise when reducing saturation, and lacks a uniform full-desaturation endpoint. It is not a production operator. More elaborate perceptual/adaptive machinery needs a demonstrated benefit, not a familiar tool label or a physical-film claim.

## Implementation boundaries

Keep camera-native calibration/inversion, exposure/WB, signed/HDR AP1, AP0 EXR and pinned ACES unchanged. Calibration and neutral fitting remain pre-creative. Use compensated GPU temporaries and actual-intermediate preflight rather than clamps or silent fallback.

Extend supported schema-5 Photo settings with identity-default fields instead of rejecting current grades or adding migration. Older readers reject the new fields rather than silently discard a look. This does not revive older unsupported schemas. Projects retain exact control values; export metadata identifies the photographic operator contract, not a complete reconstructible recipe.

[Processing](../contracts/processing-model.md#photo-contrastpivot-and-saturation) · [Controls](../contracts/editor-controls.md) · [Persistence](../contracts/project-state.md#photo-grade-extension) · [Checks](../development.md#feature-specific-checks) · [Remaining acceptance](../plans/completion.md#photographic-controls-and-bw-handling)
