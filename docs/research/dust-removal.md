# Dust detection and repair: experimental evidence

This is exploratory evidence, not a selected product algorithm or an extension of the editor's processing contract. The maintained [comparison tools](../../scripts/dust-experiments/README.md) operate on private, read-only samples; photographs, masks, measurements and visual comparisons remain under ignored `.cache/`.

## Evidence and its limits

The real-mark comparison contains 14 visually labelled dust cases in native-resolution 512×512 crops from five scans: compact bright specks, a small cluster, an irregular flake, an elongated fiber-like mark and a curved fiber-like mark over wood, fur, foliage and broad tonal structure. Two additional crops label legitimate scene detail: decking nail heads and a bright building/pipe joint beside a cable. An additional bright fur feature remains unverified and is not counted as a dust label. The available samples contain convincing dust-like defects; survey contacts and sparse native crops are insufficient to assess their presence or prevalence.

Candidate discovery covers the complete metadata image areas of seven RAWs, approximately 348 million unique native pixels, in bounded overlapping camera-RGB tiles. Visual inspection includes ranked candidate contexts, 17 production-colour crops, six unseeded 512×512 windows and six additional structured scene-coordinate windows (five 1024×1024 and one 1024×1000 clipped at the metadata border). This is not exhaustive visual inspection of seven frames, much less the 491-file collection. Diagnostic density images explicitly map each patch's 1st/99th percentiles to grayscale for visibility; this mapping never feeds development or repair.

Most real dust cases were selected from Gaussian proposals. One small window fiber was identified in a scene-coordinate region chosen without proposal scores. Their visual annotations establish useful repair examples, not an independently sampled detector benchmark. Masks include conservative repair margins, not physically measured defect support. There are no clean recaptures.

Controlled reconstruction also covers 12 native-resolution 1024×1024 crops from eight scans: sky, wires/mast, cables, board seams, seating, mesh, foliage, flowers, water, fur and woven fabric. It uses 21 held-out masks plus three unverified blemishes. Held-out pixels supply an existing-source reconstruction reference, not a physically clean master. Two detector-generated masks exercise the identical repair functions. Manual versus detected origin is not an algorithm input.

Working crop sources use pinned production LibRaw, fixed reference WB and the scalar model-4 inversion with new-grade toe 2.5, shoulder off and identity Photo grading. One explicit diagnostic Dmin/Dmax pair is shared; it is not accepted roll calibration and does not establish cross-stock colour accuracy. Working AP1 remains signed/HDR f32 without repair clipping. Viewing uses the production ACES 2 sRGB renderer with explicit +2 EV inspection exposure, leaving working data unchanged. These uncorrected snapshots apply no flat-field gains. Comparisons do not use camera JPEGs or survey proxies.

Visual dust annotations are available, but independently sampled exhaustive defect labels, clean recaptures, defects crossing fine in-focus geometry, human faces, B&W material, learned-model comparisons, physical-monitor acceptance and production GPU/interactive performance evidence remain absent. These limits preclude a real-dust precision/recall claim or a production algorithm choice.

## Detection: local anomalies are not dust identity

The tested candidates are multiscale Gaussian residuals, median residuals, and bright/dark morphological top-hats. All analyze mean negative-log camera RGB with a local highpass-scale estimate. Their scores are not probabilities or Gaussian sigma values.

Simple residuals strongly respond to legitimate scene detail. In a one-megapixel cable crop, the common score-6 setting produces 3,388 Gaussian components versus 1,198 median components. Visual inspection shows real cables, seams and texture among them. A distant mast is one of the strongest compact Gaussian responses in a sky crop. High confidence in a local anomaly is not high confidence that it is dust.

Median residuals are a useful conservative baseline, not a solved detector. At score 10, the same cable crop still has 40 components, while a smooth-sky crop retains one tentative blemish. Higher thresholds can suppress weak defects along with real detail.

Common numerical thresholds are not equivalent operating points: morphological residuals have a different grain/background distribution. Controlled achromatic transmission-loss experiments therefore also use method-specific baseline score quantiles. At a 0.1% pre-component tail, median support coverage across eight toy masks is approximately 6% for 25% opacity and 59% for 75% opacity. At 0.5%, it is approximately 31% and 71%. These are support-coverage observations, **not real-dust recall**; existing scene structure can already intersect the support. Baseline coverage and accepted background areas are retained with the measurements.

Maximum-component rejection introduces another trap: increasing a threshold can split a previously rejected large component into accepted fragments. The resulting mask can be nonmonotonic. These baseline masks must not be adopted as product sensitivity semantics unchanged.

A limited physical-polarity test retains only increased negative-log camera density, consistent with reduced negative transmission. At Gaussian score 10 followed by four-pixel dilation, this excludes the entire annotated nail-head control, while still flagging approximately 57% of the bright building-joint control. Median coverage of the 11 manually painted dust repair supports is approximately 54%. This is annotation overlap in a Gaussian-selected set, **not dust recall**. Shared score thresholds remain incomparable across methods; pixels outside annotations include unlabelled dust and must not be counted as measured false positives. Polarity helps with dark scene features but does not distinguish dust from legitimate bright structures.

Shape/context tests cover 14 visual dust annotations, one ambiguous fur feature and the two scene controls. The tested shape rule requires component compactness >=0.35 and aspect <=4. It reduces some extended scene responses but leaves the bright joint control flagged. Adding a structure-tensor coherence veto in a 6..20-pixel annulus also leaves that control flagged, while completely rejecting four labelled dust supports at coherence limit 0.5 and two at 0.8, including both near-seam examples. An edge's presence is not evidence that a nearby mark is legitimate; blindly vetoing structured backgrounds loses exactly the defects that need geometric care. Long-fiber coverage remains unestablished, and aspect rejection must not become product policy.

The evidence supports keeping detection separate from repair and reviewing masks. It does not establish that an unreviewed automatic detector is usable. A more selective detector needs evidence beyond isolated local contrast: structure/context, real-film training labels, or an additional optical observation.

## Repair: grain, structure and pixelwise error disagree

Five candidates receive identical masks:

- **Harmonic interpolation:** a four-neighbour Laplace solve using only known boundary pixels.
- **Telea:** OpenCV fast-marching inpainting, per-channel float with explicit numerical conditioning.
- **Navier–Stokes:** OpenCV structure-propagating inpainting, using the same float conditioning.
- **Donor clone:** local masked-context patch matching followed by copying.
- **Donor heal:** the same donor, plus a harmonic correction of the destination-minus-donor boundary difference.

The donor search is deliberately simple: up to 128 pixels per direction, ten pixels of context, no scale/rotation search, and no source patch containing masked pixels. It is not PatchMatch or a reproduction of Lightroom's proprietary implementation.

Diffusive methods can reconstruct small holes and continue narrow simple structures, but flatten fine texture in wider masks. Donor healing preserves more fine-frequency energy while correcting the obvious tonal seams of cloning. However, donor matching can select the wrong local structure; the building/pipe example beside a cable is worse with the automatic donor than with local interpolation. Boundary blending cannot repair an incorrectly chosen donor.

Illustrative medians across the 21 held-out cases:

| Method | Masked highpass-energy ratio to source | Repair CPU time |
| --- | ---: | ---: |
| Harmonic | 0.15 | 6.8 ms |
| Conditioned Telea | 0.52 | 9.6 ms |
| Navier–Stokes | 0.36 | 9.1 ms |
| Donor clone | 1.43 | 16.8 ms |
| Donor heal | 1.07 | 17.3 ms |

Highpass energy includes scene texture and edges, not just grain. A ratio near one does not establish correct grain spectrum, RGB correlation, geometry or photographic meaning. Pointwise RMSE often favours harmonic averaging because it avoids guessing unpredictable source grain; that does not make its smooth fill visually preferable.

Times cover the Python/CPU repair operation in bounded native-pixel contexts on one Apple Silicon machine. They exclude source development, rendering, I/O and interactive scheduling, and are not resource policy or a GPU performance claim.

The real-mark comparisons show that these classical repair engines can remove the supplied specks and fiber-like marks. For the larger loop/cross-shaped fiber cluster, interpolation produces a conspicuously smoother patch; cloning can mismatch the surrounding tone, while donor healing retains texture with a less obvious tonal transition. For small specks the differences are subtler. A neighbouring unmasked mark remains unchanged by design: a successful individual repair is not a dust-free crop. These observations do not establish correct reconstruction beneath defects or a universal method ranking.

The baseline comparisons, 472 current donor-refinement outputs and 77 repair-support comparisons assert finite output and bit-exact working-data preservation outside the supplied mask. Masked input values are discarded before computation, preventing held-out reference leakage. This is implementation consistency, not photographic acceptance.

## Repair coordinates and donor selection

The stage study covers eight cases: four visual dust marks, one ambiguous fur feature, and three held-out wood-seam, near-cable building/pipe and fur regions. Five baseline methods and fixed-donor variants operate before inversion in normalized native-camera RGB and after the full pipeline in AP1. A negative-log-camera healing variant uses the same donor/healing solver, then exponentiates only the repaired pixels before the unchanged scalar pipeline. It is a multiplicative transmission correction, not a new film inversion, pedestal or clamp. It rejects nonpositive known transmission rather than silently flooring it.

Fixed integer donor translations are identical across representations and were chosen from masked-context inspection, not optimized against hidden truth. Automatic donors are selected separately and can change with representation. These manual selections do not consistently outperform the simple automatic matcher: a translation can preserve some nearby alignment while still having incompatible texture or structure. Fixed-donor tests isolate blending-coordinate effects; automatic-stage differences also include matching changes.

A donor-adjustment repeat uses the held-out seam's visible approximately -0.1 slope. The nearby translation [20,-2] yields AP1-heal RMSE 0.01052 versus 0.01364 for [100,-12], and a highpass-energy ratio of approximately 1.00 versus 1.07. Automatic AP1 selection still has lower RMSE, 0.00968. Source adjustment materially improves this particular repair but does not make manual selection an oracle; neither low error nor matched highpass energy proves grain fidelity.

Camera float normalization introduces a small baseline difference from exact RGB16/f64 development: maximum selected-pixel AP1 difference is approximately 1.19e-7 in these cases. The probe develops only selected pixels and copies original AP1 bytes elsewhere. As a pointwise-processing control, fixed cloning before/after the pipeline agrees within 1.49e-8, so the larger healing differences are not a donor-position or rendering mismatch.

Masked AP1 RMSE with the **same fixed donor**:

| Held-out case | Additive camera heal | Log-camera heal | AP1 heal |
| --- | ---: | ---: | ---: |
| Building/pipe beside cable | 1.58027 | 0.01841 | 0.01385 |
| Siding seam | 0.01329 | 0.01333 | 0.01364 |
| Fur | 0.00117 | 0.00123 | 0.00131 |

In the first case, additive camera healing pushes a blue-channel value down to approximately 0.000136, still above the production floor of 1/65535. Nonlinear inversion amplifies the implausible repaired transmission into an extreme bright/blue artifact. No production floor event or nonfinite output occurs. Log-camera blending avoids this gross failure without clipping, but the incompatible donor still limits reconstruction. This does not establish that all pre-inversion repair is bad, or that log-camera/AP1 healing has a universal ranking. It shows that transplanting an additive working-space healer into transmission coordinates is not a neutral stage choice.

On real small flecks, log-camera and AP1 repairs can both remove the marked blemish with subtle differences; the larger fiber cluster continues to expose smoothing versus donor-texture tradeoffs. No clean recapture establishes the true hidden structure, and the study does not yet measure grain spectra or cross-channel correlation.

## Donor refinements: useful local improvements, not a dependable default

The refinement study keeps the original healer as a reference and separates source eligibility, matching and boundary correction. Eight structured cases compare AP1 and log-camera repair, including fixed translations. A further 18 held-out cases and ten real marks extend AP1 inspection. Together they cover the existing 21 reconstruction references, all 14 visual dust labels and the ambiguous fur mark; they are not a newly independent benchmark. No source is selected using hidden reference pixels.

The original whole-rectangle exclusion rejects some nearby donors even when their copied pixels and healing boundary are clean. The local variant instead requires clean source pixels under the repair core and a two-pixel exterior, and compares only context pairs known at both ends. Missing context is excluded rather than treated as black. Nearby mask components are grouped so that one component's hidden pixels cannot become another's correction boundary. The structure variant adds smoothed RGB-gradient correspondence to colour matching. Its known-pixel validity includes the smoothing/derivative footprint; gradient features must not incorporate zero-filled defects. A ranked variant weights context toward the defect boundary, uses a 3:1 colour/gradient mixture, and prefers the nearest source among candidates within 1% of the best matching score. This is a local-texture prior, not calibrated confidence. Its constants were explored on these same held-out cases, so their results are development evidence rather than independent validation. Unmarked source defects remain possible.

Boundary comparisons retain identical structural donors. An exact biharmonic correction can follow curved tonal differences better than the harmonic model, but also spread noisy boundary differences through the fill. In fur it creates a visible tonal blotch. The limited-boundary variant starts with harmonic correction and performs a few stable biharmonic steps rather than converging the whole interior. It reduces that regression without establishing a universal best correction. All maintained variants retain the same binary output support. A direct outward feather expansion was rejected: even a two-pixel transition generally increased reconstruction error and made useful nearby sources ineligible when their expanded footprint reached the unknown core. This does not rule out an explicitly authored core/transition mask, but automatic expansion is not retained. Selection-aware boundary exclusions are not implemented.

Across the 21 AP1 held-out cases:

| Variant | Lower RMSE than baseline | Cases over 10% worse | Median RMSE / baseline |
| --- | ---: | ---: | ---: |
| Local eligibility and paired RGB | 9 | 2 | 1.000 |
| Structure matching, harmonic | 11 | 1 | 0.995 |
| Structure matching, exact biharmonic | 10 | 5 | 1.024 |
| Structure matching, limited boundary refinement | 15 | 1 | 0.969 |
| Boundary-weighted local ranking, limited refinement | **19** | **1** | **0.937** |

The boundary-ranked variant improves the near-cable building/pipe reference from RMSE 0.01484 to 0.00987 and the sky-wire reconstruction from 0.00590 to 0.00539. It recovers the small water reference from the unranked refinement's regression to 0.00192 versus the 0.00197 baseline. The seam reference remains slightly worse, 0.00992 versus 0.00968, and the mesh scratch is approximately 17% worse. Gradient matching and a local-distance prior therefore reduce failures without solving structural selection. Pixel error, smoothed-gradient error and highpass RGB covariance are complementary diagnostics: they still include scene texture, do not establish grain fidelity and cannot rank real-dust reconstruction without clean truth.

AP1 and log-camera remain distinct experimental choices. In the structured subset, donor positions can differ by representation. Ranked log-camera matching removes the unranked fur/seam regressions in the three held-out stage cases, but does not outperform its baseline in every case. Zero production-floor events do not establish photographic validity. The refinement outputs reproduce bit-exactly, including native selected-only development and donor/floor metadata; the original stage baseline also remains bit-exact. This verifies implementation consistency, not competitive acceptance.

After freezing the ranked matcher, a separate deterministic generator selected one circle and one line per native crop from SHA-256 coordinates without reading pixel values. This adds 24 held-out cases across the 12 existing crops. It is independent of matcher output and avoids visual target selection, although it reuses the same images and does not guarantee difficult geometry.

| Frozen variant on 24 new cases | Lower RMSE than baseline | Cases over 10% worse | Median RMSE / baseline | Worst RMSE / baseline |
| --- | ---: | ---: | ---: | ---: |
| Structure matching, harmonic | 15 | 2 | 0.963 | 1.295 |
| Structure matching, exact biharmonic | 9 | 11 | 1.085 | 1.378 |
| Structure matching, limited refinement | **18** | 2 | **0.957** | 1.291 |
| Boundary-ranked local matching, limited refinement | 17 | 2 | 0.976 | **1.235** |

The frozen check supports a modest benefit from local eligibility, structural matching and limited boundary refinement. It does not reproduce the ranked variant's tuned 6.3% median gain, and it leaves a tradeoff between average improvement and worst observed regression. Exact biharmonic correction regresses strongly again. Pixel-independent coordinates make this a better development check, not a photographic benchmark: the targets are existing source pixels, share the same 12 scans and are not verified dust crossing known structures.

## Defect core versus repair support

The visual annotations were deliberately conservative and are often much larger than the visible marks. That can make healing replace clean grain or nearby fine structure unnecessarily. A support study therefore keeps the frozen boundary-ranked healer unchanged and derives a high-confidence core from positive-density Gaussian score pixels inside each annotation. Components below three pixels reject; a fixed elliptical growth creates repair support, clipped to the editable annotation. The binary support then enters the same repair function as every manual or detected mask. The healer receives neither scores nor annotation provenance.

The study covers all 14 visual dust labels plus the ambiguous fur mark. At score 10 plus four-pixel growth, median support is 53% of the conservative annotation, ranging from 28% to 71%. A two-pixel growth retains only 31% at the median and is visibly more vulnerable to narrow support and abrupt donor changes. Score 10/+4 remains an experimental segmentation setting: scores are not probabilities, low-contrast halos can lie below threshold and there are no clean recaptures.

For the flagged siding fleck, the old ellipse contains 633 pixels while the visible high-contrast mark occupies a much smaller area. Score 10/+4 produces 213 pixels and selects donor offset `[-21,3]`; an independently drawn 10×6 ellipse contains 217 pixels and selects `[-23,3]`. Both preserve the nearby fine line better than replacing the full annotation, and the automatic support closely matches the explicit tight support without encoding that ellipse. This is useful evidence that the flakiness was primarily excess repair support, not a need to retune donor ranking.

This result does not redefine annotations as physical truth or establish a product detector. Core extraction used the annotation as a maximum ROI; a production proposal must provide equivalent component isolation, and an edited mask must remain authoritative. Too-small support can leave a defect halo, while too-large support needlessly guesses texture. The next mask evaluation needs clean or synthetically controlled defects with independently fixed support criteria.

## Frozen candidate on previously unused scans

A first independent repair round froze ten scan names before camera development or visual inspection. The selection removed all 24 scans referenced by prior dust metadata, divided the remaining 467 of 491 sample names into ten ordered strata, and selected one per stratum by namespaced SHA-256 of the filename. Complete native metadata areas were searched for proposals, covering approximately 498 million pixels. Three selected scans supplied no frozen case; ten proposal-assisted dust-like marks from the other seven were annotated and hashed before any repair existed. Empty scans and the weak case were not replaced after inspection.

The sole candidate was the previously frozen AP1 boundary-ranked healer with a positive Gaussian score-10 core and four-pixel growth clipped to each annotation. It accepted nine cases and rejected one weak mark whose maximum score was 5.18. First native-pixel inspection rates eight accepted repairs as visually clean defaults. These include the largest new fiber-like mark on diagonal wood, a compact mark near a horizontal seam, an irregular mark near intersecting wood and an oblique flake beside a thin curved line. One dark fleck is reduced but leaves a smaller bright blue residual or donor artifact. No masks, donors or constants were changed in response. All accepted repairs preserve AP1 f32 bits outside support.

This is genuinely new-scan evidence for default behavior, but not statistical or physical acceptance. The labels remain anomaly-proposal-assisted rather than exhaustive detector labels; visual appearance does not physically verify dust, there are no clean recaptures, and no selected mark crosses a fine in-focus wire. The common diagnostic calibration is still unaccepted; two first-roll crops report one and six inversion-floor pixels respectively, although the healer introduces no clamp. The observed result is therefore eight visually clean defaults, one visible residual/artifact and one automatic-support rejection—not a general 80% success-rate claim.

## Lightroom pilot: competitive sanity check only

A pilot package supplied matched RGB16 sRGB inputs from Antitone's production TIFF renderer/encoder, frozen Antitone references, support guides and a correction-effort template for manual non-generative Lightroom Heal. The TIFF pairs share the same unhealed Antitone AP1 crop snapshot and encoded pixels remain exact outside repair support.

The returned Lightroom Cloud results are original-size 8-bit Display P3 JPEGs rather than the requested RGB16 sRGB TIFFs. The comparison converts them through their embedded profile and round-trips both Antitone input and repaired reference through JPEG using each result's quantization tables and 4:4:4 sampling. On visual inspection at native pixels, both methods remove the intended mark without a clear consistent winner in most cases. The operator prefers Lightroom for the siding fleck; on the largest loop-and-cross cluster, Antitone removes a cross-shaped residual left in the Lightroom result but retains a diffuse patch. JPEG-matched controls account for most global code and high-frequency differences, so these outputs cannot establish exact preservation or grain fidelity.

The operator reports accepting Lightroom's automatic result in all eight cases without source moves, mask edits or retries. That demonstrates zero correction adjustments in this small pilot, not elapsed effort or a general default-success rate. The cases had already influenced Antitone development and have no clean truth. The pilot supplies competitive context; further Lightroom comparison is out of scope. Current evidence still does not justify a production algorithm choice. An authored core/transition-mask experiment remains possible, but automatic feather expansion is not supported by the current evidence.

## Provisional reversible workflow foundation

The Rust implementation of the frozen `ap1-boundary-ranked-score10-grow4-v1` healer supports the transient editor prototype; it is not a production algorithm selection. It evaluates manual or detected binary support with automatic or explicit integer donor placement and changes only support pixels. Support masks for separate repairs may not overlap. Each patch is evaluated independently from immutable unrepaired signed/HDR AP1; another repair's evaluated output is never donor or boundary input.

Evaluation reads only a bounded source window around the repair and donor search. Its automatic window includes the frozen 128-pixel search range and the feature footprint needed at that range. Manual evaluation includes the explicit source and destination windows. A four-million-pixel limit rejects pathological transient operations. A supplied mask is authoritative: the core has no detector score or automatic-support threshold, so a manually authored or edited mask follows exactly the same healer without passing score-10 acceptance.

The port was compared with retained Python evidence on 60 real, structured and independently generated held-out cases, plus the nine accepted repairs from previously unused scans. All 69 selected the same donor rectangles and preserved every outside-support AP1 f32 bit. Sixty-six were bit-identical inside support; in each of the remaining three, exactly one channel value differed by one f32 ULP. Bounded evaluation produced the same reproduction pattern as full-crop evaluation. This establishes numerical continuity with the frozen research implementation, not photographic correctness or production acceptance.

The port preserves the frozen weighted score rather than using OpenCV's approximate correlation shortlist. Release profiling identifies automatic matching as the main release-to-result cost. Search reuses invariant context weights and required-pixel offsets, proves disjoint donors clean from support bounds, and abandons partial scores only when their conservative lower bound already exceeds the competitive margin. Once an exact zero fixes the global best, even a perfect candidate must improve the incumbent's full distance/score/coordinate rank to merit evaluation. These are exact-work eliminations, not a smaller search, changed score or quality setting. Exhaustive-reference tests and retained-case comparisons preserve donor metadata and repaired f32 bits. Evaluation remains asynchronous; content-dependent speedups and controlled GPU/component timings are not end-to-end native-input or scanout acceptance. Explicit donor reevaluation avoids automatic search.

The [editor workflow](../editor.md#provisional-dust-cleanup) owns gestures, transient repair state and working-view integration. It evaluates ungraded AP1 after correction, inversion and camera conversion, before photographic Roll/Photo controls. The full-grade sources used by the experiments above are a different input domain: healing and nonlinear grading do not commute, and donor matching can change. Those experiments therefore do not establish appearance under every downstream grade.

Repairs enter original neutral samples before view filtering and grading; subtracting a correction from an already rounded proxy cannot recover detail lost beneath an extreme HDR value. Viewer/scopes share the repaired result, while provisional camera proxies, thumbnails and TIFF/EXR remain unrepaired. [Remaining acceptance](../plans/dust-repair.md) includes pre-grade photographic correspondence and complete interaction/resource evidence.

## Large-mask acceleration

Automatic matching scales with the known context inside the support's bounding rectangle, not simply its marked-pixel count. A release-mode phase probe on one retained photographic crop took about 23.7 s for a thin diagonal spanning 384 pixels in both axes: 23.4 s searching, 0.25 s computing distance weights and 0.005 s filling. Its large mostly-known context explains why a small selected area can remain expensive after exact pruning. Accelerating only the fill misses this bottleneck.

A wider supported diagonal exposed a different regime: with a 128-pixel brush and roughly 59,000 marked pixels, GPU-assisted evaluation still took about six seconds. Almost three seconds were all-pairs distance weighting, with another two seconds of exact CPU verification surviving loose screening. A separable integer squared-distance transform now produces the same minimum-distance f64 bits before the unchanged weight calculation, without the quadratic scan; the scalar path remains beyond its exact-integer range. In that case distance work fell to about 1.5 ms. Screening also uses exact sums of dyadic weight ceilings as conservative paired-denominator bounds when source context is substantially missing, rather than always dividing by full target totals. This tightened bound changes neither final scores nor coverage requirements. Its extra coverage pass is avoided when the measured seed pairing does not justify it. The same wide case then evaluated in about 1.4–1.8 s across repeated runs with identical output and metadata.

A diagnostic trace from the user's real workflow measured six successful commits reaching repaired-view selection in 0.35–2.15 s (median 1.11 s). Worker queueing was under 1 ms, original AP1 preparation 17–37 ms, and reconstruction/acquisition 13–28 ms; remaining cost was principally matching. The user reported substantially improved responsiveness. These observations do not establish physical scanout, broader-device latency or aggregate resource acceptance.

GPU candidate scoring and correlation-based matching are relevant directions; accelerating only the fill would miss the observed bottleneck. Neither a plain f32 GPU winner nor an FFT/OpenCV shortlist preserves the frozen f64 score arithmetic and one-percent/epsilon membership automatically. The current Metal/WGSL path cannot simply request wgpu's Vulkan-only `SHADER_F64`. A quality-preserving hybrid needs conservative numerical bounds for every excluded candidate, retaining uncertain candidates for exact CPU comparison and preserving final donor metadata. An empirically successful top-N shortlist is not that guarantee.

The Metal screening path floors normalized screening features and weights into integer bins, accumulates conservative positive-error bounds with paired-u32 integer arithmetic, then verifies the bounds against the competitive ceiling on CPU. Invalid/out-of-range screening features contribute nothing; no photographic sample or repaired value is clipped or quantized. GPU f32 arithmetic only hints when to stop accumulating a prefix, so even an inaccurate hint cannot strengthen its integer bound. Exact CPU seed scores establish an upper bound, not a shortlist; their candidates must also enter ranking state, especially when a seed is exactly zero. Final scoring, one-percent/epsilon membership, tie breaks, metadata and filling remain CPU-owned.

Paired Apple M5 Pro evaluation of the integrated core, including its bounded submissions and job-specific waits, gave these approximate costs against the already-optimized CPU search:

| Support | CPU | GPU-filtered |
| --- | ---: | ---: |
| 128-pixel circle | 0.60 s | 0.27 s |
| 768-pixel line, 16-pixel brush | 1.77 s | 0.25 s |
| 384-by-384 diagonal, 8-pixel brush | 22.85 s | 1.46 s |

All 69 automatic and eight explicit-donor retained comparisons, 15 controls and the larger masks preserved donor metadata and repaired bits. Additional checks covered one million scalar bound terms, all 5,445 returned candidate prefixes under deliberately varied float stopping hints, coverage thresholds, multiword carries and a reproduced/fixed seeded-zero ranking bug. These tests support the conservative argument; they are not universal numerical or photographic proof.

The editor uses the same source-owning backend and shared submission gate. Screening has an explicit transient working-set cap, conservative device-limit checks and a per-slice point-work bound; observed long slices shrink later batches. Each slice waits only for its own submission, outside the gate. Tiny contexts, exact-zero seeding and declined acceleration admission keep exact CPU work; genuine GPU errors are not hidden by retrying a different renderer. Per-request/source retirement cancels obsolete work without memoizing failure. A real authored-RAW backend regression interleaves an offscreen presentation submission between heal slices, retires the source, checks acknowledgment/admission and completes subsequent work bit-exactly.

The timings include core preparation, GPU upload/dispatch/readback and CPU verification from already-developed AP1, not camera development or release-to-display timing. Tiny/flat controls do not justify unconditional GPU dispatch, and some retained cases still gain little or regress. Packed features/results add transient memory; no retained full-resolution float photograph or larger source-processing budget is introduced. Actual shared-UI latency, aggregate/in-flight memory, cold initialization and broader devices remain acceptance work, not consequences of passing numerical checks.

## OpenCV float Telea requires particular caution

In pinned OpenCV 4.10.0, applying float Telea directly to a constant normalized field of 0.02 with a small circular mask produces errors greater than one. The comparison includes an executable test demonstrating this scale-dependent limitation.

The experimental baseline maps known float values into a code-sized numerical range, calls inpainting without clipping or RGB8 quantization, and maps back. This reduces the gross scale problem but does not establish an exact, scale-invariant Telea implementation. It must not be promoted unchanged into the signed/HDR pipeline or described as a weakened production numerical gate.

## Useful literature and implementations

- [Telea, 2004](https://doi.org/10.1080/10867651.2004.10487596): fast-marching local inpainting. [OpenCV API](https://docs.opencv.org/4.x/d7/d8b/group__photo__inpaint.html) supports float/16-bit single-channel input but only 8-bit three-channel input.
- [Georgiev, Photoshop Healing Brush](https://download.gimp.org/references/Photoshop_Healing_Brush_a_Tool_for_Seamless_Clonin.pdf): published photographic donor healing with biharmonic boundary refinement, soft compositing and selection-aware boundaries. It documents historical Photoshop, not current Lightroom internals; the experiment does not reproduce the complete brush.
- [Criminisi, Pérez and Toyama](https://www.microsoft.com/en-us/research/publication/region-filling-and-object-removal-by-exemplar-based-image-inpainting/): structure-prioritized exemplar filling.
- [Barnes et al., 2009](https://gfx.cs.princeton.edu/pubs/Barnes_2009_PAR/): PatchMatch is a patch-correspondence accelerator, not a complete repair algorithm. The authors' original reference implementation is noncommercial research-only.
- [darktable retouch](https://docs.darktable.org/usermanual/development/en/module-reference/processing-modules/retouch/) and [healing source](https://github.com/darktable-org/darktable/blob/master/src/common/heal.c): source-patch healing and wavelet-scale retouch in a photographic workflow. GPL-3.0-or-later; its current OpenCL healing wrapper delegates the solve to CPU.
- [Lewis, Fast Normalized Cross-Correlation](https://scribblethink.org/Work/nvisionInterface/nip.html): FFT correlation and summed-area normalization address large-template matching cost; the paper also explains why statistical early rejection can miss narrow extrema. This is inspiration, not the frozen weighted RGB/Sobel score or a floating-point equivalence guarantee. [OpenCV template-matching formulas](https://docs.opencv.org/4.x/df/dfb/group__imgproc__object.html) document masked squared difference/correlation and f32 result storage.
- [Felzenszwalb and Huttenlocher, Distance Transforms of Sampled Functions](https://cs.brown.edu/people/pfelzens/dt/): separable lower envelopes replace all-pairs Euclidean distance work. The healer uses integer squared distances/intersections within an exact binary64 range, then the same square root; this is not an approximate distance field.
- [wgpu `SHADER_F64`](https://docs.rs/wgpu/30.0.1/wgpu/struct.Features.html#associatedconstant.SHADER_F64): native SPIR-V/Vulkan support, not a double-precision switch for the current Metal/WGSL path.
- [Kämper, Chizhov and Weickert, 2024](https://arxiv.org/abs/2401.06744): GPU domain decomposition plus multigrid for fast homogeneous-diffusion inpainting. Useful solver architecture, but not evidence of equivalent bounded biharmonic correction or faster automatic donor matching.
- [G'MIC inpaint](https://gmic.eu/reference/inpaint.html) and [transport-diffusion](https://gmic.eu/reference/inpaint_pde.html): useful classical comparison implementations, with CeCILL-family licensing.
- [Wan et al., 2020](https://arxiv.org/abs/2004.09484): separate scratch detection and learned old-photo restoration. Its [implementation](https://github.com/microsoft/Bringing-Old-Photos-Back-to-Life) does not release the labelled scratch dataset; old-photo results do not establish film-negative detection.
- [LaMa, WACV 2022](https://arxiv.org/abs/2109.07161): broad-context learned large-mask completion. Resolution robustness and natural-image completion are not grain-preserving signed/HDR repair evidence.
- [Rüfenacht et al., ICIP 2013](https://doi.org/10.1109/ICIP.2013.6738432) and [Trumpy and Gschwind, 2014](https://journal.smpte.org/periodicals/SMPTE%20Motion%20Imaging%20Journal/123/3/19/): polarized dark-field and changed-illumination captures make surface defects optically observable, including on silver-based material. This is a different evidence regime from RGB-only inference.
- [VueScan IR documentation](https://www.hamrick.com/vuescan/html/vuesc31.htm): scanner infrared cleaning requires an available IR channel and appropriate material; ordinary silver-image B&W is not equivalent to dye-based film.

## Open questions

The next detector evaluation needs an independently selected, more completely annotated set, including weaker defects and dust crossing fine in-focus geometry, with clean-detail counterexamples. Gaussian-discovered examples alone cannot establish completeness, and more threshold tuning on unlabelled crops cannot establish accuracy. Physical polarity, component shape and surrounding edge continuity are useful classical hypotheses to test before assuming learned inference is necessary.

Repair evaluation still needs clean truth, grain-spectrum assessment and verified real dust crossing fine wires or repeating structures. Highpass RGB covariance includes scene structure and does not establish isolated grain correlation or fidelity. Near-seam marks and held-out geometry do not close that real-defect gap. The frozen AP1 candidate is credible enough for a reversible workflow prototype, not accepted as the production stage or donor policy. Collect support rejections, residuals and donor failures through normal prototype use rather than retuning the current corpus. Manual and detected masks must continue to use the same reconstruction path.
