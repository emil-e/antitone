# Shared scan-region consumers

## Remaining scope

Analyze's first sampling-area review is accepted for current use. The [Analyze guide](../roll-analysis.md#1-choose-what-to-sample) owns the implemented roll-pooled conservative holder support, film exclusions, independent picture evidence and editable photo exceptions. This plan owns remaining consumer design and physical acceptance—not another implementation of that review.

Flat-field correction and auto-cropping need related source-space evidence, but different acceptance. [Auto-crop](../geometry.md#automatic-framing-consumes-measurements-not-sampling-support) uses complete strong retained picture measurements through explicit Photo/Roll/selection actions. Independent evidence preparation and broader framing acceptance remain unfinished:

- **Holder opening:** correction needs usable bare-light support inside the illuminated aperture, excluding the holder. A conservative negative-scan sampling interior is not proof that a reference's physical opening matches it.
- **Film area:** Analyze may use film margins outside the picture. Detecting film does not establish that it is undamaged or a known calibration reference; [dense-tail follow-up](roll-assistance.md#deferred-sampling-and-endpoint-work) remains separate.
- **Picture area:** statistical support need not be an accurate crop. Auto-crop requires independent complete measured edges and includes their angle, not an inscribed sampling interior. The shared tool previews a fixed successful set until Apply installs crop/angle atomically; Cancel and uncertain sources retain existing framing. Broader photographic acceptance must measure wanted content lost and unwanted rebate/holder admitted separately.

These boundaries need not coincide or all be present. Bare-light references contain no film; sheet film and flatbed-style scans may have no visible holder or perforations; already trimmed scans may censor physical boundaries. Film placement can change inside a stationary holder. Do not assume a complete holder → film → picture hierarchy or one rectangle for every source and purpose.

## Consumer boundaries

Share source-space measurements, provenance and ambiguity below feature controllers where the evidence supports reuse. Keep geometric inference separate from capture/sample eligibility and source-format compatibility. Flatbed/other ingestion and its colour/response interpretation are separate work, not delivered by this detector.

Feature owners retain editing, acceptance, persistence, transactions and numerical validation. Opening evidence must not automatically install correction, change a crop or rerun Analyze. Explicit user geometry remains authoritative. Distinguish absent elements from unobserved or ambiguous boundaries; missing holder edges alone must not invalidate an otherwise usable scan, and failed detection must not certify the whole raster as film or picture.

Current [flat-field estimation](../flat-field.md) requires rectangular support containing the image-centre patch. Reuse must respect that constraint, not silently relax it. Representation and acceptance of tilted, nonrectangular, partially hidden or ambiguous reference support remain open. Compatible native development does not prove physical reference/negative compatibility. Avoid a general mask framework until demonstrated consumer cases require it.

## Evidence and acceptance

- Compare bare-light references and negatives with independently marked boundaries where observable. Include sheet film, flatbed-style and full-bleed images, missing holder/rebate/perforations, partial openings, dense legitimate scenes, faint content, uneven illumination, shifted/tilted film and damaged edges. Private sources and overlays remain ignored and read-only.
- Evaluate missed wanted content and admitted unwanted material separately for opening, film and picture. Test absent/unobserved regions and roll reordering. The largest statistical interior is not automatically the correct physical boundary or preferred crop; define consumer tolerances from observed cases before claiming quality.
- Verify nonzero active-area origins and independence from user crop/turn/straightening. Exercise correction and Analyze separately. Numerical clipping/floor checks, signed/HDR processing, original source preservation and the complete-roll contract remain unchanged.

## Detector accuracy and latency

Improve holder and picture measurements without conflating conservative sampling support with physical boundaries. NegPy and Kader are idea-level precedents for threshold stability, local robust edge fitting and bounded filtered inference; implement and evaluate our own algorithms. Their behavior and reported confidence are not an accuracy or speed oracle for our RAW inputs.

Use the bounded [production preparation diagnostic](../diagnostics.md#bounded-area-preparation-profiling) to identify costs and verify exact-output optimizations. It distinguishes application-source warmth from uncontrolled filesystem caching. Remeasure after relevant changes; latency measurements do not establish photographic acceptance.

[Bounded request-local inference scratch](../architecture.md#editor-workspace) avoids a second development for admitted rasters; larger rolls reacquire measurements beyond the cap. Revisit measurement compactness/lifetime or repeated search work only for demonstrated benefit, not automatically to increase capacity. Preserve conservative holder support, frame provenance, numerical gates, source freshness, cancellation and authored edits. Exact-equivalence optimizations must retain exact outputs; changed search algorithms need independent accuracy evaluation.

Remaining stages:

1. **Refine locally and evaluate inference sampling.** Use plausible coarse frame candidates to request narrow higher-resolution edge measurements, with robust position/angle fitting, support/residual reporting and explicit competing edges. Compare filtered inference previews with current point samples for narrow rebates, fuzz, perforations and shifted sampling phase. Inference filtering must not replace original-pixel Analyze samples, eligibility checks or export reconstruction. Measure accuracy and acquisition/memory cost together; neither a larger raster nor a more precise fit proves material identity.
2. **Add complementary proposals only for observed failures.** Evaluate threshold-stable and polarity-aware proposals against the baseline and local refinement, using independent per-side transition and material evidence. Roll agreement may guide searches but must not silently turn inferred/missing picture sides into measured crop edges. Prefer the simplest candidate that fixes demonstrated cases; do not adopt a large always-on detector ensemble without benefit and latency evidence.

For every behavior-changing stage, evaluate wanted content lost and unwanted holder/rebate admitted separately for opening and picture, plus abstention/unsafe-proposal rates, edge/angle error where independently marked, complete latency distributions and sampled memory. Include scene-confounded, faint/dense, partial/censored, shifted/tilted and no-visible-holder cases. Keep held-out setups and all photos in their complete rolls. Check reordered rolls, source-coordinate mapping, authored exceptions, stale/cancelled work and explicit crop application. Define physical tolerances from reviewed photographic cases rather than improving application counts or weakening existing gates.

## Next actions

- Separate raw source-owned measurement metadata from accepted sampling choices when adding lazy requests. Analyze, cropping and future consumers should be able to request compatible evidence without owning it, silently approving support or requiring successful numerical fitting. Define development/file freshness, roll-context identity, cancellation, transient/persisted lifetime and authored-edit behavior before changing that boundary. Auto-crop consumes only already-retained accepted evidence.
- Extend independently marked crop checks across supported photographic cases and native inputs. Initial bounded-raster diagnostics and authored UI/geometry checks do not establish physical border accuracy, monitor appearance or broad camera/setup coverage. Keep ambiguous/censored cases explicit rather than optimizing for successful application counts.
- Scope bare-light/reference support using matching references/negatives and declared physical compatibility. Compare evidence with that consumer's requirements before selecting an API or changing runtime behavior. This remains separate from both Auto-crop and the accepted Analyze iteration.
