# Coordinates, composition and pixel coverage

An image can be developed in one orientation, cropped in another, shown at a third resolution and placed anywhere on a canvas. Geometry keeps those descriptions connected without changing which original pixels a measurement or export represents.

## Give each space one job

| Space | What it describes |
| --- | --- |
| **Source** | Original developed-image pixels, before user orientation or framing |
| **Oriented source** | The active source after exact Roll and Photo turns/flips |
| **Frame** | The finished photograph after orientation, crop and straightening |
| **Raster** | A sampled representation of a declared region at a chosen resolution |
| **Canvas** | Logical UI positions, related to the image by viewport layout |

Source coordinates are developed RGB coordinates, not RAW Bayer coordinates. The metadata active area is a rectangle within the source; its origin need not be zero. A thumbnail and an export can represent the same frame at different raster resolutions; zoom and pan move that frame on the canvas without changing composition.

Coordinates use a top-left origin, x right and y down. Integer positions describe pixel edges; integer regions are half-open. Mapping a point does not silently clamp it: an outside-image click must not turn into a measurement of an unrelated edge pixel.

## Orientation and straightening are different

Shared Roll orientation comes first, followed by each Photo's orientation. Quarter-turns and flips are exact pixel permutations that transform the finished composition while preserving selected source content. Straightening instead rotates the image beneath the crop frame: the crop retains its source-coordinate centre and basis, and its corners must remain inside real source coverage — constraints may shrink the frame rather than invent blank corners.

The original source stays in place for development, illumination correction and calibration. Turning the photograph does not turn a flat-field pattern within the source.

## A bounding box is not coverage

Consider a rectangular picker over a straightened photograph. Mapping it back to the source produces a tilted polygon, and its bounding rectangle includes pixels the user never selected. Patch measurements therefore use the actual polygon: each intersecting original pixel cell contributes according to its covered area. A bounding box only limits the cells that need inspection; it cannot replace those weights. This keeps picks consistent through orientation, crop, zoom and arbitrary straightening, and full-resolution measurements never use display, proxy or already-resampled pixels.

Analyze support has a different job: complete spatial footprints on the original sampling grid. An 8×8 footprint must lie inside film support, picture footprints obey their picture rectangle and observed half-planes, and membership never substitutes a bounding box for the selected shape. Compatible captures normally share a conservative holder interior, with measured departures and authored edits kept as photo exceptions. Statistical rectangles may extend off-source; these are saved sampling choices, not completed physical boundaries or accepted crops.

## Automatic framing consumes measurements, not sampling support

[Auto-crop](editor.md#auto-crop-is-an-explicit-accepted-edit) interprets [shared nominal picture regions](spatial-metadata.md) as source-space framing, including straightening. Credible native edges and common fitted dimensions can locate missing sides; measured and inferred boundaries stay distinct. Missing film does not move the original picture frame or veto its crop, while unresolved geometry and conflicting picture evidence still abstain. Analyze qualifies material and picture sampling separately.

The bounded capture raster and original source can have slightly different scale ratios because dimensions are rounded; metadata preserves both transformed source directions. The adapter inscribes an orthogonal rectangle in nominal picture/camera coverage and rounds inward, independently of film/holder exclusions. The optional margin is a crop-consumer policy, not a biased boundary measurement: it trims an equal distance from all four edges relative to the shorter side, with angle and source centre fixed and every adjustment starting from the untrimmed proposal. Margin does not relax eligibility or alter picture statistics.

During the Auto-crop draft the viewed frame is the uncropped oriented source, separate from both accepted composition and proposed framing, so a tilted proposal stays visibly tilted against its physical boundary. Apply or Cancel ends that view; only Apply promotes geometry. Evidence belongs to the source, not to Analyze or Auto-crop — Auto-crop does not run Analyze, approve support or change grading.

[The crop consumer](../src/auto_crop.rs) owns eligibility and the rectangle adapter; [scan regions](../src/scan_regions.rs) own the measurements and their coordinate conversion.

## Process before averaging

Nonlinear processing does not commute with averaging: averaging two negative samples and then inverting them generally gives a different result from inverting each and averaging the positives. The same issue applies to display rendering:

- Original-resolution reference/inspection refinement and TIFF process/render original pixels first, then average **linear target RGB**, before output encoding.
- EXR averages the signed/HDR working positives and converts to AP0, without display rendering.

A destination pixel inverse-maps to a source footprint, and exact intersection areas provide its reconstruction weights. Exact turns and integer framing retain simple permutation/rectangular paths; arbitrary-angle framing uses polygon coverage. Thumbnails and initial/inspection previews deliberately use cheaper proxy approximations. The positive working view instead filters neutral calibrated AP1 at the visible physical-pixel footprint before grading; [processing](processing.md#physical-view-processing-is-an-approximation) explains its noncommutativity with original-sample reconstruction.

## Source-anchored local masks

Local gradients remain in original developed-source coordinates. Authoritative original-sample processing evaluates them at original pixel centres after Global grading, before any rendered/working reconstruction; the working view evaluates precise original-coordinate masks at mapped footprint centres after filtering, which remains a sampling approximation rather than export authority. Crop, turns/flips, straightening and navigation transform the photograph and its overlay, not the stored mask.

Proxy rasters must evaluate at the actual native source pixel their sampling lattice chooses; simply replacing a proxy crop with a rounded source rectangle can change that lattice, so regional processing needs explicit original-raster addressing before it can claim pitch-preserving reuse. Geometry is supplied independently of flat-field correction, so clearing correction must not erase it. [Local interaction](editor.md#immutable-local-grade-routing) describes gesture acceptance and cancellation.

## Drafts do not change accepted composition

Crop editing has a temporary frame and viewport. Apply accepts the proposed composition; Cancel restores the accepted composition and starting view. Save and Export consume only accepted state. Dmin/Dmax sampling temporarily reveals uncropped oriented source, and a flat-field reference uses untransformed source; those are viewing arrangements, not changes to the working Photo's crop. [Editor state](editor.md#accepted-state-and-drafts) explains ownership.

## Working on this area

[Geometry](../src/geometry.rs) owns shared mappings; [coverage](../src/coverage.rs) owns actual cell intersection; [viewport](../src/viewport.rs) owns image-to-canvas layout. [Frame rasters](../src/frame_raster.rs), [refinement](../src/refinement.rs) and [export](../src/export.rs) consume those definitions rather than inventing parallel coordinate conventions.

Use asymmetric images and nonzero active-area origins when testing mappings: square, centred fixtures hide mistakes. [Affine reconstruction](../tests/affine_resample.rs), [geometry export](../tests/geometry_export.rs), [viewer](../tests/viewer.rs) and [scope geometry](../tests/scopes/geometry.rs) cover the composed paths.