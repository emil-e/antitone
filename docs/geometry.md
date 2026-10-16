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

Source coordinates are developed RGB coordinates, not RAW Bayer coordinates. The metadata active area is a rectangle within the source; its origin need not be zero. Texture UVs are an adapter for drawing, not an additional authoritative composition space.

A thumbnail and an export can represent the same frame at different raster resolutions. Zoom and pan move that frame on the canvas; they do not change composition.

Coordinates use a top-left origin, x right and y down. Integer positions describe pixel edges; a pixel's centre is halfway between them. Integer regions are half-open, so their far edge is not another pixel index. Mapping a point does not silently clamp it: an outside-image click must not turn into a measurement of an unrelated edge pixel.

## Orientation and straightening are different

Shared Roll orientation comes first, followed by each Photo's orientation. Quarter-turns and flips are exact pixel permutations. They transform the finished composition, preserving selected source content; a quarter-turn can swap its displayed dimensions.

Straightening instead rotates the image beneath the crop frame. The crop retains its source-coordinate centre and basis, and its corners must remain inside real source coverage. Constraints may shrink the frame rather than invent blank corners.

The original source stays in place for development, illumination correction and calibration. Turning the photograph does not turn a flat-field pattern within the source. The [crop implementation](../src/crop.rs) owns exact framing and constraints.

## A bounding box is not coverage

Consider a rectangular picker over a straightened photograph. Mapping it back to the source produces a tilted polygon. Its bounding rectangle includes pixels the user never selected.

Patch measurements use the actual polygon. Each intersecting original pixel cell contributes according to its covered area. A bounding box only limits the cells that need inspection; it cannot replace those weights. Capture-clipping diagnostics inspect the same contributing cells as the RGB measurement.

This keeps picks consistent through orientation, crop, zoom and arbitrary straightening. Full-resolution measurements never use display, proxy or already-resampled pixels. Only the resulting calibration/balance values persist, not calibration-picker coverage.

Analyze support has a different job: select complete spatial footprints on the original canonical sampling grid. An 8×8 footprint must lie inside film support, and picture footprints also obey their picture rectangle and observed half-planes. Membership never substitutes a bounding box for the selected shape or replaces numerical eligibility. Opening and picture poses may differ. Compatible captures normally share a conservative holder interior, with measured departures and authored edits kept as photo exceptions. The shared interior is inscribed against pooled exterior evidence and fine safety bounds, so rounded corners need not be admitted just because straight side fits intersect there. Picture coverage follows opening edits against retained independent frame evidence; authored picture geometry stays fixed. Statistical rectangles may extend off-source and use coverage constraints for missing sides; actual sampling stays within the active source. These are saved sampling choices, not completed physical boundaries, accepted crops or automatic straightening.

## Automatic framing consumes measurements, not sampling support

Auto-crop interprets retained independent picture-edge measurements as a source-space frame, including straightening. It does not crop to the conservative opening or the opening-limited picture statistics rectangle. Four strong measured sides are required; weak, missing, authored statistics-only and off-source boundaries do not certify a crop. Conflicting edge fits also abstain rather than silently moving or shrinking a frame into coverage. Full-bleed scans can therefore remain uncropped even after successful Analyze.

The bounded capture raster and original source can have slightly different scale ratios because dimensions are rounded. The adapter maps the measured frame into source coordinates, inscribes an orthogonal rectangle in that mapped parallelogram and rounds its extent inward. Zero-angle crops retain integer source edges. Roll/Photo turns and reflections remain independent; they never change which source pixels the proposal selects.

The optional inward margin is a crop-consumer policy, not a biased boundary measurement. It removes an equal distance from all four edges, relative to the original proposal's shorter side; it is not uniform aspect-ratio scaling. Angle and source centre stay fixed. Pixel rounding goes inward, with symmetric whole-pixel trims for zero-angle framing and a positive minimum extent for tiny frames. Every adjustment starts from the untrimmed proposal, so returning to an earlier value restores the same result. Margin does not relax eligibility or rescue missing/off-source edges, and it does not alter picture statistics or Analyze's independent sampling safeguards.

During the [Auto-crop draft](editor.md#auto-crop-is-an-explicit-accepted-edit), the viewed frame is the uncropped oriented source, separate from both accepted composition and proposed framing. The outline maps draft crop corners through that source-to-canvas composition, so a tilted frame stays visibly tilted against its physical boundary. Edge/corner drags inverse-map into the original proposal's own axes and adjust the same symmetric margin; they do not freely translate or rotate it. The physical working/original view does not change on each margin tick. Apply or Cancel ends that view and restores normal accepted composition; only Apply promotes geometry.

Evidence belongs to the source, not to Analyze or Auto-crop. The current first version reads the compact measurements already persisted with accepted source sampling areas; Analyze remains their only runtime producer. Auto-crop does not run Analyze, approve support or change grading. Independent lazy measurement requests and separation of raw evidence lifetime from sampling edits remain [follow-up work](plans/region-detection.md). Geometry acceptance is not intrinsically dependent on successful colour fitting.

[The crop consumer](../src/auto_crop.rs) owns eligibility and the rectangle adapter; [scan regions](../src/scan_regions.rs) own the measurements and their coordinate conversion. Their tests establish geometry/provenance behavior, not universal photographic border accuracy.

## Process before averaging

Nonlinear processing does not commute with averaging. Averaging two negative samples and then inverting them generally gives a different result from inverting each and averaging the positives.

The same issue applies to display rendering:

- Original-resolution reference/inspection refinement and TIFF process/render original pixels first, then average **linear target RGB**, before output encoding.
- EXR averages the signed/HDR working positives and converts to AP0, without display rendering.

A destination pixel inverse-maps to a source footprint. Exact intersection areas provide its reconstruction weights. Exact turns and integer framing retain simple permutation/rectangular paths; arbitrary-angle framing uses polygon coverage. Thumbnails and initial/inspection previews deliberately use cheaper proxy approximations. The positive working view instead filters neutral calibrated AP1 at the visible physical-pixel footprint before grading; [processing](processing.md#physical-view-processing-is-an-approximation) explains its noncommutativity with original-sample reconstruction. Draw bounds and raster dimensions align together to physical pixels, preserving 100% pitch. Navigation can transform retained support, but replacement acquisition always returns to originals. Original tiles alone do not bound magnified destination work; acquisition and reference reconstruction also split destination coverage into bounded windows without reducing output density.

## Source-anchored local masks

Local gradients remain in original developed-source coordinates. Authoritative scalar/GPU original-sample processing evaluates them at original pixel centres after Global grading, before any rendered/working reconstruction. The internal view-sized candidate evaluates precise original-coordinate masks at mapped footprint centres after filtering; this approximation has no normal-editor local admission yet. Crop, turns/flips, straightening and viewport navigation transform the photograph and its overlay, not the stored mask.

Nearest-centre proxies must evaluate at the actual native source pixel chosen by their sampling raster; reusable tiles retain their origins and bypass locals in padded samples outside the declared area. Geometry is supplied independently of flat-field correction, so clearing correction must not erase it. Simply replacing a proxy crop with a rounded source rectangle can change its sampling lattice; regional processing needs explicit original-raster addressing before it can claim exact pitch-preserving reuse.

## Drafts do not change accepted composition

Crop editing has a temporary frame and viewport. Apply accepts the proposed composition; Cancel restores the accepted composition and starting view. Save and Export consume only accepted state. Changing inspector tabs does not implicitly accept or cancel a tool.

Dmin/Dmax sampling temporarily reveals uncropped oriented source, while a flat-field reference uses untransformed source. Those are viewing arrangements, not changes to the working Photo's crop. Tool exit restores its working context. [Editor state](editor.md#accepted-state-and-drafts) explains ownership.

## Working on this area

[Geometry](../src/geometry.rs) owns shared mappings; [coverage](../src/coverage.rs) owns actual cell intersection; [viewport](../src/viewport.rs) owns image-to-canvas layout. [Frame rasters](../src/frame_raster.rs), [refinement](../src/refinement.rs) and [export](../src/export.rs) consume those definitions rather than inventing parallel coordinate conventions.

Use asymmetric images and nonzero active-area origins when testing mappings: square, centred fixtures hide mistakes. [Affine reconstruction](../tests/affine_resample.rs), [geometry export](../tests/geometry_export.rs), [viewer](../tests/viewer.rs) and [scope geometry](../tests/scopes/geometry.rs) cover the composed paths. Spatial tolerances do not replace colour-processing gates.
