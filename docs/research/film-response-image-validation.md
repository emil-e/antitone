# Corrected Dmin plus film-response toe: offline image check

Historical pre-integration check. The user later approved the signed continuation and versioned production integration; see [accepted decision](../decisions/versioned-film-response.md) and [production gate](film-response-precision-gate.md#production-film-response-checkpoint). Statements below about unaccepted policy or pending integration describe this earlier checkpoint only.

## Result

The combined check supports the proposed direction: **an appropriately sampled Dmin prevents the main hard clipping, and the manufacturer-informed toe opens the remaining positive shadows without changing ACES or adding a black offset**.

With a midtone patch held at the same rendered brightness, the beta-2.5 candidate makes the dog's deepest fur tones easier to distinguish. The sky remains separated and becomes slightly darker, rather than being pushed toward white by a global exposure increase. Shadow foliage also opens in the secondary ring image; the canopy comparison is less dramatic. The gentle shoulder makes relatively small changes and is not needed for the shadow improvement in these examples.

This is an offline photographic check, not proof of calibrated radiometry, universal defaults, exact NLP matching or production readiness. Both independently established dog references give similar results. Flat-field correction has not been implemented or simulated.

## Controlled comparison

The [Dmin-first diagnosis](dmin-shadow-trace.md) established two useful references on `DSC04393.ARW`:

- Whole-picture estimate: `[0.16856635015466970,0.13359162596075455,0.11715123642832656]`, from the coherent `32×32` muzzle-underside patch. This is not proof of unexposed film.
- Measured clear film: `[0.16551166738395376,0.13368683444783128,0.11901288348757422]`, bottom patch `(5000,5660,128,32)`.

The primary pass uses the whole-picture estimate on dog, ring (`DSC04399`) and canopy (`DSC04415`). A sensitivity pass repeats the dog comparison with the measured clear-film value.

Unchanged conditions:

- Established LibRaw development, fixed WB `[2133,1024,1771,1024]` and projected input floor.
- AP0 inversion followed by signed AP1 working values.
- Screenshot-rounded Dmax `[0.008061,0.003409,0.001442]`, global gamma `0.65`, assumed Photo WB/exposure zero.
- Existing ACES 2.0 100-nit Display P3 rendering and matching RGB16 P3 TIFF encoding.
- No pedestal, scan clipping, illumination compensation or ACES-derived shaping.

Three response variants are compared:

| Variant | Toe beta | Shoulder k |
| --- | ---: | ---: |
| Current production inversion | 1 | 0 |
| Manufacturer-informed toe candidate | 2.5 | 0 |
| Same toe plus gentle upper bend | 2.5 | 0.02 |

For beta 1/k 0 the probe calls the **unchanged library inversion directly**, rather than a reimplementation. All models retain Dmin→0 and Dmax→1.

### Balance and exposure are controlled, not held at inappropriate old fits

Each variant is re-balanced through its actual response using the same full-resolution 4,096-pixel visually white ring patch `(3200,3700,64,64)`. This is a shared photographic reference, not a laboratory neutral target. The current model is re-balanced too; the new figures must not be read as Dmin-only changes to the earlier screenshot grade.

The current model at +3 EV supplies each image's brightness target. Candidate exposure is fitted to the same **rendered linear P3 luminance mean** over 16,384 source pixels:

- Dog/ring: `(4700,4200,128,128)`.
- Canopy: `(4000,3000,128,128)`.

Primary matched exposures:

| Source | Current | Toe | Toe + shoulder |
| --- | ---: | ---: | ---: |
| Dog | +3 EV | +2.574 EV | +2.971 EV |
| Ring | +3 EV | +2.794 EV | +3.154 EV |
| Canopy | +3 EV | +2.848 EV | +3.194 EV |

All brightness matches are within one part per million of their respective targets. Matching one patch is not matching the whole histogram or removing the intended tonal changes.

## Dog: what changes after correcting Dmin

Full-resolution `512×384` fur crops contain 196,608 samples each. The table gives rendered grey-luminance code percentiles; these are brightness diagnostics, not claims of grey pixel colour or an 8-bit processing stage.

| Region | Current p05 / median | Toe p05 / median | Pixels with all RGB codes <1/255: current → toe |
| --- | --- | --- | ---: |
| Forehead | 0.0064 / 0.0668 | 0.0377 / 0.0993 | 3,004 → 1 |
| Muzzle | 0.0154 / 0.1094 | 0.0548 / 0.1304 | 2,044 → 25 |
| Chest | 0.00016 / 0.0740 | 0.00835 / 0.1044 | 36,719 → 1,645 |

The sky crop's mean grey-luminance code changes from about `0.712` to `0.660`; its local variation remains. This is not merely extra exposure brightening both the fur and sky. The shoulder variant puts the sky near `0.680` and makes much smaller changes to the fur than the toe itself.

Repeating with the measured clear-film reference yields very similar toe medians: forehead `0.0987`, muzzle `0.1299`, chest `0.1038`. The shadow improvement is not contingent on selecting the interior estimate instead of the clear film.

The exposure sheets separately show each model at matched brightness and ±2 EV, with ±1 EV TIFFs also saved. A brighter final grade reveals more fur in both models, while the candidate gives more room to open the shadows relative to the background. These sheets are exposure diagnostics, not a new default or an exact NLP grade match.

## Near-base caveat: no hidden signed-domain fix

The experimental curve still uses the previously declared **signed-root extension** above measured base. This is a diagnostic continuation for noise/reference/colour-basis inconsistencies, not physical exposure below true film base, and not an accepted production policy.

A small number of previously nonzero pixels become exactly black because this extension changes mixed-sign vector geometry before the existing ACES input clamp:

- Whole-picture reference, dog preview: current 47 black → toe 63 (16 newly black).
- Native chest crop: current 194 black → toe 332 (138 newly black).
- Clear-film reference, chest: current 450 black → toe 515 (65 newly black).

These new black pixels were already extremely faint: the largest previous encoded channel over the dog crops is about `1.29e-5`, below one RGB16 code step (`1/65535 ≈ 1.53e-5`). The secondary ring preview's maximum is also below that step. There is no detected loss of previously visible fur detail from this effect, but it must not be erased from the record or solved with an invented pedestal.

No tested output pixel becomes exactly black while any of its candidate working channels is positive. This CPU check does **not** establish numerical parity for a new GPU inversion; the existing-model GPU checks in the Dmin diagnosis cannot be transferred to the candidate without implementing and testing it.

## Other frames and spatial limitations

The same bottom rectangle was inspected with surrounding negative/holder context on all three frames. It lies in clear film, with the photo above and holder below. Full-resolution AP0 means:

| Frame | Bottom clear-film mean |
| --- | --- |
| Dog | `[0.165512,0.133687,0.119013]` |
| Ring | `[0.165304,0.133181,0.120409]` |
| Canopy | `[0.163630,0.132076,0.120501]` |

These are close but not identical. In particular the dog-derived interior estimate's blue component is below the measured blue on the other frames. The primary pass is a **shared-roll diagnostic**, not fully validated local calibration on every frame. Its near-base/signed results must be interpreted accordingly.

This check does not establish that flat-field correction alone explains all variation. Illumination, lens effects, film/process variation and reference placement are not separately isolated here. No compensation was silently applied.

## Verification and evidence

The historical `film-model-images.rs` probe was built and run for both references. Its two tests pass when invoked directly with the local OCIO runtime library path:

- Anchors, monotonicity, signed base-boundary checks and the full scan-floor domain at the tested settings, for both corrected references.
- Explicit rejection of a shoulder pole at low gamma, not silent clamping. Consequently k=0.02 cannot be treated as universally valid over all existing gamma/balance settings.

Forward/inverse density checks have maximum error `1.12e-15`. The 106 generated TIFFs are recognized with profile `Antitone Display P3 v1`. Full-image contacts, native-resolution crops, source reference context and exposure sheets were inspected. The full production suite was not rerun because no production implementation changed.

Source and compositor:

- `docs/research/tools/film/src/bin/film-model-images.rs`
- `docs/research/tools/film-model-images/contact.swift`

Logs under `.cache/probes/film-model-images/`:

- `corrected-build.txt`, `corrected-whole.txt`, `corrected-clear.txt`
- `corrected-test-build.txt`, `corrected-tests.txt`, `corrected-image-metadata.txt`

Image directories:

- `run2-whole-dmin/`: three-image comparison, dog/canopy crops, dog exposure sheet, all full-resolution crop TIFFs, source base contexts.
- `run3-clear-dmin/`: dog sensitivity comparison/crops/exposure sheet using measured clear film.

The source is preserved as a pre-integration snapshot; it predates the current export API. See [tool status](tools/README.md) before attempting reproduction. At the original checkpoint, the Dmin build/run pattern selected bin `film-model-images`. No arguments produces the whole-picture pass; `--clear-film` produces the dog-only sensitivity pass. Exports refuse overwrite: change the probe output directory to a fresh ignored path before reproducing.

## Decision boundary

The photographic check supports a shared adjustable toe as a practical generic starting model, with shoulder off as a defensible starting choice for these shadow examples. It does not establish a universal film profile/default, eliminate the need for grading, or require a rendering change.

Production integration remains separate: settle parameter/normalization and signed/noise semantics, preserve existing grades through explicit model/version handling, and verify CPU/GPU/f32, actual-model fitting, resets, persistence, source bypass and TIFF/AP0 EXR behavior. No editor control, default, processing version, shader or export contract was changed in this pass.
