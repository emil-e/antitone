# Image geometry and coordinate contract

This defines the shared coordinate vocabulary and the agreed interaction requirements. Implementation coverage and remaining design questions live in the [geometry plan](../plans/flip-crop-rotate.md), not here. Exact orientation, integer cropping, arbitrary-angle straightening and coverage-aware reconstruction/measurement are implemented in this worktree.

## Coordinate spaces

| Space / term | Definition |
| --- | --- |
| **Source coordinates** | Original developed-image pixel coordinates before user orientation/crop/rotation. Not RAW Bayer/sensor coordinates. Full-resolution measurements and flat-field support use this space. |
| **Active area** | Metadata-defined usable rectangle in source coordinates. A region, not a separate space or a user crop; its origin may be nonzero. |
| **Oriented-source coordinates** | Full active area after exact roll/photo flips and quarter-turns, before straightening/crop. Origin is the oriented area's top-left edge. |
| **Frame coordinates** | Final composition after orientation, straightening and crop, relative to the composed frame's top-left edge. Without crop/straightening this coincides with oriented-source coordinates. |
| **Raster coordinates** | Pixel coordinates in a particular preview, thumbnail, tile or output. Each raster declares the region/space it represents and its resolution. Raster resolution is not composition. |
| **Canvas coordinates** | Logical UI points. The viewport maps the viewed frame or oriented source to the canvas. Physical display pixels derive from the UI scale factor. |

Texture UVs are a normalized rendering adapter, not another authoritative composition convention. Sensor-site mapping stays in the RAW bridge; geometry does not redefine Bayer coordinates.

All image spaces use top-left origin, x right/y down. Continuous coordinates have integer pixel edges and centres at `(i + 0.5, j + 0.5)`. Integer pixel indices are distinct from continuous positions. Integer source regions are half-open (`[x,x+width) × [y,y+height)`); dimensions must be positive and arithmetic bounds checked. Continuous region edges may include the far edge when constructing a selection; that does not make it a valid pixel index.

Mapping a coordinate does not silently clamp it. Source coverage checks, selection-drag constraints, raster edge handling and pixel-index conversion are explicit separate operations. Outside-image clicks must not become measurements of unrelated edge pixels.

## Ownership and mappings

A pure `geometry` module owns space-specific types, source-region/raster-grid mappings, exact orientation composition, dimensions and source↔oriented-source↔frame mappings. Consumers use these operations rather than independently implementing flip, turn, offset or nearest-centre formulae. GPU adapters serialize mappings from that same definition; their shader implementations need CPU-reference tests.

`viewport` owns transient viewed-domain↔canvas layout, Fit, magnification, pan and pointer anchoring. UI conversion to egui types stays at this boundary. Composition does not depend on egui. Colour-space transforms, output encoding and monitor conversion are not spatial geometry.

The user accepted this discrete order, extended for Hasselblad roll scanning: **active source → roll quarter-turn → roll flips in the resulting axes → photo quarter-turn → photo flips in the resulting displayed axes**. Roll orientation is the shared scanning baseline; photo orientation remains an independently stored exception applied afterward. Roll Turn left/right rotates that baseline including its flips, not a rewrite of each photo's settings. Do not infer orientation from metadata.

Exact flips/quarter-turns are pixel permutations; each 90°/270° stage swaps dimensions, with the combined roll/photo stages determining the final size. Source development, active-area origin, RGB16 storage, calibration RGB and numerical processing models remain unchanged. Source conversion/flat-field evaluation still uses original source pixel centres. Persist geometry separately from colour processing and viewport state.

## Cropping and orientation actions

Accepted crops select original source content independently of roll/photo orientation, including shared roll turns. Turn/flip controls transform the **finished composition**, keeping the selected content; a quarter-turn swaps the crop's displayed aspect. The UI exposes turn/flip actions rather than checkboxes for canonical flip flags; two flips on the same displayed axis undo one another, while rotation/reflection order remains meaningful. They do not turn the image beneath an unchanged landscape crop. Straightening in crop mode instead rotates the image beneath the frame, constrained to real source coverage: no blank/transparent exported corners.

One optional `Crop` per source stores a source-coordinate centre/pivot, positive integer frame extents in native pixel units and an optional clockwise source-plane basis `angle` in radians. Zero angle retains the exact legacy integer-edge representation; fractional source centres are allowed for a rotated basis. All four frame corners must be inside the immutable active area. Angle must be finite and within −π…π; zero is omitted on save and missing means zero. Older crop readers reject a nonzero angle rather than silently misrendering it. Exact orientation transforms the source-anchored frame/basis together, so subsequent turn/flip actions preserve the selected polygon and swap output dimensions where appropriate. The source bounding rectangle restricts iteration only. Full-frame, zero-angle Apply normalizes to no crop.

Crop editing is a transient draft:

- Drag inside the crop to move the image beneath a stationary frame; drag edges/corners to resize, retaining the opposite anchor.
- Free or locked aspect; Current/Original and common presets, with portrait/landscape swapping. Integer frame extents preserve a locked ratio to pixel rounding precision. Extent rounding retains the opposite resize anchor unless source-coverage constraints require adjustment.
- Enter/Apply commits; Escape/Cancel restores accepted geometry and the starting viewport. Reset draft is cancellable; Reset crop is independent of colour/orientation. Composition reset clears photo orientation/crop, not roll flips or grading.
- Entry Fits the entire active source, including its rotated projection, with a handle margin. Source magnification/placement stays steady during edge/corner resizing and after release, so expansion can reach surrounding source in one drag. Straightening keeps magnification and the screen pivot. Explicit viewer magnification and Fit source remain available; only Apply Fits the finished composition. Ordinary primary-click/pan navigation is not used during the draft.
- The editing view reveals and dims source content outside the frame using the same grade and mappings; it is explicitly approximate. File/export/source selection, grading, output target and picker actions cannot consume or mutate the draft. Roll/Photo tab switching only hides/reveals the photo crop controls; it does not cancel the draft or change the viewport/pipeline.

Outside-frame dragging turns the image beneath the frame about the crop's source centre. A signed displayed-angle control (−180…180°, 0.1° drag steps) and Reset straighten edit the same cancellable draft. Coverage constraints shrink the proposed frame about its pivot; move/resize proposals also constrain centre/extents to keep every corner covered. Reset draft/Reset crop clear framing including straightening, not exact roll/photo orientation or colour. The crop marquee uses a contrasting thin border, corner brackets and edge ticks with unchanged hit targets. Optional rule-of-thirds guides follow the crop frame and affect neither measurements nor exports; dense alignment guides remain deferred.

## Reconstruction

The source is treated as piecewise-constant original pixel cells. A destination pixel inverse-maps to a convex quadrilateral; its exact intersection area with each contributing cell determines the weight. Weights normalize the mean; bounds only limit iteration. This box-footprint reconstruction applies at native resolution and at reduced scale, where the enlarged footprint integrates all contributing cells for anti-aliasing. Settled preview does not oversample the native frame grid at magnification. Exact orientation/zero-angle integer crop retains its existing permutation and rectangular area-average paths, without a new interpolation step.

Processing is per original source pixel, before averaging. Settled preview and TIFF apply unchanged ACES to each processed pixel, then average **linear target RGB**, before presentation/target encoding. EXR averages signed/HDR scene-linear data and converts AP1→AP0 without ACES, transfer or clamping. CPU export uses f64 geometry/compensated sums; GPU reconstruction uses an affine f32 adapter and bounded source tiles. Spatial shader tolerances are separate from unchanged colour-processing gates. Interactive preview and thumbnails retain their bounded approximate source proxies and affine presentation; they are not measurement/export sources.

## Measurements

Picker selections are **canvas-aligned rectangles**, not source-aligned outlines. Full-resolution measurements inverse-map the selection's actual coverage into original source pixels; a source bounding box may bound iteration but must not replace that coverage. This is a sampling requirement, not a requirement that the shape remain a source rectangle: the same principle would apply to a circle or hexagon. Developed RGB averaging and native clipping diagnostics must agree on contributing coverage when arbitrary rotation is added. No display/proxy/resampled values are measured. Only measured RGB/balance values persist, never a selection rectangle or mask.

Selections snap/constrain in the final frame's native pixel grid and remain axis-aligned on canvas at Fit, zoom and pan. With exact orientation/integer crop, their inverse is an exact source rectangle and uses the existing averaging/diagnostic path. Rotated selections use exact positive cell-intersection area with compensated full-resolution means; neutral fitting weights each individually processed pixel, not an averaged negative input. Native diagnostics cover precisely the same contributing developed-pixel index cells, including partially covered boundary cells, via contiguous row spans in the existing RAW bridge. Zero-area bounding-box-only cells contribute neither RGB nor diagnostic sites. Any contributing cell at black/white, or an unusable fitting input, rejects the pick without changing processing state.

## Panels and picker views

Changing Roll/Photo panels alone changes control scope, not the composition or numerical preview pipeline. Developed-source inspection ordinarily retains the photo composition while bypassing numerical grading.

- **Gamma-balance neutral and photo WB picks:** retain the current composition, displayed grade/balance and navigation. Starting a picker adds selection interaction; it does not bypass photo edits or reveal hidden pixels. Numerical fitting retains its existing inputs/operators, independent of what helps identify a target visually.
- **Dmin/Dmax picks:** start on the current source with an empty region. Temporarily show developed source over the full active area, retaining exact roll/photo orientation but bypassing photo crop/straightening. This exposes film base/leader without changing persistent geometry. Accepted source correction remains active; inspection gain remains display-only. Full-resolution measurements map to original source pixels and store only RGB.
- Entering any picker preserves precision navigation. Dmin/Dmax's viewed-domain change preserves physical magnification and the source-coordinate anchor at the canvas centre, rather than invoking Fit or copying a centre expressed in another space. Restore the original composition/viewport on completion, rejection or cancellation. A temporary reference RAW must not overwrite the working-photo return context.
- Shared roll orientation applies to temporary reference sources. Registered sources use their own exact photo orientation; unregistered sources start with identity photo orientation. Crop/straightening are never used to reinterpret a calibration reference.
- Geometry controls are unavailable while picking/measuring. Invalid/cancelled picks and transient view changes alter no persistent geometry or processing state.

Flat-field reference editing is a separate bare-light workflow, not emulsion-side photo calibration. It temporarily shows untransformed, uncropped original source through the shared canvas layout; its integer support is never photo crop/frame state. Accept/Cancel restores the working composition and viewport. Sparse-preview and full-tile GPU field lookup use canonical nearest-centre `SourceRaster` coordinates and an equivalent shader adapter. Correction happens before native-camera inversion and camera→AP1; rotated reconstruction, measurements and exports evaluate gains at original source pixel centres. Stored calibration and field identity do not change when geometry changes.

## Alignment audit

At geometry integration closure, inspect every existing spatial mapping: metadata active-area offsets, source proxies, full-resolution tiles/flat-field coordinates, viewport/navigation, picker rectangles/handles, provisional presentation, settled reconstruction, thumbnails, and export raster callbacks. Confirm coordinate types, pixel-centre rules, coverage, rounding and forward/inverse consistency against the shared utilities. Record exceptions with their reason (for example, deliberate transient drag clamping or sensor-site mapping); do not claim consistency from similar-looking formulae.

Retain asymmetric non-square fixtures, nonzero active origins and exact pixel mapping tests. Spatial/filter tolerances do not replace or weaken colour-processing gates. Future scopes cover the exported composition; they are not implemented by this contract.
