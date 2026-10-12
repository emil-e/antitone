# sRGB + Display P3 SDR output

Completed output-update record. Current policy: [output contract](../contracts/output.md), [accepted decision](../decisions/sdr-output-and-presentation.md). Test counts below belong to this historical checkpoint.

Status: implemented after the user explicitly approved trying P3-D65 instead of the failed SDR Rec.2020 candidate. The user also reported that yellow collapse matched what they saw. That supports the diagnosed mechanism but is not a reconstructed/visually accepted current grade.

## Completed scope

1. [x] Replace ProPhoto with **Display P3** for preview and matching RGB16 TIFF export. Native/Rust target index 0 remains sRGB; index 1 is now P3. The second provisional preview, settled refinement, target selector, default wide target, export path and smoke all use the same target.
2. [x] Keep unmodified OCIO 2.6.0 ACES 2 mathematics. Both targets use 100-nit SDR rendering and D65; P3 primaries are `(0.68, 0.32)`, `(0.265, 0.69)`, `(0.15, 0.06)`. Both TIFF encodings use the standard sRGB transfer; Display P3 is not cinema P3 gamma 2.6. Native definitions drive CPU and generated GPU transforms. Regenerate/pin P3 shaders, 2D tables and an original matching ICC profile; retire unused ProPhoto assets.
3. [x] Version project compatibility: schema **2**, output contract **`ocio-2.6.0-aces2-100nit-antitone-srgb-displayp3-v2`**, serialized TIFF target `display_p3`. The subsequent [RGB-only reference cleanup](history/feedback-plan.md#2-simplify-references-and-build-consistent-editing-controls) advances project schema to **3**, without changing the output contract. Old schemas, `pro_photo`, and old output contracts are rejected, not silently reinterpreted. This is the standalone state foundation, **not Save/Open UI integration**. Processing model 2, RAW development and reference units are unchanged.
4. [x] Verify color-preservation regressions, CPU/GPU rendering, managed presentation, ICC/transfer conversion, TIFF/EXR safety and a real editor/full-resolution export smoke. No saturation correction, narrower rendering hidden in a wider file, ACES patch or CPU-viewer fallback.

Scene-linear AP0 EXR, half/full precision, photographic AP1 processing, calibration and source sampling are unchanged. The active presentation reference no longer depends on or emits the retired system-ROMM diagnostic; its signed extended-sRGB carrier check passes. The presentation layer remains encoded extended-sRGB RGBA16Float, explicitly tagged, SDR/EDR=false; conversion from P3 preserves out-of-sRGB coordinates before OS monitor conversion. P3 rendering does not require the whole monitor to be P3, but its visible result still depends on the monitor and viewing conditions. The 100-nit ACES reference is not a claim of measured physical monitor luminance or exact matching to Apple's Photography preset.

## Verification

- **56 all-feature tests**, strict all-target Clippy and formatting pass. Added independent target-color properties rather than only CPU/GPU agreement: 990 nonneutral AP1 grid samples per target remain colored; 21,828 yellow/tint/exposure samples per target retain yellow and have bounded neighboring exposure changes. A Metal test also checks 963 yellow-ramp pixels per target for CPU agreement, hue preservation and continuity. A P3 regression explicitly confirms rendered colors outside sRGB are retained.
- Complete generated-shader comparison: **4,816 pixels per target**, including 3,072 camera-derived samples. Maximum CPU/GPU rendered error `3.75509262e-6` sRGB / `2.86102295e-6` P3; camera working-space error remains `1.33514404e-5`.
- Pinned ICC and regenerated definitions pass exhaustive 65,536-code/channel tests, colorants and XYZ conversion. Independent ColorSync/Core Image maximum error over 200 combinations: `5.9604645e-8` sRGB / `6.2599778e-5` P3.
- Real editor smoke exercises both targets, GPU edits/source bypass, P3 fit/native-pixel refinement and reference switching/return. **14.18 s / 522 submitted frames** on M5 Pro. P3 fit `8640×5760 → 1598×1066`: **2.375 s**; native region `1598×1320 → 1598×1320`: **0.161 s**.
- Full developed `DSC04416` diagnostic export, **8704×6144**, writes P3/sRGB RGB16 TIFF and AP0 half EXR. ImageIO independently reopens both TIFFs and returns the exact pinned ICC bytes. The latest three-output run took **57.68 s** on this machine; this is not a controlled performance-regression study. The sRGB TIFF is byte-identical to the prior full-source export. EXR magic/version, unordered header attributes, header length and complete offset-table/compressed-pixel remainder match the prior export; only pre-existing unordered header serialization order differs.

Tests use explicit diagnostic calibration/settings, not the user's current grade. Physical monitor matching, actual-image visual acceptance and Lightroom/Resolve interoperability remain unverified. No Lightroom/Resolve automation or source-sample changes.

## Historical reproduction and evidence

The following commands record the original checkpoint and include the removed Cargo wrapper. Use [development checks](../development.md) and [specialist verification](../build-and-probes.md) for current setup and supported commands; the snapshot is not a fresh model-4 compatibility result.

```sh
./scripts/probe-gamut.sh
./scripts/probe-ocio.sh
./scripts/probe-rendering.sh
CC=/usr/bin/clang CXX=/usr/bin/clang++ ./scripts/cargo.sh test --offline --all-features
CC=/usr/bin/clang CXX=/usr/bin/clang++ ./scripts/cargo.sh clippy --offline --all-features --all-targets -- -D warnings
```

`./scripts/probe-gamut.sh --compare-rec2020` additionally evaluates the rejected SDR Rec.2020 parameterization and the 500-nit control; it still exits 1 on the known SDR Rec.2020 failures. The default command checks only the active sRGB/P3 pair and passes. The self-contained rendering probe `-- --gamut-check` now checks the active pair and passes; it no longer reproduces the retired ProPhoto target.

Ignored evidence: `.cache/probes/p3-{focused-tests,all-tests,clippy,ocio-gpu,profile-reference,gamut-gate,yellow-check}.txt`, `p3-all-tests-final.txt`, `p3-clippy-final.txt`, `p3-presentation-reference.txt`, `p3-exr-unchanged.txt`, `editor-p3-smoke.txt`, `export-p3-1.txt`, `tiff-imageio-p3.txt`, and `export-p3-1/`. Pinned payload definitions/checksums are in `../../assets/ocio/README.md` and `../../assets/profiles/README.md`. Historical ProPhoto/Rec.2020 diagnosis remains recorded separately; no correction was deployed to either algorithm.

## Remaining feedback work

At this checkpoint black clipping and control cleanup remained open. Later work is recorded in the [Dmin trace](dmin-shadow-trace.md), [film-response integration](film-response-precision-gate.md) and [control contract](../contracts/editor-controls.md). The [current roadmap](../../PLAN.md) distinguishes subsequent implemented features from remaining photographic, persistence and release work. This output-target change alone did not resolve all tonal feedback.
