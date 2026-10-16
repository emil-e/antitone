# Dust detection and repair experiments

Standalone research tools for the provisional repair candidate, not an accepted production algorithm. The separate [editor prototype](../../docs/editor.md#provisional-dust-cleanup) uses the frozen Rust healer. [Findings and limitations](../../docs/research/dust-removal.md) · [remaining acceptance/output work](../../docs/plans/dust-repair.md).

Run from the repository root, using the normal project Rust/native setup. Samples are read-only; all photographic inputs extracted from them, masks, case manifests, measurements and HTML/PNG outputs belong in ignored `.cache/`. Use fresh output directories: existing directories/files reject rather than overwrite. No maintained photograph or label dataset is bundled.

## Setup

The extraction tool reuses pinned production LibRaw development, model-4 inversion and OCIO rendering. It needs the same native libraries as the editor; [setup and worktree reuse](../../docs/native-development.md). It does not build or launch the editor.

```sh
cargo build --locked --offline --release --features raw,export,project,ocio-reference \
  --bin antitone-dust-source-probe
python3 -m venv .cache/venv
.cache/venv/bin/python -m pip install --cache-dir .cache/pip \
  -r scripts/dust-experiments/requirements.txt
```

Dependency installation needs permission/network access. The pins were exercised with Python 3.9.6 on Apple Silicon. Nothing is installed globally.

## Native crops and survey previews

```sh
SAMPLES=/absolute/path/to/read-only/samples
mkdir -p .cache/dust/crops
# Explicit diagnostic calibration, NOT the user's accepted roll calibration.
# MAX_SIDE=0 extracts every pixel of the specified source-coordinate crop.
target/release/antitone-dust-source-probe extract \
  "$SAMPLES/DSC04397.ARW" 4000 5644 128 32 \
  "$SAMPLES/DSC04378.ARW" 4000 2600 128 128 \
  "$SAMPLES/DSC04393.ARW" .cache/dust/crops/sky 0 4500 200 1024 1024
```

For a scene survey, omit the crop rectangle and use `MAX_SIDE=1400`. Survey pixels use nearest-source sampling and must not feed comparisons. The script rejects `stride != 1`.

Each extraction writes:

- `source.json`: source rectangle, resolution, fixed WB, camera basis, measured diagnostic anchors, processing settings, floor count and provenance.
- `camera.rgb16`: tightly packed little-endian RGB u16 triples. This generated interchange file is not the padded production GPU source cache.
- `working.ap1-f32`: tightly packed little-endian signed/HDR AP1 f32 triples, from the production scalar pipeline, without a working clamp.
- `original.ppm`: production ACES 2 sRGB rendering, transfer-encoded RGB8 for diagnostic viewing; no embedded ICC.

Working-AP1 extraction is bounded to four million output pixels. RAW development remains full resolution; no custom demosaicer, wide production cache, RAW fingerprint or source modification is introduced. Metadata image orientation is recorded but not applied: rectangles/masks remain in original source coordinates.

The comparisons use an explicitly brighter inspection rendering, leaving analysis/repair data unchanged:

```sh
target/release/antitone-dust-source-probe render \
  .cache/dust/crops/sky/working.ap1-f32 1024 1024 \
  .cache/dust/crops/sky/inspection-plus2.ppm 2
.cache/venv/bin/python - <<'PY'
from PIL import Image
Image.open('.cache/dust/crops/sky/inspection-plus2.ppm').save(
    '.cache/dust/crops/sky/inspection-plus2.png')
PY
```

## Whole-frame native candidate discovery

Survey reduction can hide dust entirely. Scan native camera data before selecting repair crops:

```sh
mkdir -p .cache/dust/search/frames .cache/dust/search/proposals
target/release/antitone-dust-source-probe camera \
  "$SAMPLES/DSC04520.ARW" "$SAMPLES/DSC04378.ARW" \
  .cache/dust/search/frames/DSC04520
PYTHONDONTWRITEBYTECODE=1 .cache/venv/bin/python scripts/dust-experiments/search.py \
  .cache/dust/search/frames/DSC04520 .cache/dust/search/proposals/DSC04520
```

The camera command develops the production RGB16 source once, then streams the complete metadata image area to tightly packed RGB16 with a row-sized temporary. `frame.json` records the area offset, fixed reference WB and camera basis. It performs no inversion or grading and creates no full-frame floating working cache. Each 8640×5760 generated camera file is approximately 299 MB; it is disposable research storage, not a production cache policy.

The Python search memory-maps this file and analyzes overlapping native-pixel tiles. Optional `--polarity positive` searches increased log density only; the default `both` preserves the bright/dark search. Multiscale Gaussian anomalies are ranked using contrast and component compactness, with a spatial quota to avoid showing only one textured object. These are unlabelled proposals, not dust classifications. The search excludes a 64-pixel image perimeter and filters component sizes; weak, long, edge-connected and border defects can be missed. This tool does not establish complete frame inspection or detector recall.

Atlas panels show a 192×192 context and enlarged 32×32 detail. They explicitly map the mean negative-log camera density's local 1st/99th percentiles to grayscale, without denoising or changing source/working data. Use them to find candidates, then verify with wider native contexts and production-colour extraction. Do not interpret a contrast-normalized atlas as an accepted photographic rendering.

`candidates.json` records source-coordinate centres in `source_xy`; `x`, `y` and `bbox` remain relative to the exported metadata area. Masks for `repair` are relative to an independently extracted native crop. Include scene-detail counterexamples and manually inspect some windows selected without scores. Proposal-selected labels are not an independent detection test set.

## Detection

```sh
.cache/venv/bin/python scripts/dust-experiments/experiment.py detect \
  --crops .cache/dust/crops --output .cache/dust/detection
```

Detection uses mean negative-log camera RGB, floored at one camera code **only for analysis**. Gaussian residuals, median residuals and bright/dark morphological top-hats share a local highpass-scale estimate. Scores are not calibrated probabilities or Gaussian sigma values. Source images and production inversion are unchanged.

The sweep uses scores 4/6/8/10. Components smaller than three pixels, larger than 2048 pixels, or wider/taller than 256 pixels reject; a 16-pixel crop perimeter is excluded. Maximum-component filtering can make accepted masks nonmonotonic as threshold changes and splits components. These are experimental masks, not product sensitivity semantics.

Outputs include float score maps, binary masks, high-contrast overlays and measurements. All method timings exclude common analysis/noise-map construction, I/O and display rendering.

## Fixed-mask repair

Supply a private JSON list of cases. For example:

```json
[
  {
    "name": "sky-held-out",
    "crop": "sky",
    "kind": "held-out",
    "shapes": [{"kind": "circle", "center": [480, 450], "radius": 14}],
    "zoom": [416, 386, 544, 514]
  }
]
```

Supported shapes are circles, ellipses (`center`, `axes`, optional `angle`) and lines (`start`, `end`, `width`). Coordinates are native pixels relative to the extracted crop. Instead of `shapes`, a case can specify `mask_file`, a binary PNG of matching dimensions. Manual masks and detector PNGs go through the **same repair functions**.

Only use `kind=held-out` for controlled reconstruction of hidden existing source pixels. It supplies reference pixels for error metrics, not proof that the source was physically clean. Use `visually-labelled-dust` for manually inspected real marks, `unverified-blemish` for ambiguous marks, or `detected-mask` for unreviewed detector output. Record the visual label basis and selection method in the case manifest. None has clean reconstruction ground truth; no reconstruction-error claim is made for them.

```sh
.cache/venv/bin/python scripts/dust-experiments/experiment.py repair \
  --crops .cache/dust/crops --cases .cache/dust/cases.json \
  --probe target/release/antitone-dust-source-probe \
  --output .cache/dust/repairs --exposure 2
```

All five methods receive identical masks. Masked input values are replaced before reconstruction so they cannot leak into donor selection, conditioning or filling. Compact masks use a bounded, up-to-384-pixel context window; dispersed masks retain the whole extracted crop. The actual window is recorded in `case.json` and measurements. Donors search up to 128 pixels in each direction, use ten pixels of matching context, and exclude any source patch containing masked pixels.

Methods: harmonic interpolation; float-conditioned OpenCV Telea; float-conditioned OpenCV Navier–Stokes; masked-context donor matching with cloning; the same donor with harmonic boundary-difference healing. No clipping, RGB8 repair, synthetic grain, global denoising, rotation/scale donor search or neural model is used.

Outputs include native rendered PNGs, enlarged detail sheets, masks, donor coordinates and repaired f32 values inside the mask (`*-masked-values.npy`). Full temporary render buffers are discarded. Every result asserts finite output and exact preservation outside its mask. Timings exclude rendering, RAW development and input/output storage; they are not editor/GPU latency measurements.

The probe can reproduce the frozen boundary-ranked path through the provisional Rust port. `MASK_U8` is one byte per source pixel; nonzero bytes are repair support. Both commands write little-endian AP1 f32 to a new path:

```sh
target/release/antitone-dust-source-probe rust-repair \
  INPUT.ap1-f32 WIDTH HEIGHT MASK_U8 NEW_OUTPUT.ap1-f32
target/release/antitone-dust-source-probe rust-evaluate \
  INPUT.ap1-f32 WIDTH HEIGHT MASK_U8 NEW_OUTPUT.ap1-f32
```

`rust-repair` evaluates the full supplied crop for reference reproduction. `rust-evaluate` uses the bounded window and independent evaluated-patch interface intended for workflow prototyping. Append `DX DY` to either command to use an explicit donor translation instead of automatic matching. These commands are diagnostics; they do not connect repair to the editor, project persistence or export.

## Visually labelled detection inspection

A label manifest uses the same case format and masks as repair. Include `kind=scene-detail` cases marking legitimate features that should survive; don't send those controls to `repair`.

```sh
.cache/venv/bin/python scripts/dust-experiments/experiment.py labelled-detect \
  --crops .cache/dust/crops --cases .cache/dust/labels.json \
  --baseline-scores .cache/dust/detection \
  --output .cache/dust/labelled-detection
```

This compares cached detector scores and a positive-only Gaussian residual, testing the hypothesis that reduced negative transmission increases log density. It evaluates scores 6/10 with mask dilation radii 0/2/4 and writes annotation-overlap measurements and a local HTML mask comparison. Polarity is a limited physical prior, not a universal classifier: legitimate bright scene features still respond.

Painted repair supports include margins and are not physically measured defect masks. Coverage is annotation overlap, not measured recall; Gaussian-discovered cases have selection bias. Flagged pixels outside annotations contain both unlabelled dust and scene detail and must not be called measured false positives. A shared numerical threshold is not an equivalent operating point across methods.

## Shape and edge-context veto study

```sh
.cache/venv/bin/python scripts/dust-experiments/context_study.py \
  --crops .cache/dust/crops --cases .cache/dust/labels.json \
  --output .cache/dust/context-study
```

This compares absolute versus positive Gaussian scores, then experimental compactness/aspect filtering and structure-tensor coherence vetoes. The descriptor is computed in a 6..20-pixel annulus around each component. It is background orientation evidence, not scene identity. Coherence rejection can discard real defects near seams while leaving bright junctions flagged; do not adopt it as a product rule. HTML overlays and JSON descriptors retain both successes and collateral rejection. Incomplete/proposal-selected labels still preclude measured precision/recall.

## Donor and processing-stage study

The stage-study manifest extends the normal case format with an optional integer `donor_offset: [dx, dy]`, relative to each component's destination context rectangle. It specifies a fixed translation for comparison; the basic `experiment.py repair` command remains the automatic-donor baseline. Choose fixed sources from masked surrounding context, not by optimizing against held-out truth. Entire donor rectangles must stay inside the crop and avoid every masked pixel. Unmarked defects in a donor are not automatically excluded.

```sh
.cache/venv/bin/python scripts/dust-experiments/stage_study.py \
  --crops .cache/dust/crops --cases .cache/dust/stage-cases.json \
  --probe target/release/antitone-dust-source-probe \
  --output .cache/dust/stage-study --exposure 2
```

The five baseline repairs operate on camera RGB before inversion and on AP1 after the complete scalar pipeline. Fixed-donor clone/heal variants use identical geometric translations in both representations. Automatic source selection runs separately, so its stage comparison also includes matching changes. All paths use the shared repair functions, independent of mask origin.

Camera-only density-heal variants apply the same donor/difference/harmonic solver to negative-log camera RGB, then exponentiate only repaired pixels. This gives multiplicative transmission correction without clipping or a replacement inversion. Nonpositive known transmission rejects; no silent new floor is introduced. Log-camera healing is an experimental representation, not a selected processing contract or physical dye model.

The probe's `develop SOURCE_DIRECTORY CAMERA_F32 MASK_U8 NEW_AP1` command reads the extracted native crop's processing snapshot, accepts normalized little-endian RGB f32 and one byte per mask pixel, and runs the **existing** scalar pipeline only for selected pixels. Original AP1 bytes outside the mask remain untouched: converting RGB16 normalization to ordinary f32 must not perturb them. Unsupported models, survey proxies, mismatched dimensions and more than four million pixels reject. Selected nonfinite input and AP1 f32 overflow reject; output uses create-new.

The study records the unrepaired f32-normalization baseline error, production floor flags, stage/domain, donor rectangles, fixed translations, selected camera/AP1 values and outside-mask bit preservation. Source JSON carries diagnostic calibration/settings provenance. Fixed cloning is a pointwise-processing control: it should agree across stages within normalization/storage rounding. Healing need not commute. An additive camera correction can remain above the standard floor yet create implausible transmission that inversion amplifies; finite output and zero floor flags do not imply a good repair.

Held-out cases provide reference-only error/highpass measurements. Real and ambiguous marks have no clean truth. Timings cover the repair engine only, excluding source development, inversion, rendering and I/O; full crop sizes differ from the compact-window baseline and do not establish interactive performance. Donor diagrams show target context in cyan, source in yellow, mask in magenta. Local `index.html` links the native detail panels; all photographic artifacts stay private.

## Donor-healing refinement study

The original `donor-heal` remains unchanged as the baseline. This study tests nearby source eligibility, structure-aware matching and two kinds of boundary refinement without changing the mask or choosing donors against hidden reference pixels:

```sh
PYTHONDONTWRITEBYTECODE=1 .cache/venv/bin/python scripts/dust-experiments/donor_study.py \
  --crops .cache/dust/crops --cases .cache/dust/stage-cases.json \
  --probe target/release/antitone-dust-source-probe \
  --output .cache/dust/donor-refinements --exposure 2
```

Both AP1 and log-camera domains run by default; `--domains ap1` runs only post-pipeline repair. The same private manifest and optional fixed translations used by the stage study apply. Binary `mask_file` and painted shapes enter the same `experiment.repair` engine.

The comparisons separate the following questions:

- **Local:** can the source provide clean copied pixels and a clean two-pixel correction exterior, even if unused context overlaps a mask? Matching uses pairs known at both ends and requires enough remaining context. Missing pairs are not black samples.
- **Structure:** does adding smoothed RGB-gradient correspondence to colour matching improve geometric alignment? Feature validity excludes the entire smoothing/derivative footprint around unknowns. The metric is not dust identity or confidence.
- **Biharmonic:** does an exact higher-order correction improve the repair with precisely the same structural donor? It requires two pixels of crop exterior; unsupported edge masks reject explicitly.
- **Boundary:** do a few stable biharmonic steps from the harmonic solution improve integration without the tonal blotches an exact solve can introduce? This is a bounded refinement, not a converged biharmonic solution.
- **Ranked:** does emphasizing context near the defect, weighting colour three times more than gradients and choosing the nearest source within 1% of the best score reduce implausibly distant matches? These constants were explored on the same held-out cases and require independent evaluation; the score is not confidence.

Nearby mask components are jointly matched and solved so hidden pixels cannot become correction boundary data. All maintained variants keep the identical binary output support, discard masked input before matching/filling and preserve known f32 bits. No feather, selection-aware boundary policy, clamp, synthetic grain or new inversion is added. Straight outward feather expansion is intentionally absent: it degraded the current reconstruction controls and prevented nearby donors when their enlarged source footprint intersected unknown pixels. An explicitly authored core/transition mask would be a different experiment. Fixed sources must satisfy the required-pixel and paired-context checks; they need not have an entirely unmasked context rectangle as the old baseline does. Unmarked source contamination remains possible.

After freezing a candidate, generate pixel-independent reconstruction controls before further tuning:

```sh
PYTHONDONTWRITEBYTECODE=1 .cache/venv/bin/python scripts/dust-experiments/heldout_cases.py \
  --crop-root .cache/dust/crops --case-prefix crops \
  --crop sky-wire --crop sky-smooth --crop fur --crop mesh \
  --output .cache/dust/heldout-validation.json
PYTHONDONTWRITEBYTECODE=1 .cache/venv/bin/python scripts/dust-experiments/donor_study.py \
  --crops .cache/dust --cases .cache/dust/heldout-validation.json \
  --probe target/release/antitone-dust-source-probe --domains ap1 \
  --output .cache/dust/heldout-validation
```

The generator creates one circle and one line per named native crop from namespaced SHA-256 coordinates and dimensions only; it never reads pixels. Crop order does not affect coordinates. This prevents visual or repair-result selection of targets, but reusing known images is not independent photographic acceptance and random coordinates do not guarantee structured content. Freeze the candidate before generating/running this set and do not tune it against the result.

The report retains rendered comparisons, donor diagrams, selected values and native floor statistics. Held-out cases additionally record smoothed-gradient error and highpass RGB covariance alongside existing pixel/highpass metrics. Those signals contain scene structure as well as grain; they do not establish grain spectra, real-dust truth or Lightroom parity. Treat comparisons used to choose matcher weights/tolerance as exploratory tuning, not an independent accuracy estimate. See the [findings](../../docs/research/dust-removal.md#donor-refinements-useful-local-improvements-not-a-dependable-default) for the observed gains and regressions. No experimental variant is a selected production default.

## Defect-core and repair-support study

Conservative visual annotations can force the healer to replace clean texture. Compare them with score-derived support while keeping the frozen donor engine unchanged:

```sh
PYTHONDONTWRITEBYTECODE=1 .cache/venv/bin/python scripts/dust-experiments/mask_study.py \
  --crops .cache/dust --cases .cache/dust/mask-cases.json \
  --probe target/release/antitone-dust-source-probe \
  --output .cache/dust/support-study --exposure 2
```

For each annotation, positive Gaussian score thresholds 6/8/10 are tested with four-pixel elliptical growth, plus score 10 with two-pixel growth. Threshold components below three pixels reject. Growth is clipped to the annotation, which remains the maximum editable support; bits outside each resulting support remain exact. `support_variants` may add explicitly drawn comparison shapes to a private case, but they must remain inside the annotation.

The derived high-confidence core is diagnostic segmentation input, not a second repair path. Every resulting binary support goes through `donor-heal-ranked`, exactly as a manual mask would. The healer does not receive scores, thresholds or mask provenance. Score-derived support is label-assisted because the annotation isolates the candidate ROI; it is not a complete production detector. Smaller masks can preserve more known texture while leaving low-contrast defect halos, and real marks have no clean reconstruction truth.

## Frozen validation on unused scans

Freeze scan selection, visual annotations and candidate identity before producing repair output. The private freeze record stores the SHA-256 of the case manifest, not any RAW content identity. Then run only the frozen score-10/+4 boundary-ranked candidate:

```sh
PYTHONDONTWRITEBYTECODE=1 .cache/venv/bin/python scripts/dust-experiments/independent_validation.py \
  --crops .cache/dust/independent-validation/crops \
  --cases .cache/dust/independent-validation/cases.json \
  --freeze .cache/dust/independent-validation/freeze.json \
  --probe target/release/antitone-dust-source-probe \
  --output .cache/dust/independent-validation/results --exposure 2
```

The freeze record must name `ap1-boundary-ranked-score10-grow4-v1` and match the case-manifest digest. A visual annotation remains only a maximum support; an absent score-10 component rejects explicitly instead of lowering the threshold or falling back. Accepted repairs record the core, grown support, donor and exact outside-support AP1 preservation. Do not change cases, masks, thresholds, growth or matching after viewing results. Proposal-assisted marks are independent repair cases when taken from previously unused scans, but they are still not exhaustive detector labels or physical dust verification.

## Production TIFF and manual Lightroom comparison

Export a native AP1 crop snapshot through Antitone's production ACES 2 TIFF renderer/encoder:

```sh
target/release/antitone-dust-source-probe tiff \
  .cache/dust/crops/sky .cache/dust/crops/sky/working.ap1-f32 \
  .cache/dust/sky-srgb.tif srgb
```

The command requires model 4, native `stride=1`, matching bounded dimensions and a new output path. It reads the exact response metadata from `source.json`, applies no inspection exposure, and calls the production TIFF API: RGB16, lossless LZW/horizontal prediction, matching sRGB or Display P3 encoding and embedded Antitone ICC. The input is an already-developed f32 AP1 crop snapshot, so this isolates repair/export and is not a substitute for full-photo f64 export acceptance.

Prepare a manual Lightroom Heal package from selected private real-mark cases:

```sh
PYTHONDONTWRITEBYTECODE=1 .cache/venv/bin/python scripts/dust-experiments/lightroom_benchmark.py \
  --crops .cache/dust --cases .cache/dust/mask-cases.json \
  --case siding-fleck --case window-fiber --case seam-speck \
  --probe target/release/antitone-dust-source-probe \
  --output .cache/dust/lightroom-benchmark
```

Each case contains an unhealed Antitone-produced sRGB TIFF under `inputs/`, the frozen AP1/score-10/+4 Antitone result under `antitone-reference/`, a support guide, manifest and correction-effort template. Both TIFFs reopen as RGB16 with ICC/processing metadata and match exactly outside encoded repair support. Import only the unhealed files into Lightroom, change no tone/colour/geometry, use non-generative **Heal**, accept its initial source before adjustment, and export at original dimensions as 16-bit sRGB TIFF without output sharpening. Record default success, source moves, mask edits and elapsed effort. The generated HTML contains exact instructions.

This deliberately does not automate Lightroom. If Lightroom Cloud returns named JPEGs instead of the requested TIFFs, create colour-managed, JPEG-matched labelled and deterministic blind comparisons:

```sh
PYTHONDONTWRITEBYTECODE=1 .cache/venv/bin/python scripts/dust-experiments/lightroom_compare.py \
  --package .cache/dust/lightroom-benchmark \
  --output .cache/dust/lightroom-benchmark/comparison
```

The command requires one `outputs/CASE.jpg` per manifest case. It converts each embedded output profile to Antitone sRGB and round-trips the input and Antitone reference through JPEG with that output's quantization tables and sampling. This controls much of the encoding bias but does not restore 16-bit values or make JPEG encoders identical. Outside-support measurements diagnose global encoding/colour differences only; without clean recaptures they are not repair scores.

The [pilot findings](../../docs/research/dust-removal.md#lightroom-pilot-competitive-sanity-check-only) provide competitive context, not interoperability or production acceptance. Further Lightroom comparison is out of scope.

## Controlled detection support coverage

Select a small held-out subset in a separate private case manifest:

```sh
.cache/venv/bin/python scripts/dust-experiments/experiment.py synthetic-detect \
  --crops .cache/dust/crops --cases .cache/dust/synthetic-cases.json \
  --baseline-scores .cache/dust/detection \
  --output .cache/dust/synthetic-detection
```

This introduces achromatic transmission loss (25%/75%), with a one-pixel Gaussian edge, into camera RGB. It is a toy occlusion model, not a measured dust/scattering/demosaicing model. Support is the blurred shape at alpha >= 0.1. Reported support coverage is **not** real-dust recall: an existing scene edge can already cover that support. Baseline support coverage is reported separately.

Alongside common score thresholds, method-specific original-image score quantiles target 0.1%/0.5% pre-component tails. Actual accepted baseline/background areas are reported; component rejection means these are not identical final-mask budgets. Background contains unlabelled real defects, so it must not be called a measured false-positive rate.

## Inspect and check

```sh
.cache/venv/bin/python scripts/dust-experiments/experiment.py report \
  --detection .cache/dust/detection --repairs .cache/dust/repairs \
  --output .cache/dust/report
# Open .cache/dust/report/index.html locally; images never leave the machine.

PYTHONDONTWRITEBYTECODE=1 .cache/venv/bin/python -m unittest discover \
  -s scripts/dust-experiments -p 'test_*.py' -v
cargo fmt --all -- --check
cargo clippy --locked --offline --features raw,render,export,project \
  --bin antitone-dust-source-probe -- -D warnings
```

Native probe integration tests run when the release binary exists; without it they report an explicit skip. Build the probe before interpreting a full test pass.

Pointwise reconstruction error favours averaging away unpredictable grain. Masked highpass energy is a complementary diagnostic, not a guarantee of grain spectrum, colour correlation, structure or semantic fidelity. Neither replaces native-resolution photographic inspection.
