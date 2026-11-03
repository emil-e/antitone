# Independently sourced B&W materials

Antitone reconstructs characteristic curves and film sensitivities from manufacturer publications. These are source-derived approximations, not manufacturer-authored profiles or laboratory measurements. Source records retain publication/process details, graph readings, uncertainty allowances and reconstruction choices; generation does not fit photographs. Trace complete clipped vector curves, including separate baseline/toe strokes: a single long path can omit published endpoints.

For new graph-derived materials, reconstruct a smooth mean density-versus-log-exposure response within the source-reading allowances, retaining toe, contrast, shoulder and broad knees. Ink-trace slope fluctuations are not measured material character. The current authoring tool uses low-complexity monotone cubic B-splines with fixed endpoints; this is a reconstruction choice, not measured material bandwidth. Source records retain readings alongside reviewed controls. Component generation evaluates those controls without refitting. Published analytic responses can be retained directly.

| Material | Maintained source |
| --- | --- |
| Ilford Multigrade RC Deluxe · filter 2 | [ILFORD J20, October 2020, p3](ilford-multigrade-rc-deluxe-filter-2/source.json): MULTIGRADE 1+9, 20 °C, 60 seconds, new-generation RC DELUXE/PORTFOLIO |
| Ilford Multigrade FB Classic · filter 2 | [ILFORD, October 2013, p2](ilford-multigrade-fb-classic-filter-2/source.json): MULTIGRADE 1+9, 20 °C, 120 seconds; Japanese manufacturer edition |
| Fomaspeed Variant glossy · filter 2 | [FOMA 04/25, p2](fomaspeed-variant-glossy-filter-2/source.json) |
| Fomabrom Variant glossy · filter 2 | [FOMA 04/25, p2](fomabrom-variant-glossy-filter-2/source.json) |
| Ilford FP4 Plus · Ilfotec HC 8 min | [ILFORD, November 2018, p5](../negatives/ilford-fp4-plus-ilfotec-hc-8m/source.json): HC 1+31, 20 °C, intermittent agitation |
| Ilford HP5 Plus · Ilfotec HC 6½ min | [ILFORD, November 2018, p5](../negatives/ilford-hp5-plus-ilfotec-hc-6m30s/source.json): HC 1+31, 20 °C, intermittent agitation |
| Ilford Delta 100 · ID-11 8½ min | [ILFORD, November 2018, p4](../negatives/ilford-delta-100-id11-8m30s/source.json): stock developer, 20 °C, intermittent agitation |
| Ilford Delta 400 · ID-11 8 min at 24 °C | [ILFORD, November 2018, p6](../negatives/ilford-delta-400-id11-8m-24c/source.json): stock developer, intermittent agitation |
| Kodak T-Max 400 · D-76 8 min | [Kodak F-4043, February 2016, p8](../negatives/kodak-tmax-400-d76-8m/source.json): stock developer, 20 °C, small tank |
| Kodak T-Max 100 · D-76 7½ min | [Kodak F-4016, June 2018](../negatives/kodak-tmax-100-d76-7m30s/source.json): stock developer, 20 °C, small tank |
| Kodak Tri-X 400 · D-76 8 min | [Kodak F-4017, December 2016](../negatives/kodak-trix-400-d76-8m/source.json): 400TX/35mm, stock developer, 20 °C, large tank |

The FOMA plots identify filtration but do not separately specify development conditions; recommended processing ranges in their records are not exact curve-process claims.

## Formation and printing

Ilford's spectral plots are interpreted as linear relative sensitivity, with the plot baseline at zero. The 2850 K tungsten caption is retained as a test condition; no extra source-spectrum division is applied. Embedded-raster coordinates and reading allowances are recorded; printed ticks provide piecewise axis calibration. This is a documented interpretation, not manufacturer-confirmed radiometric calibration.

The [native provider](../../src/looks/monochrome.rs) projects the shared unwindowed Hanatos scene-spectrum family through each negative's scalar sensitivity. CAT16/D55 and configured input-gamut compression precede exposure; colour-stock adaptation windows, dyes and DIR are absent. Spectra remain estimates from RGB. Sensitivity outside the published wavelength extent is set to zero rather than extrapolated.

Film development supplies one total density, interpreted as neutral transmission `T(λ) = 10^(-D)`. With fixed printing light and filtration, this gives paper log exposure equal to a calibrated constant minus negative density. Fixed-grade paper curves therefore suffice for this neutral-negative model without inventing paper emulsion channels. Cross-family printing is not implemented; [editor selection](../../docs/editor.md#colour-selection-prepares-before-acceptance) confirms changes to supported combinations.

The relative film exposure convention places AP1 gray 0.18 four stops above the retained reading's Dmin+0.1 point; a fitted response uses the original trace for this placement, so reconstructing its toe does not shift exposure calibration. Scalar print time anchors gray to the shared authoring view. Without a synthetic negative, positive AP1 luminance uses the 0.6-stop compression adapter. Neither convention changes source density endpoints. Curves extend constantly outside their published range; an endpoint without a plateau is not measured film Dmax.

Paper total density becomes neutral reflection `R(λ) = 10^(-D)`, with no additional base. Measured paper-white/image-tone spectra are unavailable. Pinned Spektrafilm algorithms supply interpolation, one-absorber density-to-light conversion, D50/CIE1931 readout, CAT02 adaptation and configured CAM16-UCS finishing. The shared inverse authoring DRT produces styled AP1; final LUTs bake on demand for ordinary tetrahedral execution.

## Add a film or paper

Record publication, process, exposure range, graph coverage intervals and reading uncertainty in a source record. For negatives, also retain the spectral readings and their declared scale. Retain a monotone characteristic trace in `reconstruction.samples` with family `tabulated`; record uncertainty from the source before fitting.

Propose a smooth reconstruction with the standard-library [authoring tool](../../scripts/spektrafilm/reconstruct.py):

```sh
python3 scripts/spektrafilm/reconstruct.py .cache/new-material/source.json --output .cache/new-material-fit
```

The tool tries a bounded coarse-to-fine sequence, selecting the first fit inside both trace and graph-coverage allowances. It writes `source.json`, `audit.json` and a trace/fit `curve.svg` to a fresh candidate directory. If no candidate fits, inspect the readings and model choice rather than enlarging uncertainty to obtain a pass. Already fitted sources are refused, so adding a material does not refit accepted curves.

Review the plot and audit, then place the candidate source under `assets/negatives/<process>/source.json` or `assets/papers/<process>/source.json` and follow regeneration below. Review the resulting treatment on ordinary rendered gradients before finalising it; keep sources, components and fixtures together.

## Regeneration

Use the [pinned environment and OCIO setup](../spektrafilm/README.md#regeneration):

```sh
.cache/spektrafilm/venv/bin/python scripts/spektrafilm/monochrome.py generate --output .cache/bw-candidate
python3 scripts/spektrafilm/monochrome.py check --output .cache/bw-candidate
```

After inspecting the candidate, promote `components.json.z`, `spectral.f32.z`, `index.json` and `provenance.json` together to `assets/monochrome/`, and `conformance.json.z` to `tests/fixtures/monochrome/`. Then run:

```sh
python3 scripts/spektrafilm/monochrome.py check
cargo test --locked --release --features ui-tests --lib looks::monochrome
python3 -m unittest scripts.spektrafilm.test_monochrome
```

[Provenance](../monochrome/provenance.json) binds sources, producers, dependency lock, components and fixture bytes. Tests check source-model conformance, GPU/CPU sampling of the same numeric cube, and neutral exposure order through both delivered targets separately. Final-cube error statistics locate approximation differences rather than set visibility thresholds. Judge bake quality on ordinary exposure, hue and saturation gradients through the actual target DRT and encoding.

Algorithms retain the [Spektrafilm GPLv3 code license](../spektrafilm/LICENSE-code.txt); negative reconstruction also uses its shared [CC BY-SA Hanatos data](../spektrafilm/LICENSE-data.txt), but no colour-stock profile. Manufacturer publications retain their rights. Public-distribution clearance remains part of the separate [release review](../../NOTICE.md#local-packages-versus-public-releases).
