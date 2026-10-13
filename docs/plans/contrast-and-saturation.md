# Photo contrast/pivot and saturation

Status: active implementation, based on the accepted UI-polish workspace (`plan/ui-polish`, checkpoint `ee556a0`). [Research/operators](../research/contrast-and-saturation.md) and [reference tool](../research/tools/contrast-saturation/README.md) retain comparison evidence. Production delivery/integration is not yet complete.

## Selected scope

The user selected log-domain contrast without an extra S-curve, both RGB and black-offset-ratio contrast, and constant-luminance versus HSV-like saturation. Both pairs expose a continuous linear-AP1 endpoint-output blend. Print-density power remains research only. Keep the creative maths simple and semantics honest; no perceptual-preservation claim or extra appearance model.

Photo-owned controls follow exposure/WB: Contrast, Pivot and secondary Colour preservation; Saturation and secondary Density. Initial defaults: contrast 1, pivot 0.18, saturation 1, both blends 0, as selected by the user: RGB contrast and constant-Y saturation by default, with preservation/density opt-in. Identity amounts must preserve previous processing exactly. The user approved modestly extending contrast to 0.4..1.6; pivot 0.01..1, saturation 0..2, blends 0..1.

Current upstream supports schema 5/model 4. Preserve existing schema-5 grades with absent new fields as exact identity; older incompatible schemas remain rejected. Strict unknown-field handling must keep old readers from silently ignoring nonidentity grades. UI preferences remain separate from grading.

## Boundaries

```text
native-camera calibration/inversion → camera-to-AP1 → exposure/Bradford WB
  → blended ACEScct contrast → blended AP1 saturation
  → signed/HDR AP1
       ├→ AP0 EXR (no DRT)
       └→ unchanged pinned ACES sRGB/P3 → preview/scopes/thumbnails/TIFF
```

- ACEScct is a transient coordinate, with its signed linear toe and no interchange high-end clip. Contrast can map black below zero; add no pedestal/clamp.
- Ratio proxy preserves black-relative RGB direction, not perceived saturation. Stable divided differences/analytic zero-Y continuation retain finite signed behavior.
- Constant-Y saturation uses the actual AP1 XYZ-Y row. HSV-like maximum anchoring has an explicit algebraic signed extension. Endpoint interpolation precedes ACES/resampling, not displayed-image blending.
- Source bypass/inspection and calibration/roll/Photo WB measurements ignore creative grading. Photo WB fitting must explicitly reset creative state rather than retain it through `..photo`.
- Preserve compact RGB16 source, compensated GPU precision and existing numerical tolerances. Extend actual-intermediate range preflight; do not add clamps, silent fallback or a CPU interactive path.
- One accepted photo snapshot reaches project persistence, GPU signatures, cached selected previews, refinement, scopes, thumbnails and full-resolution exports. Rejected proposals change no state.

## Work order and acceptance

1. **Scalar/state foundation:** production operators, Photo defaults/ranges, strict identity-default serialization, constructor validation and pre-creative neutral-pick boundary. Compare production equations against the retained reference on signed/HDR/zero-Y input.
2. **GPU processing:** positive-only compensated grade stage, uniform layout/constants and complete bounds. Test both endpoints/blends, extremes, source bypass, floor flags and identity against existing CPU/GPU gates. No weakening of tolerances.
3. **Editor/durable integration:** reuse polished parameter rows/sections; scoped resets, disabled/pending text, collapsed summaries, source switching and dirty/Save/Open. Confirm all preview/cache/scope/thumbnail/export paths use the same grade.
4. **Output/contracts:** retain unchanged ACES and signed AP0 EXR, declare creative metadata and update affected processing/project/UI/output/architecture documentation. Exercise composed/corrected processing and neutral picks together.
5. **Verification/handoff:** focused scalar/project/GPU/editor/render/export checks and diff inspection. Independent review is useful before handoff because this crosses numerical, durable and UI boundaries. Physical input, monitor and broader photographic/device acceptance remain separate; do not claim them from headless tests.

## Progress and verification

- Scalar/state, pre-creative rectangular/coverage WB measurements, compensated positive-only GPU processing and Photo controls are implemented. Source-owned identity defaults preserve supported schema-5 grades; unknown/invalid fields remain strict. Shared parameter snapshots carry the grade through previews/scopes/thumbnails/export; output metadata identifies the operator contract.
- Processing/project/control/output/product contracts, architecture and [decision](../decisions/photographic-grading.md) are current. Research/probe sources are retained; private photographs remain outside tracked files. The user selected Colour preservation as the contrast blend label and zero defaults for both blends. These are UX choices, not calibrated photographic facts; saved field names and explicit blend values remain unchanged.
- Independent static review found stale planning/product text, now corrected. Direct Metal zero-Y testing subsequently exposed cancellation across a decode-toe boundary near zero luminance; scalar/reference/Metal now integrate the continuous piecewise curve instead of subtracting near-equal decodes. Regressions exercise near-zero signed planes, exact analytic gain and toe-centred pivots without weakening gates.

Verification snapshot, **2026-10-13**, macOS 26.6.2 arm64 / Rust 1.99.0 / pinned native dependencies:

```sh
cargo test --locked --offline --no-default-features
cargo test --locked --offline --features editor --lib \
  --test photographic --test project --test gpu_processing --test gpu_film_response \
  --test viewer --test scopes --test film_export --test geometry_export
cargo test --locked --offline --features editor \
  --test contrast-saturation-reference --bin antitone-contrast-saturation-probe
cargo clippy --locked --offline --features editor --all-targets -- -D warnings
cargo fmt --all -- --check
cargo build-app --offline
git diff --check
```

All passed. The editor library snapshot is 209 passed/11 ignored. Metal covers 432 new end-to-end grade configurations plus direct signed/HDR/zero-Y/toe-centred primitive cases at unchanged tolerances. Graded viewer/scopes, thumbnail parameter ownership, corrected/rotated TIFF/AP0 EXR reopening and supported-project identity/save/load regressions pass. Local documentation file links resolve. Ignored RAW/physical/performance checks were not run; the historical photo galleries were not regenerated after the numerical corner fix.

The approved Colour preservation/zero-default follow-up passed editor-library, photographic-default/serialization, project and Metal checks, plus dedicated enabled/disabled row-reset tests at both inspector widths, strict Clippy, formatting, diff checks and a rebuilt release editor. Explicit saved halfway values round-trip unchanged.

**Next action:** photographic/physical interaction acceptance of the new controls and initial defaults on representative full-resolution/noisy-shadow/skin/coloured-light photographs. The approved range extension and main integration are in progress; delivery checks must run on the integrated result. Monitor/macOS-14/broader-device and older-binary interoperability acceptance remain separate.
