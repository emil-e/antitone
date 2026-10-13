# Contrast and saturation in a scene-referred photographic pipeline

Status: retained research/comparison evidence, not the production specification. The selected simple endpoint families and blend semantics are now defined in the [processing contract](../contracts/processing-model.md#photo-contrastpivot-and-saturation) and [decision](../decisions/photographic-grading.md). The [focused plan](../plans/contrast-and-saturation.md) owns implementation/acceptance status; early shortlist proposals below are historical.

## Conclusions for Antitone

- The user clarified log-domain contrast/pivot, with ACEScct as a useful candidate and no additional S-curve. **Scene-referred does not itself specify the operator's domain**; the linear-light analysis below explains the distinction, not an unresolved user-intent question.
- “Pleasing saturation” has at least three independent meanings: restrained boosts to weaker colours (vibrance), increased chroma at constant luminance, and denser-looking colour accompanied by reduced luminance. There is no universally preferred or standardized subtractive-saturation algorithm.
- Cullen Kelly's HSV technique is concrete, useful prior art for the density-like direction, not proof of a physical film model or a signed/HDR AP1 implementation. Other practitioners explicitly disagree about its merits and terminology.
- A contrast colour-preservation option is defensible, but must name what it preserves. Constant chroma, constant chromaticity and constant perceived saturation are different contracts. A pre-DRT invariant cannot guarantee unchanged displayed saturation after nonlinear ACES rendering.
- Compare endpoints **and blends** through Antitone's actual outputs. The retained experiment included print-density power; the production shortlist excludes it and uses RGB/proxy contrast plus constant-Y/maximum-anchor saturation. Initial controls/defaults are implementation choices, not settled photographic acceptance or additional gamut mapping.

## Evidence and limitations

Sources were read as public HTML/text and, for the Resolve 18.5 reference manual, visually in a browser PDF viewer using the linked mirror. Vendor documentation is evidence of documented behavior; educator articles and forum posts are evidence of practice/preferences, not specifications of proprietary internals. The prior-art investigation did not automate Lightroom/Resolve or verify their exact proprietary equations. The separately selected [reference experiment below](#reference-experiment-checkpoint) now checks our own f64 candidates and renders bounded local comparisons; it is not a production CPU/GPU gate or photographic acceptance.

Adobe Help and Adobe product articles returned HTTP 403. Lightroom colour-control coverage therefore uses an identified secondary source. The Resolve 20 Colorist Guide was not text-readable in the browser fetch; the later visual reading of the 18.5 reference manual resolves the Primaries/HDR behavior question for that documented version, not current-version application verification or exact proprietary equations. Search-result snippets and general forum landing pages are not evidence here. Kodak's print-film text was read from an archived OCR derivative; its graph shapes were not independently measured.

The darktable manual page reports Development in its version selector; treat it as living documentation, not a verified installed-version behavior claim. In particular, its introductory/internal-order descriptions retain references to JzAzBz while its saturation-formula section also describes darktable UCS. Algorithm-specific reproduction requires source/version inspection.

## Application prior art

### Lightroom: ordinary saturation versus adaptive vibrance

[Elizabeth Halford, *Vibrance vs Saturation in Photography*, Digital Photography School][lightroom-secondary] shows both Lightroom Classic sliders with photographic examples. Saturation increases the intensity of all colours; vibrance preferentially increases muted colours and is described as protecting warmer/skin colours. The author recommends evaluating the controls separately and sometimes combining them rather than treating either as universally superior.

This supports a familiar **adaptive colour-boost** interaction and explains why portraits can benefit from it. It does not disclose Adobe's colour space, exact skin weighting, transfer functions, gamut handling or contrast/saturation coupling. The article's explanation of warmer-colour protection is an educator's description, not an independently verified skin classifier.

Lightroom exposes both Contrast and a Tone Curve, but this research did not retrieve a readable primary explanation of their current processing relationship. Do not describe Lightroom Contrast as scene-linear scaling, claim a particular S-curve, or emulate a proprietary midpoint from this evidence. [Adobe's tone-adjustment documentation][adobe-tone] remains a follow-up source, not a retrieved algorithm reference.

**Implication:** Lightroom-style vibrance is a separate alternative to density saturation. Skin protection adds hue-dependent policy; it should not appear invisibly in a supposedly uniform saturation control.

### Resolve: compact primary tools, adaptive colour boost and explicit colour shaping

[Blackmagic's Color page][resolve-color] documents primary contrast/saturation controls, separate custom RGB/luminance curves, and Hue vs Sat, Hue vs Lum, Lum vs Sat and Sat vs Sat curves. It describes Color Boost as vibrance that preferentially adjusts low-saturation areas for a subtle, more natural-looking result. The HDR palette is advertised as perceptually colour-constant and configurable by tonal zones; the page does not expose its equations.

[Cullen Kelly's contrast article][kelly-contrast] demonstrates using exposure, Contrast and Pivot to adapt an image's photographed contrast ratio. His recommendation is often less post contrast for high-contrast photography and more for low-contrast photography, expressly not a universal prescription. The example adjusts Pivot to direct the tonal effect rather than forcing every scene into one look.

The [Resolve 18.5 reference manual, p. 2883][resolve-primaries] says disabling **Use S-curve for contrast** makes Primaries Contrast adjustments linear, rather than rolling off shadow/highlight extremes. Contrast/Pivot is shared across Primaries modes; selecting the Log wheels palette does not itself choose an ACEScct encoding. Linear scaling in the current log-encoded signal is not affine scene-linear-light scaling. The candidate equations below express that distinction, not a verified mapping of Resolve's numeric strength/pivot controls.

The [HDR chapter, pp. 2900–2906][resolve-hdr] documents an internal perceptually uniform operational space, colour-space awareness and contrast adjustments that preserve **perceived saturation**. It explicitly allows small changes to numerical saturation to maintain that perception; the vectorscope can change. HDR Contrast ignores the S-curve project setting, and tone mapping affects preservation at the outer tonal boundaries. The manual does not identify enough equations to reproduce its perceptual model exactly.

Page 2884 also documents Color Boost as nonuniform low-saturation weighting and Luma Mix as balancing YRGB/master/ganged-curve adjustments with Y-only adjustments, plus cross-channel luma compensation for individual channel edits. This is not a specification of the HDR preservation algorithm.

**Implication:** compare ordinary RGB ACEScct contrast with a declared preservation endpoint and their blend. Chromaticity preservation can be a useful proxy, but is not a faithful definition of Resolve HDR behavior. The user welcomes preserved colourfulness when reducing contrast and often welcomes the coupled colour boost when increasing it; neither endpoint is presumed universally better.

### darktable: explicit separation of chroma, saturation, brilliance and contrast

The [Color Balance RGB manual][darktable-balance] is the strongest retrieved application-level technical comparison:

- It expects scene-referred linear RGB, with creative operations that can break radiometric linearity without making the output display-referred.
- Contrast acts on luminance at constant hue/chroma, with a grey fulcrum documented as 18.45%; the manual warns that it expands dynamic range and can invalidate downstream filmic settings.
- Linear chroma adjustment preserves luminance and hue; perceptual saturation changes luminance and chroma together at constant hue.
- Vibrance prioritizes low-chroma colours.
- It offers JzAzBz and darktable UCS saturation formulas. The manual identifies excessive near-black darkening and the missing Helmholtz–Kohlrausch effect as limitations of the older choice.
- The module includes working-gamut mapping, which can change an image even at nominally neutral settings.

The [colour-dimensions chapter][darktable-dimensions] explains why luminance, brightness, chroma and saturation are not synonyms, and why HSV/HSL coordinates are not perceptually uniform.

**Implication:** a luminance-only contrast tool is meaningful, but constant chroma is not constant relative saturation. Perceptual controls bring model/viewing assumptions and domain constraints. Do not port darktable's always-on gamut mapping: Antitone currently preserves signed/HDR AP1 and leaves output rendering to unchanged ACES.

## Grading practice and community disagreement

### Cullen Kelly: a denser alternative to the ordinary saturation knob

In [*The Simple Technique for “Cinematic” Color Saturation in DaVinci Resolve*][kelly-saturation] (2023), Kelly first establishes colour management and balance. He notes that increasing saturation amplifies existing colour imbalance. He then demonstrates an HSV node with only its S channel enabled, adjusted with Gain; Gamma or a custom S curve can preferentially boost lower saturation rather than already-hot colours.

His preferred result becomes darker as it becomes more colourful, which he calls subtractive saturation. This is an aesthetic/practical preference, not a derivation of dye absorption. HSV Value is loosely brightness-related, not physical luminance; holding it constant can still reduce luminance. Kelly explicitly presents alternatives rather than a universal rule.

**Implication:** evaluate density-like saturation after exposure/WB, and test both a uniform increase and a restrained low-saturation increase. Saturation cannot repair an incorrect balance.

### LiftGammaGain: HSV is not a consensus film-density model

[*The right way to emulate subtractive colour?*][lgg-subtractive] (2023) discusses Kelly's method alongside direct density/DCTL approaches. The questioner distinguishes making newly added saturation darker from darkening colour already present in the input. Chris Wright questions whether HSV behaves like proper subtractive colour and raises artifact concerns; another participant reports finding HSV Color Warper gentler than the DCTLs they tried.

This is useful disagreement, not a controlled comparison or consensus. It also distinguishes **saturation-induced darkening** from an independent **density treatment of already-colourful pixels**. These should not be merged without a deliberate design choice.

**Implication:** “Density” must specify whether neutral settings leave all existing colours untouched. Antitone needs exact identity defaults, not an always-on film-look transformation disguised as zero saturation adjustment.

### ACESCentral: the DRT is part of the visible result

[*ACES 2.0 Output Rendering*][aces-rendering] (2025) compares difficult saturated-light/CG examples across ACES versions and other DRTs. Stu Maschwitz reports improved hue behavior/posterization but questions lost yellowness. Doug Walker explains the tradeoff among lightness, saturation and hue in gamut mapping; strict hue preservation may require substantial desaturation. Participants caution against generalizing an extreme synthetic example to normal imagery.

[*ACES 2.0 grading colorful lights*][aces-lights] (2025) reports difficult transitions when trying to make intense lights pop, while other participants prefer ACES 2's aesthetic treatment. This documents real workflow preferences and disagreements, not a defect diagnosis for Antitone or permission to replace its DRT.

**Implication:** judge each candidate through both current 100-nit outputs. More pre-DRT chroma does not necessarily produce proportionally more visible saturation. Preserve scene-space invariants without claiming invariance in rendered output.

### Print-film inspiration is a model family, not a saturation knob

[Kodak's 2383/3383 technical data][kodak-print] describes blue-sensitive/yellow-forming, red-sensitive/cyan-forming and green-sensitive/magenta-forming layers. It supplies density-versus-log-exposure and spectral dye-absorption curves, with stated exposure/process/viewing conditions and a warning that representative curves are not specifications for every roll. The OCR identifies the curves but does not substitute for measuring them.

The general density relation `T(lambda)=10^(-D(lambda))` motivates exploring selective absorption/density rather than only constant-luminance chroma expansion. Real dye spectra and layer responses are not ideal independent CMY coordinates or AP1 channels. This evidence does not establish one global saturation formula, a universal “more saturated means darker” rule, or a preferred strength curve.

Compare a separately specified print-density-inspired candidate against constant-Y and HSV-inspired saturation. Judge through existing ACES; do not silently add a complete print-emulation LUT whose tonal/rendering assumptions would confound the comparison. The user's aesthetic bias is toward print-film behavior, with the eye deciding preference and numerical checks guarding stability/artifacts.

## Mathematical distinctions

The equations below are our analysis of candidate families, **not claimed reconstructions of Lightroom or Resolve**. Let `r` be signed linear AP1 RGB, `1` the neutral vector, `p > 0` the pivot, and `a > 0` contrast strength (identity `a = 1`).

### Affine contrast in linear light

```text
r' = p*1 + a*(r - p*1)
f(x) = p + a*(x-p)
```

This is literal scene-linear scaling around a pivot, with no additional S-curve. It is finite for signed inputs, preserves the neutral axis, and fixes the grey `p`. It does not fix black: `f(0) = (1-a)*p`. Increasing contrast drives sufficiently dark values negative; decreasing contrast lifts black. With `p=0.18` and `a=1.2`, the neutral zero crossing is `0.03`.

No intermediate clamp is necessary, but the DRT cannot display distinctions among values it subsequently maps to black. Keeping negative EXR data is different from keeping visible shadow separation. This is a material choice, not a reason to silently add a pedestal or toe.

### Affine contrast in stops/log coordinates

For positive scalar `x`, scaling stops around the pivot gives:

```text
log2(x'/p) = a*log2(x/p)
x' = p*(x/p)^a
```

It expands/compresses exposure ratios, fixes the pivot and approaches zero at black. It is not affine in linear light and it still has no separately designed S-curve. A channelwise power operation changes channel ratios/hues; applying the scalar function to luminance and scaling RGB is a different operator.

Pure log is undefined at zero/negative values. ACEScct has an explicit linear toe supporting those values, documented in its [specification][acescct]. The clarified baseline comparison is `q'=q_p+a*(q-q_p)` on ACEScct channels, then conversion back to AP1. The toe means this is not globally pure-log or black-preserving: with a linear pivot of `0.18` (`q_p≈0.4135884`) and `a=1.2`, neutral black becomes approximately `-0.0064644` in AP1, with a neutral zero crossing around `0.0053870`. These are consequences to evaluate, not permission to add a clamp/pedestal. Any high-range decoding limit also needs reconciliation with Antitone's unclipped working domain before a probe.

### What contrast could preserve

Use AP1's actual XYZ Y row from `src/color.rs`, approximately:

```text
w = [0.2722287168, 0.6740817658, 0.0536895174]
Y = dot(w, r)
d = r - Y*1
```

For affine contrast, `Y' = p+a*(Y-p)` and `d' = a*d`. Thus it scales absolute RGB opponent distance by `a`, but relative colourfulness resembling `||d||/Y` changes by `a*Y/Y'` where that ratio is meaningful. It is not uniformly “more saturation” everywhere.

| Desired invariant | Candidate construction | Limitation |
| --- | --- | --- |
| Absolute linear opponent chroma | `r' = f(Y)*1 + d` | Relative saturation changes as luminance changes; chosen opponent basis is not perceptually uniform |
| RGB ratios/chromaticity | `r' = r * f(Y)/Y` | Preserves ratios for positive scaling, not perceptual saturation; singular at zero luminance and problematic across sign changes |
| Perceptual saturation | Transform to a declared appearance model, modify brightness/lightness and reconstruct at fixed model saturation | Viewing/reference-luminance assumptions, signed-domain handling and reconstruction/gamut behavior must be specified |

For true affine contrast, exact global ratio preservation is impossible at a nonzero RGB vector with `Y=0` when `f(0) != 0`: finite common scaling cannot produce the requested output luminance. Near-zero signed luminance can also imply arbitrarily large gains. A vague “preserve saturation” checkbox cannot settle this boundary.

The [ACEScct ASC CDL appendix][acescct] specifies an unclamped luma-axis saturation operation using `0.2126/0.7152/0.0722` on ACEScct **code values**. Those are not AP1 scene-linear luminance weights. This is a useful reminder that a CDL-style saturation label does not establish physical luminance preservation.

### Saturation candidates

1. **Constant-Y baseline:** `r' = Y*1 + s*(r-Y*1)`. Identity at `s=1`; `s=0` gives AP1 neutral at the original Y. Defined for signed/HDR RGB; exact Y preservation uses AP1 weights. It can create negative channels when increasing chroma, which must not be silently clipped. “Constant luminance” is not “constant perceived brightness.”
2. **HSV-inspired maximum-anchor/density candidate:** for positive RGB, holding HSV H/V and scaling S is equivalent to `r' = M*1 + s*(r-M*1)`, where `M=max(r)`. For `s>1`, `Y' = M+s*(Y-M)` decreases except on neutrals, while the maximum remains fixed. This explains density-like darkening without invoking a physical film model. This algebra can extend to signed/HDR values without dividing by M, but meaningful signed colour behavior, sector-boundary smoothness and any nonlinear S remapping still require evaluation. It is not a verified replica of Kelly's Resolve node.
3. **Adaptive boost/vibrance:** reduce added chroma for already-colourful inputs. This describes a weighting policy, not a colour model; it can be layered onto either of the first families. Choose the metric, neutrality behavior, response and any hue/skin protection explicitly.
4. **Perceptual saturation:** darktable UCS or a specified appearance-space direction may give more even visual increments. Complexity is justified only if it produces a clearly preferred result with robust signed/HDR and CPU/GPU behavior. A positive-only model is not directly interchangeable with Antitone's working domain.

The density candidate's `s=0` endpoint is grey at M, not at Y. Decide whether selectable modes should have distinct desaturation behavior or share an intentional monochrome endpoint. Dedicated B&W film handling remains a separate problem.

## Comparison design, not production acceptance

Use channelwise ACEScct contrast as the log-domain baseline. Specify and compare a colourfulness-preserving endpoint, clearly distinguishing any chromaticity proxy from perceptual preservation, and evaluate their **contrast colour-coupling blend**. Prefer a shared neutral tonal response/pivot; record any differences rather than silently normalizing each image.

For saturation, compare constant-Y, HSV-inspired maximum-anchor and a separately specified print-density-inspired candidate. Evaluate a **saturation-character blend** between constant-Y and each useful density endpoint. Adaptive weighting and at most one justified perceptual alternative remain separate candidates, not automatic additions.

Start with output interpolation in linear AP1 before ACES: `(1-t)*A(input)+t*B(input)`, using independent contrast/saturation blends at `t=0, 0.25, 0.5, 0.75, 1`. Both endpoints process the same input at the given stage; do not apply them sequentially. Parameter interpolation is a different experiment, not an equivalent implementation. The intermediate result need not retain either endpoint's colour invariant.

Check every blend at neutral amounts for exact identity, neutral-axis/pivot consistency, signed/HDR behavior and continuity. Evaluate contrast reduction/increase and saturation reduction/increase, then their interactions. Hold exposure/WB and ACES constant. Compare nominally equal amounts as well as explicitly labelled strength-matched results so a character difference is not just an amount difference. Density-like desaturation endpoints have different brightness; retain that distinction in the comparison.

Choose production blends only if both endpoints and intermediate values are useful. A single preferred model is a valid outcome; no production control/default or automatic asymmetric preservation is selected.

Use synthetic neutral ramps, hue/chroma sweeps, low-luminance colour, mixed-sign/zero-Y RGB, values above one, near-gamut primaries and smooth channel-maximum crossovers. Use read-only photographs with skin, foliage, skies, painted/dyed surfaces, saturated lights and coloured shadows. Include corrected and uncorrected source paths and both output targets. Photographs and rendered outputs stay ignored/local; retain authored methods and selected compact measurements if a probe is separately selected.

Evaluate hue stability, shadow separation, highlight colour, skin plausibility, colour noise, continuity, identity and the user's preference. Record pre-DRT luminance/chromaticity metrics separately from rendered waveform/vectorscope observations. Scopes measure the rendered output, not proof of preserved working saturation.

## Reference experiment checkpoint

Snapshot: **2026-10-12**, Apple M5 Pro/arm64, macOS 26.6.2, Rust/Cargo 1.99.0, pinned local LibRaw/OpenMP and OCIO 2.6.0. [Maintained source, exact equations and reproduction](tools/contrast-saturation/README.md). Production processing/UI/schema are unchanged.

The experiment compares:

- Channelwise ACEScct contrast with a **black-offset-ratio proxy**, sharing the entire neutral tone response and pivot. The proxy preserves ratios after subtracting the same inherent transformed black, not perceived saturation or unconditional output RGB ratios. Its divided-difference extension avoids a zero-Y singularity without a clamp/fallback. This is an explicit proxy, not Resolve HDR.
- Constant-Y versus maximum-anchor saturation and versus **relative-density power**, with declared signed continuations. Positive density power scales ideal relative optical density; it is not measured print-film dye behavior. Nominal and fixed-patch-chroma-matched amounts remain separately labelled.
- Five linear-AP1 endpoint blends in each direction and four combined contrast/saturation grades, before unchanged ACES 2 sRGB/P3.

The local gallery is `.cache/probes/contrast-saturation/run3/index.html` in the planning worktree. It contains **45 variants × four fixtures × two targets = 360 TIFF/PNG pairs**, all `360×240`. The synthetic fixture includes signed/HDR neutral and colour ramps, maximum crossovers and near-zero Y. Dog/ring/canopy photographs use current camera-native measured anchors/neutral fitting and toe 2.5, fixed photo exposure/WB, no correction and two stratified original pixels per output cell. Nonlinear processing precedes target-linear averaging, but this is not full-resolution settled reconstruction. The [recipe](tools/contrast-saturation/README.md#reproduction) and local manifest explicitly distinguish these illustrative diagnostic grades from recovered user settings.

### Measurements and initial observations

Selected **pre-DRT AP1** mean opponent-distance measurements from this snapshot (not perceived saturation):

| Fixture | Identity C | Contrast 0.8 RGB / proxy | Contrast 1.2 RGB / proxy |
| --- | ---: | ---: | ---: |
| Dog | 0.073838 | 0.050812 / 0.060877 | 0.107353 / 0.091491 |
| Ring | 0.050943 | 0.046439 / 0.051486 | 0.056467 / 0.052859 |
| Canopy | 0.195667 | 0.133143 / 0.153246 | 0.284107 / 0.253483 |

The proxy retains more colour on contrast reduction and less of the coupled boost on increase in these fixtures. It does **not** preserve absolute C globally. Its nonneutral tonal response differs slightly from RGB contrast: canopy mean AP1 Y at contrast 0.8 is `0.302402 / 0.303149`. Shared neutral strength does not mean identical colour-pixel luminance. Initial small-image inspection finds the expected foliage difference, but the user's preferred endpoint/blend remains unselected.

At saturation 1.3 on canopy, maximum-anchor saturation creates negative AP1 channels in **31,316/172,800** source samples, whereas density power retains the input's **zero** negative samples. Both pass unchanged ACES; negative channels are not automatically defects, and sign preservation alone is not evidence of a superior look. On the red ring, the power result appears more restrained at nominal 1.3; matching the mild reference's AP1 chroma uses power `1.424893` and reveals additional density/darkening. A nominally gentler result must not be mistaken for preferred character at equal strength. The corresponding decrease match is `0.646558` versus baseline 0.7.

Important candidate limits:

- Increased ACEScct contrast still makes neutral black negative: at `p=0.18, a=1.2`, `f(0)=-0.00646442474348078`. No correction is added.
- Power saturation preserves signs/max magnitude but changes hue ratios; `s<1` has an unbounded derivative at a zero channel, so low signed noise can be amplified even though the function is continuous. It has no selected full-desaturation endpoint and rejects amounts below 0.25 in this experiment.
- Neither endpoint blending nor unchanged ACES guarantees uniform visible saturation or hue. The strongest synthetic inputs are diagnostics, not a preferred look.

### Verification and remaining choice

Passed all nine dedicated scalar tests, targeted Clippy with warnings denied, formatting/diff checks and local documentation/gallery links. Built/ran the release probe and native PNG helper. **All 360 PNGs were decoded and checked against every TIFF RGB16 code, then given the exact TIFF ICC** by the standard-library verifier. ImageIO initially substituted named system profiles, so exact metadata retention is enforced separately rather than assumed. Every gallery image/reference exists and all 360 metric rows are finite. The completed run is `run3`; earlier partial attempts stopped at `sips`' sandbox-inaccessible per-user scratch path and are not result sets.

Inspected identity photographs, selected P3 contrast/saturation endpoints and the sRGB synthetic density ramp. This is initial diagnostic inspection, not user taste selection or physical colour-display acceptance. No production/GPU suite, full-resolution noise/texture comparison, skin/coloured-light photograph, corrected-source comparison, Lightroom/Resolve interoperability or broader-device acceptance was run. The existing library/native rendering code and pinned ACES assets are unchanged.

### Smaller photographic view

The user found the full matrix overwhelming and ring/canopy exposure unsuitable for judging character. The smaller view is better received; both contrast endpoints and constant-Y versus HSV-style saturation are useful. The agreed shortlist keeps print-density power in research only. Prefer simple creative mathematics with explicit correctness and honest semantics; extra perceptual machinery needs evidence of photographic benefit.

Use `.cache/probes/contrast-saturation/simple2/index.html` for a small exposure sheet, then `comparison.html` for one photograph and two independently selected options at a time. Both sides have a baseline option. Each shortlisted pair now has a single **Halfway (50/50)** option, in both adjustment directions. It interpolates the two endpoint outputs in linear AP1 before ACES at the same grade amount—not displayed images or half-strength grading. Quarter-point blends, combined adjustments, print-density power, synthetic diagnostics and sRGB stay in the full research matrix.

The exposure sheet brackets ring at `+2.5/+3.0/+3.5 EV` and canopy at `+1.5/+2.0/+2.5 EV`. The viewer retains ring `+3.0`, canopy `+2.0`, dog `+2.5 EV`. The user reports the smaller comparison is much better, but has not explicitly selected these exposure values. Each model receives the same per-photo exposure/WB before creative processing/ACES; no per-model brightness fitting is added.

The current smaller run contains 45 P3 TIFF/PNG pairs: 13 variants on three photographs plus six exposure images. Passed 11 binary tests (nine scalar plus shortlist/midpoint regressions), targeted Clippy, exact PNG/TIFF RGB16-code/ICC verification and an offline DOM-stub check of all 192 photo/adjustment/model-pair selections. All 33 retained comparison/exposure images match the preceding smaller set's codes/profiles exactly; 12 midpoint images are new. Earlier checks proved unchanged dog endpoints and old-exposure ring/canopy brackets match the full matrix, and bracket images match their comparison baselines. Selection logic is checked offline, not in a live browser (localhost serving was sandbox-blocked); monitor/taste and full-resolution acceptance remain open. [Reproduction and mode details](tools/contrast-saturation/README.md#reproduction).

**Next choice:** judge whether the halfway options add useful character beyond selecting an endpoint, before choosing controls/defaults. Regenerate if a different baseline exposure is desired. A focused higher-resolution/skin/noisy-shadow/coloured-light pass remains separately selectable. No production blend control, automatic preservation/default, production parameter range or compatibility change is approved by this checkpoint.

## Sources

- [Lightroom saturation/vibrance examples][lightroom-secondary] — secondary educator article, Elizabeth Halford.
- [Adobe tone-adjustment help][adobe-tone] — access blocked; follow-up only.
- [Resolve Color page][resolve-color] — vendor descriptions, not operator equations.
- [Resolve 18.5 Primaries, p. 2883][resolve-primaries] and [HDR palette, pp. 2900–2906][resolve-hdr] — Blackmagic reference manual via a third-party mirror, visually read; exact internal equations remain unknown.
- [Kodak 2383/3383 technical data][kodak-print] — primary manufacturer document via archived OCR; curves not measured.
- [Cullen Kelly on contrast ratio][kelly-contrast] — practitioner workflow.
- [Cullen Kelly on HSV saturation][kelly-saturation] — practitioner method and aesthetic preference.
- [darktable Color Balance RGB][darktable-balance] and [colour dimensions][darktable-dimensions] — technical application documentation; version caveat above.
- [LiftGammaGain subtractive-colour debate][lgg-subtractive] — practitioner disagreement.
- [ACESCentral output-rendering discussion][aces-rendering] and [colourful-light grading][aces-lights] — DRT behavior and aesthetic tradeoffs.
- [ACEScct specification][acescct] — normative encoding/CDL formulas.

[lightroom-secondary]: https://digital-photography-school.com/vibrance-vs-saturation/
[adobe-tone]: https://helpx.adobe.com/lightroom-classic/help/tone-control-adjustment.html
[resolve-color]: https://www.blackmagicdesign.com/products/davinciresolve/color
[resolve-primaries]: https://ltbits.github.io/davinci-resolve-manuals/DR18/DR18-RM10-Color.pdf#page=110
[resolve-hdr]: https://ltbits.github.io/davinci-resolve-manuals/DR18/DR18-RM10-Color.pdf#page=127
[kodak-print]: https://archive.org/download/kodak-vision-color-print-film-2383-3383-technical-information/KODAK-VISION-Color-Print-Film-2383-3383-technical-information_djvu.txt
[kelly-contrast]: https://blog.frame.io/2023/07/24/how-professional-colorists-look-at-image-contrast-ratio/
[kelly-saturation]: https://blog.frame.io/2023/04/03/cinematic-color-saturation-resolve-hsv/
[darktable-balance]: https://docs.darktable.org/usermanual/development/en/module-reference/processing-modules/color-balance-rgb/
[darktable-dimensions]: https://docs.darktable.org/usermanual/4.6/en/special-topics/color-management/color-dimensions/
[lgg-subtractive]: https://www.liftgammagain.com/forum/index.php?threads/the-right-way-to-emulate-subtractive-colour.18094/
[aces-rendering]: https://community.acescentral.com/t/aces-2-0-output-rendering/5770
[aces-lights]: https://community.acescentral.com/t/aces-2-0-grading-colorful-lights/5765
[acescct]: https://docs.acescentral.com/encodings/acescct/
