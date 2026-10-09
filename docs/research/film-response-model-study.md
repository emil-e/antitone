# Published colour-negative response: toe and shoulder model study

Historical model study, retained as evidence rather than a current task list. Its candidate was subsequently validated and integrated: [production checkpoint](film-response-precision-gate.md#production-film-response-checkpoint), [accepted response decision](../decisions/versioned-film-response.md). Limits below describe what this study itself established.

## Conclusion in plain language

A simple adjustable film-response model can follow the published Portra 400 and Ektar 100 curves reasonably well. It does not need to be derived from ACES. ACES can remain the final display renderer.

The two ends deserve different treatment:

- **Toe:** both films have a more localized transition out of the base region than the toe implied by Antitone's current inversion. A separate toe-shape parameter is supported by this study.
- **Shoulder:** ignoring it is a reasonable approximation for Portra over the published range. Ektar has a modest but consistent decline in upper-end contrast, especially in the green and red measurements. A gentle optional shoulder term is worth retaining; a strong textbook shoulder is not justified as a universal default.

A rounded shared toe shape corresponding to `beta = 2.5` is a promising starting candidate. This is a generic parameter choice from six traces in two films, **not a statistical median of photographic film and not a calibrated profile for the user's roll**.

This study changes no production processing, GPU assets, editor defaults, schema or exports. No black offset is proposed as a physical consequence of these fits.

The subsequent [Dmin-first shadow trace](dmin-shadow-trace.md) checks the dog frame with both clear-film measurements and an independent whole-image estimate. A better Dmin removes almost all of its hard-black clipping with the existing inversion; the remaining dark positive values survive in working data. The [corrected-Dmin plus toe image check](film-response-image-validation.md) then opens those shadows with midtone brightness matched and ACES unchanged. These results support the candidate direction, not a second processing-defect diagnosis or exact live-grade acceptance. Flat-field correction remains deferred.

## Primary sources and conditions

Both documents are Kodak Alaris technical publications hosted by Kodak Moments:

- [Portra 400, E-4050, February 2016](https://business.kodakmoments.com/sites/default/files/files/resources/e4050_portra_400.pdf), page 4.
- [Ektar 100, E-4046, revised February 2016](https://business.kodakmoments.com/sites/default/files/files/resources/e4046_ektar_100.pdf), page 4.

Each characteristic graph plots three density responses against log exposure. The graph labels specify daylight exposure and Status-M densitometry, with different exposure references. The films' recommended process is C-41; the documents do not provide raw measurement tables or a complete measurement uncertainty budget.

[Kodak's Basic Photographic Sensitometry Workbook H-740](https://www.kodak.com/content/products-brochures/Film/Basic-Photographic-Sensitometry-Workbook.pdf) distinguishes toe, straight-line portion and shoulder, and defines gamma as the slope of the straight-line portion. This supports separating toe shape from midrange contrast rather than expecting one gamma parameter to describe the whole response.

The study uses the manufacturer's published drawings. It cannot establish how representative they are of every production batch, lab process or real roll.

## Extraction and precision

The PDFs contain vector traces, so the curves were recovered from their actual drawing paths rather than estimated by eye from screenshots. The extraction is deliberately specific to the inspected page-4 content streams, not a general PDF parser.

- Portra's characteristic curves are polylines.
- Ektar's traces combine polylines and cubic Béziers. Cubics were subdivided into 256 parameter intervals before exposure-space interpolation.
- PDF axis coordinates convert drawing coordinates into log10 exposure and density. Axes and curve identities were checked against rendered original pages.
- Each trace was resampled at 161 equally spaced exposure positions. This avoids giving extra weight to the more numerous vertices around the toe.
- These interpolated positions are **not independent laboratory samples**. No statistical confidence interval or significance test is inferred from their count.

Comparing 64 and 256 Bézier subdivisions changed Ektar's resampled densities by at most `3.03e-5`. This numerical difference is much smaller than the chart's drawn line width, approximately `0.0208` density units. Line width is useful scale context, not a manufacturer measurement error bar.

The trace covers about four log10 exposure units for each curve. Conclusions do not extend automatically to unplotted saturation or very low exposures.

PDF hashes, axis bounds, offsets, stroke width and subdivision count are retained in [source/extraction provenance](evidence/manufacturer-sources.json), copied from the original local `extraction.json`. Downloaded PDFs and extracted curve traces remain under `.cache/`; no drawing/PDF is redistributed here.

## Candidate response model

The model is a smooth toe followed by a long approximately straight middle, with an optional gentle upper bend. It is deliberately small and has a direct inverse.

For the technical definition, let `x` be log10 exposure and `D` measured density:

```
u = w * softplus((x - t) / w)
D = b + gamma * u / (1 + k * u)
```

- `b`: base density.
- `t`: exposure placement; it absorbs film speed/exposure-position differences.
- `w > 0`: toe transition width, in log10 exposure units.
- `gamma > 0`: contrast coefficient. With `k = 0`, it is the limiting straight-line slope. With a shoulder term, the actual midrange slope also depends on `k`; these must not be silently presented as the same UI contract.
- `k >= 0`: optional upper curvature. Zero removes this term exactly.

This is a modelling function, not a claim about film's chemical mechanism. Positive parameters make the response monotonic. With `k = 0`, its upper response is linear. With positive `k`, its derivative gradually declines. The finite asymptote `b + gamma/k` is **not identified as a real film Dmax by these plots**, because the published curves do not show a saturation plateau. Do not use that extrapolated limit as measured physical data.

The inverse, inside its physical domain, is:

```
v = (D - b) / (gamma - k * (D - b))
x = t + w * log(expm1(v / w))
```

At the base, zero exposure is the limiting value. Below base or at/beyond the shoulder limit, this physical inverse does not supply a finite scene exposure. A future production model needs explicit signed/noise and range semantics, not a silent clamp or an invented recovery of saturated detail.

### Relationship to the current inversion

The unchanged inversion is:

```
E = (10^(delta_D/gamma) - 1) / (10^(delta_D_white/gamma) - 1)
```

Its implied forward response is `delta_D = gamma * log10(1 + K*E)`, with `K` set by the white reference. This is the toe-only family above with **fixed `w = 1/ln(10)`, or beta 1**. Gamma adjusts the straight-line slope, but cannot independently change that toe width.

For a generalized toe-only member, `beta = 1/(w*ln(10))`. The earlier illustrative beta-2 experiment was therefore a member of this family, not merely a power applied to the already-inverted image.

The stock curves are Status-M optical density measurements. Antitone's log ratios of developed AP0 RGB are not equivalent spectral measurements. The comparison supports a shape candidate; it does not prove that these fitted channel parameters can be copied directly into camera/AP0 inversion or that the user's symptom is wholly caused by toe width.

## Fitting and checks

Each density trace was fitted independently. Base and contrast use constrained linear least squares for fixed nonlinear parameters; toe placement/width and optional upper curvature use a multi-start Nelder–Mead search. The implementation uses the Python standard library, with no global package installation.

The asymptotic base is constrained to `0..minimum published density` so it cannot sit above the trace's left end. In a few cases this constraint changes the best width: base placement and toe width are coupled, particularly where a trace has small early structure that a one-toe model cannot reproduce exactly.

Comparisons:

1. Current fixed toe-width family, with base/contrast/placement refitted. This is a favourable comparison for that family, **not the actual camera grade or a test with measured Dmin held fixed**.
2. Free toe width, no shoulder.
3. Free toe width plus one shoulder parameter.
4. Shared, rounded `beta = 2.5` toe, with other parameters fitted independently per trace.

The fitted curves were monotonic over sampled source intervals, and model forward/inverse checks agreed to better than `1e-7` in log10 exposure. This checks implementation consistency, not inverse accuracy for real film or uncertain densities near base.

### Density fit residuals

Root-mean-square residuals against the extracted publisher trace:

| Film / channel | Current toe-width family | Fitted toe only | Fitted toe + shoulder |
| --- | --- | --- | --- |
| Portra B | 0.01959 | 0.00254 | 0.00254 |
| Portra G | 0.02261 | 0.00334 | 0.00184 |
| Portra R | 0.01664 | 0.00672 | 0.00672 |
| Ektar B | 0.02899 | 0.01285 | 0.00718 |
| Ektar G | 0.03389 | 0.01852 | 0.00304 |
| Ektar R | 0.03356 | 0.02066 | 0.00505 |

A one-toe model does not reproduce every small kink or change of contrast. Adding more parameters solely to follow those details would not establish a more accurate unknown-roll reconstruction.

### Shoulder evidence independent of the response model

Straight-line regressions on the actual traces compare the 45–70% portion of the plotted exposure interval with the upper 80–100% portion:

| Film / channel | Upper slope / middle slope |
| --- | --- |
| Portra B | 1.017 |
| Portra G | 0.987 |
| Portra R | 1.057 |
| Ektar B | 0.942 |
| Ektar G | 0.823 |
| Ektar R | 0.769 |

Portra does not show meaningful upper flattening in these windows; two responses become slightly steeper. Ektar's green/red upper contrast declines by roughly 18–23%. This supports a modest upper-bend option, not a universal strong shoulder or a measured saturation density.

A separate diagnostic fitted only the first 75% of the exposure interval and compared its straight upper continuation with the remaining 25%. Ektar's held-out upper densities consistently fell below that continuation. This is a shape diagnostic, not statistical cross-validation of independent measurements.

## Generic starting shape

Individually fitted toe widths with the shoulder option correspond to beta values of roughly `1.8..3.2`. A rounded shared `beta = 2.5` fit, allowing base, contrast, placement and shoulder to vary, has RMS residuals `0.003..0.008` density units, with maximum residuals of approximately one drawn line width.

This supports starting with **one shared adjustable toe**, rather than requiring three separate stock-calibrated toe controls. It does not show that one fixed full curve or one fixed shoulder amount fits all films.

A conservative prototype could use this shared toe shape and a weak/off optional shoulder, keeping ACES unchanged. Before choosing a universal default, more representative still-colour-negative curves and real-image comparisons would be useful. Portra and Ektar are two professional Kodak stocks, not a representative statistical sample of all photographic film.

## What has not been validated

- Exact stock/process identification, calibrated scene radiometry or camera colour inversion.
- Direct reuse of Status-M channel fits in AP0 camera data.
- Physical-monitor or exact-grade acceptance of a beta-2.5 model. Offline image comparisons are recorded separately; no new editor grade/default has been deployed.
- Behaviour near true saturation, where lost detail cannot be reconstructed.
- Signed-domain extension, noise/grain sensitivity and flat-field correction.
- Production GPU parity, f32 representability, full parameter-domain implementation, actual-model neutral-fitting integration or scoped UI/reset behavior. The offline CPU prototype checks stable inverses and actual-model fitting at the documented settings; it is not production integration.
- Versioning/default equivalence, persistence, source bypass and TIFF/AP0 EXR compatibility if a new processing model is adopted.

Measured Dmin remains a base reference. This study does not justify moving it above scene black or silently adding a pedestal. The existing [DRT-aware exploration](drt-aware-inversion-exploration.md) remains a separate historical experiment, not the basis for this candidate film response.

## Evidence

Authored extraction/fitting/plotting tools are preserved in [tools/film-response-docs](tools/film-response-docs/). Downloaded PDFs, extracted drawing data and generated plots remain local under `.cache/probes/film-response-docs/`; source URLs and limitations are recorded above. The original artifact set was:

- `extract-curves.py`: specific vector extraction with provenance and uniform exposure resampling.
- `fit-curves.py`: constrained fits, direct trace-slope diagnostics and inverse/monotonic checks.
- `plot-fits.swift`: curve overlay overview.
- `extraction.json`, `*-trace.csv`, `*-uniform.csv`, `fit-results.json`, `shared-toe-results.json`, `fit-predictions.csv`.
- `extraction-resolution-check.txt`, `fit-check.txt`, and `curve-fits.png`.

Reproduction uses the downloaded PDFs and these scripts: run extraction and fitting with Python 3, then plotting with the existing macOS Swift/AppKit toolchain. Later photographic evidence and reproduction are recorded in the [image-validation report](film-response-image-validation.md). Production files/tests have not changed; the full application suite was not rerun for these offline studies.
