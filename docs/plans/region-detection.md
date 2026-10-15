# Shared scan-region consumers

## Remaining scope

Analyze's first sampling-area review is accepted for current use. The [Analyze guide](../roll-analysis.md#1-choose-what-to-sample) owns the implemented roll-pooled conservative holder support, film exclusions, independent picture evidence and editable photo exceptions. This plan owns remaining consumer design and physical acceptance—not another implementation of that review.

Flat-field correction and future auto-cropping need related source-space evidence, but different acceptance:

- **Holder opening:** correction needs usable bare-light support inside the illuminated aperture, excluding the holder. A conservative negative-scan sampling interior is not proof that a reference's physical opening matches it.
- **Film area:** Analyze may use film margins outside the picture. Detecting film does not establish that it is undamaged or a known calibration reference; [dense-tail follow-up](roll-assistance.md#deferred-sampling-and-endpoint-work) remains separate.
- **Picture area:** statistical support need not be an accurate crop. Future auto-cropping must preserve wanted image content while excluding rebate, perforations and surrounding material; it needs its own framing/editing transaction.

These boundaries need not coincide or all be present. Bare-light references contain no film; sheet film and flatbed-style scans may have no visible holder or perforations; already trimmed scans may censor physical boundaries. Film placement can change inside a stationary holder. Do not assume a complete holder → film → picture hierarchy or one rectangle for every source and purpose.

## Consumer boundaries

Share source-space measurements, provenance and ambiguity below feature controllers where the evidence supports reuse. Keep geometric inference separate from capture/sample eligibility and source-format compatibility. Flatbed/other ingestion and its colour/response interpretation are separate work, not delivered by this detector.

Feature owners retain editing, acceptance, persistence, transactions and numerical validation. Opening evidence must not automatically install correction, change a crop or rerun Analyze. Explicit user geometry remains authoritative. Distinguish absent elements from unobserved or ambiguous boundaries; missing holder edges alone must not invalidate an otherwise usable scan, and failed detection must not certify the whole raster as film or picture.

Current [flat-field estimation](../flat-field.md) requires rectangular support containing the image-centre patch. Reuse must respect that constraint, not silently relax it. Representation and acceptance of tilted, nonrectangular, partially hidden or ambiguous reference support remain open. Compatible native development does not prove physical reference/negative compatibility. Avoid a general mask framework until demonstrated consumer cases require it.

## Evidence and acceptance

- Compare bare-light references and negatives with independently marked boundaries where observable. Include sheet film, flatbed-style and full-bleed images, missing holder/rebate/perforations, partial openings, dense legitimate scenes, faint content, uneven illumination, shifted/tilted film and damaged edges. Private sources and overlays remain ignored and read-only.
- Evaluate missed wanted content and admitted unwanted material separately for opening, film and picture. Test absent/unobserved regions and roll reordering. The largest statistical interior is not automatically the correct physical boundary or preferred crop; define consumer tolerances from observed cases before claiming quality.
- Verify nonzero active-area origins and independence from user crop/turn/straightening. Exercise correction and Analyze separately. Numerical clipping/floor checks, signed/HDR processing, original source preservation and the complete-roll contract remain unchanged.

## Next action

Choose a bounded remaining consumer slice using matching bare-light/negative examples and declared physical reference compatibility. Compare existing evidence against that consumer's requirements before selecting an API or changing runtime behavior. Future auto-crop and reference-support redesign require separate scope and acceptance; they are not unfinished pieces of the accepted Analyze iteration.
