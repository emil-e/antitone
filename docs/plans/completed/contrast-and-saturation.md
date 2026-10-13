# Photo contrast/pivot and saturation

Status: implementation complete; verified with current main's camera-roll framing, clipping and pan-cursor changes. Photographic/physical acceptance is tracked separately in [completion](../completion.md#photographic-controls-and-bw-handling).

## Delivered scope

- Photo-owned Contrast/Pivot and Colour preservation, followed by Saturation/Density. Contrast range **0.4..1.6**, pivot 0.01..1, saturation 0..2, blends 0..1. Defaults: contrast/saturation 1, pivot 0.18, both blends 0.
- Independently evaluated endpoint outputs blend in signed linear AP1: ACEScct RGB contrast versus black-relative ratio proxy; constant AP1 Y versus HSV-like maximum-anchor saturation. No extra S-curve, gamut repair, print-density-power control or perceptual-saturation promise.
- Scalar and compensated positive-only Metal processing, actual-intermediate range preflight, polished scoped controls/resets, source-owned strict schema-5 persistence and full parameter snapshots for previews/scopes/thumbnails/export.
- Identity defaults preserve supported existing grades; explicit saved blends are retained. Source inspection, calibration and neutral fitting remain pre-creative. Camera-floor flags, compact RGB16 source, native-camera inversion, pinned ACES and signed AP0 EXR boundaries are unchanged.

[Processing](../../contracts/processing-model.md#photo-contrastpivot-and-saturation) · [Controls](../../contracts/editor-controls.md) · [Persistence](../../contracts/project-state.md#photo-grade-extension) · [Decision](../../decisions/photographic-grading.md) · [Retained comparison evidence](../../research/contrast-and-saturation.md) · [Reference tool](../../research/tools/contrast-saturation/README.md).

## Verification snapshot

**2026-10-13**, macOS 26.6.2 arm64 / Rust 1.99.0 / pinned native dependencies. All commands below passed after integration with current main:

```sh
cargo test --locked --offline --no-default-features
cargo test --locked --offline --features editor --lib \
  --test photographic --test project --test camera_project --test film_project \
  --test gpu_processing --test gpu_film_response --test viewer --test scopes \
  --test film_export --test geometry_export --test contrast-saturation-reference \
  --bin antitone-contrast-saturation-probe
cargo clippy --locked --offline --features editor --all-targets -- -D warnings
cargo fmt --all -- --check
cargo build-app --offline
git diff --check
```

Editor library: 216 passed/12 ignored. End-to-end Metal checks cover 648 creative configurations including the widened endpoints, both blends and exposure extremes, at unchanged tolerances. Direct primitive tests cover exact analytic zero-Y gain, signed/HDR planes and toe-centred pivots. A discovered decode-toe cancellation defect is fixed by a continuous piecewise divided-difference integral, not an epsilon/clamp or weaker gate.

Combined regressions cover main's clipped camera-roll GPU callbacks at both contrast endpoints, multiple strip heights and 1×/2× scaling; complete grade restoration across source switches and unregistered-source defaults; save snapshots and neutral-pick isolation. Graded viewer/scopes, corrected/rotated TIFF/AP0 EXR reopening, persistence, individual/section resets and narrow control reachability pass. An initial independent static review found stale status text, corrected before delivery. Documentation links/anchors resolve.

## Remaining acceptance

Ignored RAW/performance/physical checks were not run. Full-resolution noisy-shadow/skin/coloured-light judgments, physical input, monitor/macOS-14/broader-device behavior and older-binary interoperability remain unverified; the historical galleries were not regenerated after the numerical corner fix. See [remaining photographic acceptance](../completion.md#photographic-controls-and-bw-handling) and [the specialist guide](../../build-and-probes.md). Print-density power remains research-only.
