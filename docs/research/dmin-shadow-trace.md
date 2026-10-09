# Dmin-first shadow diagnosis: DSC04393

Completed diagnosis, not pending work. This report describes the original-response diagnostic checkpoint. Later [film-response production integration](film-response-precision-gate.md#production-film-response-checkpoint) is complete; dark-source editor inspection and flat-field work remain separate.

## Findings in plain language

The screenshot's Dmin is too low for much of this scan. With the existing inversion and all other settings held fixed, a better reference removes nearly all of the fully black pixels in the tested fur regions.

Searching the whole developed image independently of the margins finds an interior estimate close to the measured clear-film reference—not a dramatically brighter base hidden in the middle. The strongest coherent picture-area patch is under the dog's muzzle. It is a **Dmin estimate from a photographed shadow**, not proof of truly unexposed film or zero scene exposure.

The stage trace locates the hard loss: the low reference makes some inverted values negative; after colour conversion, pixels with all three working channels nonpositive become zero at the existing ACES input range limit. The scan and signed working data still contain variation up to that point. No additional hard cutoff of positive working RGB was found in the tested regions, and the GPU agrees with the CPU reference.

**This explains the hard clipping, not the entire appearance.** Correcting Dmin alone leaves much of the fur very dark at the assumed grade. Those remaining positive values are present in working data but render at low brightness. An explicitly labelled exposure-only comparison reveals substantially more of them. This is not evidence of a second implementation defect, nor proof that a toe change is required to fix one.

ACES, the production inversion, controls, references in the editor, shaders and exports have not been changed. No pedestal, flat-field correction or new toe model was applied in this initial diagnosis. The subsequent [corrected-Dmin plus toe image check](film-response-image-validation.md) tests that combination separately.

## Conditions and limits

- Source: `samples/DSC04393.ARW`, read-only. The user confirmed the NLP website screenshot is a conversion of the same source, horizontally flipped and differently cropped. It is a qualitative detail/tonal-separation reference, not pixel-aligned ground truth or calibrated radiometry.
- LibRaw development is unchanged. Fixed WB `[2133,1024,1771,1024]`; selected-source as-shot WB `[2131,1024,1774,1024]` is not substituted.
- Rounded screenshot settings: Dmax `[0.008061,0.003409,0.001442]`, gamma `0.65`, balance `(-0.41,-0.01)`, Roll exposure `+3 EV`.
- **Photo WB and exposure are assumed zero**, because the original screenshot only showed Roll settings. These are not reconstructions of the exact live grade, and cannot establish an exact NLP appearance match.
- AP0 inversion, signed AP1 photography, unchanged OCIO ACES 2.0 100-nit Display P3 rendering and matching P3 TIFF encoding.
- The floor is the existing projected precision bound, approximately `2.16309e-5`; it is not used as a noise estimate.
- Clear-film spatial variation is recorded, but its cause is not isolated. It is consistent with illumination/lens falloff; neither explanation is proven separately. Flat-field investigation/remedy remains deferred.

## Reference validation

The developed negative was inspected with its margins and surrounding holder visible. A `+2 EV` gain was used **only to make the negative inspection images easier to see**; all reference measurements are unscaled. The selected clear-film patch lies below the photographic image and above the dark holder boundary, not in a perforation or on the holder.

A previously reused bottom-centre rectangle around `(4000,5644)` partly overlaps the photographic image on this frame. It is not interchangeable with a wholly clear-film patch. This does **not** identify the provenance of the screenshot's stored RGB: that sampling rectangle is unknown.

Each measurement uses every pixel in its rectangle. The selected clear-film patch has 4,096 pixels, no raw CFA sites at/below black or at/above white, and camera-channel coefficients of variation of roughly 2–3%. The interior estimate has 1,024 pixels and similarly passes the sensor checks.

| Reference | Rectangle `(x,y,w,h)` | Linear AP0 RGB |
| --- | --- | --- |
| Rounded screenshot | Unknown | `[0.140103,0.113156,0.102145]` |
| Clear film, bottom | `(5000,5660,128,32)` | `[0.165512,0.133687,0.119013]` |
| Independent clear-film holdout | `(4000,5680,128,32)` | `[0.164085,0.132210,0.118074]` |
| Clear film, left | `(36,2500,32,128)` | `[0.156397,0.125861,0.112509]` |
| Clear film, bottom right | `(7000,5644,128,32)` | `[0.158911,0.127807,0.115902]` |
| Whole-picture estimate | `(5588,3780,32,32)` | `[0.168566,0.133592,0.117151]` |

The bottom clear-film value is approximately 17–18% above the screenshot reference. The whole-picture estimate is 15–20% above it, but differs from the bottom measurement by only about `(+1.8%,-0.1%,-1.6%)`. It is not a uniform upward correction from the margin value.

The left and bottom clear-film samples differ appreciably. A single RGB reference cannot establish the illumination at every image position in a spatially nonuniform capture. Do not interpret scan values above a measured mean as film physically less dense than its true base.

## Whole-image search, independent of edge measurements

The search covers **49,766,400 developed pixels** in the metadata-defined active image: `(12,12,8640,5760)`. RAW padding outside that area is not treated as photographed film.

1. Measure all 48,600 non-overlapping `32×32` tiles at full resolution.
2. Rank their mean **camera RGB** by its geometric mean. This selection uses no edge reference, colour-conversion matrix, inversion or display rendering. Fixed per-channel WB scaling multiplies all scores by the same factor, apart from development/quantization effects.
3. Distinguish the strongest active-image candidates from the strongest candidates inside the established picture crop `(192,96,8136,5424)`. The active-image winners lie on the extreme right margin; they are recorded rather than silently used as interior references.
4. Refine alignment around the 16 strongest interior grid candidates with offsets `-16,-8,0,8,16` pixels. Check raw sensor validity before accepting the selected patch.
5. Check centred `16`, `32`, `64` and `128`-pixel squares and larger patches at strong grid locations. The 16/32 measurements are close; larger patches include visibly different image content and transmit less.

A preliminary search ranked by the minimum camera-channel ratio to the clear-film reference. The final independent geometric-mean search selected the same interior patch. Only the latter is used to claim independence from the edges.

Camera-channel maxima and upper percentiles were also recorded over the whole active image. These extrema are **not assembled into a synthetic RGB reference**: their channels need not come from the same location, and isolated bright samples include grain/noise and boundary effects. The selected Dmin estimate is one actual patch's RGB.

This is a strongest-coherent-patch estimate, not an exhaustive sliding-window optimum or a calibrated sensitometric base measurement. No image-only search can guarantee that a photographed black subject received zero exposure.

## Where variation is lost

For the same full-resolution pixels, the probe follows:

1. Developed camera RGB and raw sensor diagnostics.
2. Camera-to-AP0 conversion.
3. Existing signed AP0 inversion.
4. AP0-to-AP1 conversion and unchanged exposure/assumed Photo WB.
5. Existing componentwise AP1 range `[0,1024]` before ACES.
6. Unchanged ACES rendering to linear P3, then output encoding.

None of the tested fur, wood or sky patches has sensor black/white clipping or an input floored by inversion. Neither inversion nor the post-inversion matrix discards negative values.

For every tested case, explicitly clamping working RGB before calling the production renderer gives **bit-identical rendered output** to calling it on the signed working values. All fully black output pixels have all-nonpositive working RGB. There are **zero** fully black output pixels with any positive working channel in these patches.

Each fur patch contains 196,608 source pixels (`512×384`):

| Fur region | Rectangle | Exact-black pixels: screenshot Dmin | Clear-film Dmin | Whole-picture estimate |
| --- | --- | ---: | ---: | ---: |
| Forehead | `(5750,2400,512,384)` | 6,555 | 0 | 0 |
| Muzzle | `(5100,3350,512,384)` | 2,458 | 5 | 5 |
| Chest | `(6650,4200,512,384)` | 37,697 | 394 | 86 |

The whole-picture estimate takes the chest from about 19% exactly black to about 0.04%. These counts differ from earlier coarse probes because the source rectangles and sampling coverage differ; they are not directly interchangeable.

Positive near-black tones remain important. With the whole-picture estimate and unchanged grade, approximately 3.5% of the forehead, 1.7% of the muzzle and 21.6% of the chest are below a grey-luminance code of `1/255`. This is a diagnostic brightness threshold, **not a claim of an 8-bit cutoff**: the renderer is float and the TIFFs are 16-bit.

The unchanged-model `+2 EV` view demonstrates that much of this texture is still present. It also brightens the background; it is explicitly an exposure test, not a Dmin-only result, a proposed default or proof of NLP-equivalent highlight separation.

### Colour-basis caveat

Camera-to-AP0 has negative coefficients. Even if each camera channel is below its camera-space base reference, an AP0 component can exceed its AP0 reference. Therefore not every AP0 overshoot identifies illumination or film-density inconsistency. The trace records camera and AP0 comparisons separately. No inversion-basis change or density-unmixing experiment was made.

### Why a Dmin change mainly moves the black threshold

For fixed gamma and Dmax, per channel, let:

```
U = (Dmax / max(scan,floor))^(1/gamma)
z = (Dmax / Dmin)^(1/gamma)
P = (U - z) / (1 - z)
```

Changing Dmin gives an affine relation between the old and new working values:

```
P_new = ((1-z_old)/(1-z_new))*P_old
        + (z_old-z_new)/(1-z_new)
```

This describes the effect of correcting the reference; it does not justify inventing a pedestal instead of measuring one. In these cases the scale change is small, and pre-render texture variation changes very little. Moving the zero threshold prevents the renderer from flattening many distinct negative values into black, but does not independently change the toe shape or choose a brighter grade.

## GPU/reference checks

The existing Metal processing kernel and complete pinned P3 GPU renderer were checked on 3,223 samples: 3,072 full-resolution fur samples plus near-base rays and representative traced pixels. Cases: screenshot Dmin, clear-film Dmin, whole-picture estimate, and whole-picture estimate with `+2 EV`.

- No GPU-black/CPU-nonblack samples in any case.
- Maximum working absolute error: `1.20e-6` across the cases.
- Maximum linear rendered error: `2.12e-6`.
- Maximum encoded channel error: `2.66e-6`.
- No floor flags for these samples. Existing gates were not weakened.

All 20 final-case TIFFs reopen with profile `Antitone Display P3 v1`. Comparisons and native-resolution fur crops were inspected. These checks do not establish physical-monitor matching or exact current-grade acceptance. Production code is untouched; the full application suite was not rerun for the offline probes/documentation.

## Evidence and reproduction

Preserved historical source under `docs/research/tools/film/src/bin/` ([tool status](tools/README.md)):

- `dmin-survey.rs`: developed-source contacts, full-resolution margin measurements and sensor checks.
- `dmin-whole-search.rs`: full-active-image search, candidate ranking and patch-size sensitivity.
- `dmin-trace.rs`: unchanged-model reference comparisons and stage traces.
- `dmin-gpu-check.rs`: existing processing/rendering GPU parity checks.

Evidence under `.cache/probes/dmin-trace/`:

- `run1/`: margin survey, source/holder inspection, clear-film-only comparison, initial edge-normalized whole-picture search.
- `whole-search2/`: independent whole-image candidate CSV, chosen rectangle/RGB, source crop. Console output: `.cache/probes/dmin-whole-independent.txt`.
- `run2/`: final screenshot/margin/whole-picture comparison, full-resolution crop TIFFs, `trace.txt`, representative stage CSV, `gpu-check.txt`, image metadata.
- Final overviews: `run2/comparison.png`, `run2/fur-crops.png`, `run2/whole-picture-position.png`, `run2/whole-picture-candidate.png`.
- `docs/research/tools/dmin-trace/{source-contact,contact,contact-whole}.swift`: preserved explicit-size AppKit inspection/compositing, not application presentation.
- Compact [whole-image search log](evidence/dmin-whole-independent.txt) and [GPU check log](evidence/dmin-trace/run2/gpu-check.txt) are retained outside the cache. Other original evidence paths above remain local/regenerable.

Historical build pattern (some listed tools predate current APIs; see the tool status before reproducing):

```sh
CARGO_TARGET_DIR="$PWD/target" CC=/usr/bin/clang CXX=/usr/bin/clang++ \
  ./scripts/cargo.sh build --offline --release \
  --manifest-path docs/research/tools/film/Cargo.toml --bin BIN
DYLD_LIBRARY_PATH="$PWD/.cache/deps/ocio-install/lib" target/release/BIN
```

TIFF exports refuse overwrite. Choose fresh ignored output directories before reproducing the survey, search or trace. GPU checking reads the generated source/parameter fixtures; it does not modify images or application state.

## Consequence for the film-response work

The [published-film study](film-response-model-study.md) remains evidence for an independently adjustable toe, not a diagnosis of this particular grade. The later image probe held the screenshot Dmin fixed to isolate model differences. It must not be used to conclude that changing the toe is necessary to repair the hard clipping identified here, or that Dmin alone fully explains the desired appearance.

The subsequent [image check](film-response-image-validation.md) repeats the candidate toe with both corrected Dmin references, actual-model balance fitting and matched midtone brightness. It supports the shadow-opening direction without an ACES change, but is not an exact live-grade/NLP match or a production integration gate. Flat-field correction, signed/noise semantics and versioned CPU/GPU integration remain separate work.
