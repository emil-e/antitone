# Simple signed-AP1 photographic grading

Status: accepted operator families and blend semantics; initial control defaults/ranges are implementation choices subject to photographic acceptance.

## Choice and rationale

Photo contrast scales ACEScct about a positive pivot, without adding an S-curve. Blend channelwise RGB contrast with a black-offset-ratio proxy sharing the neutral tone response. The proxy deliberately promises black-relative RGB direction, not perceived saturation or proprietary Resolve HDR behavior.

Photo saturation blends constant AP1 XYZ-Y anchoring with HSV-like maximum-channel anchoring. These provide a useful continuum between colour separation and colour separation with darkening. Each blend interpolates independent endpoint outputs in linear AP1 before unchanged ACES. Amount 1 remains exact identity for all blends; both blend defaults are 0: ordinary RGB contrast and constant-Y saturation, with preservation/density opt-in. The contrast control is named Colour preservation, describing intent without guaranteeing unchanged displayed saturation.

The user found both contrast endpoints and constant-Y versus HSV-style saturation useful. Print-density power was less distinct and has not earned a production control; keep its source/evidence in research. More elaborate perceptual/adaptive machinery needs a demonstrated benefit, not a familiar tool label or a physical-film claim.

Keep camera-native calibration/inversion, exposure/WB, signed/HDR AP1, AP0 EXR and pinned ACES unchanged. Calibration and neutral fitting remain pre-creative. Use compensated GPU temporaries and actual-intermediate preflight rather than clamps or silent fallback.

Extend supported schema-5 Photo settings with identity-default fields instead of rejecting current grades or adding migration. Older readers reject the new fields rather than silently discard a look. This does not revive older unsupported schemas. Projects retain exact control values; export metadata identifies the photographic operator contract, not a complete reconstructible recipe.

[Processing](../contracts/processing-model.md#photo-contrastpivot-and-saturation) · [Controls](../contracts/editor-controls.md) · [Persistence](../contracts/project-state.md#photo-grade-extension) · [Implementation/verification](../plans/completed/contrast-and-saturation.md) · [Comparison evidence](../research/contrast-and-saturation.md)
