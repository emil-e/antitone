# Non-destructive flip, crop and rotation

Status: geometry and crop UX are user-accepted for now. Native-camera/model-4 and flat-field coordinate integration is implemented and combined tests pass. Photographic/onscreen acceptance and dense alignment guides remain separate. The user now authorizes local checkpoint commits, rebase onto main and integration/commit to main; pushes remain unauthorized. [The roadmap](../../PLAN.md) owns cross-package priority.

## Goal and scope

Give each photo a durable composition that agrees across the main viewer, full-resolution pickers, thumbnails and TIFF/AP0 EXR exports, without modifying its RAW or changing its grade.

The existing [completion requirements](completion.md#geometry) establish the feature scope, not the detailed design:

1. Independent roll/photo quarter-turns and horizontal/vertical flips. Shared roll turns were explicitly added for Hasselblad scanning after the integer-crop checkpoint.
2. Photo crop and arbitrary rotation/straightening, built on the same geometry model.

Keep roll flips independent of photo edits. Geometry reset must not reset calibration, WB, response or exposure; photo reset must not erase roll flips. The normal developed-source view should use the same composition as the positive view; any uncropped measurement view must be explicitly transient.

Non-goals: perspective correction, lens correction, auto-crop/auto-straighten, dust removal, batch export, new grading operators, a general progress framework, scopes implementation, recovery/relink or a general undo system. Scopes and undo are integration requirements when those facilities exist, not permission to implement them here. Until then, scoped reset and cancellable crop/rotation edits are needed, and missing undo remains a stated limit rather than a claimed acceptance pass.

## Baseline and integration boundary

The worktree `.worktrees/flip-crop-rotate`, branch `plan/flip-crop-rotate`, began at `ace5628`. Geometry was checkpointed locally as `bfcf666`, then rebased onto main `5efb302` (`Replace AP0 inversion with native-camera RGB`), following flat-field `c0b664a`. Earlier checkpoint sections below describe the historical AP0/schema-3 baseline, not supported current processing.

Current integration preserves native-camera model/schema 4 and its camera-basis identity. Flat-field gains are evaluated in **original source pixel coordinates before native-camera inversion and camera→AP1**. Geometry must not transform field support, reinterpret it as a photo crop, invalidate reference identity or resample stored Dmin/Dmax. Sensor/source diagnostics retain their original source/CFA boundary. Other worktrees and main's unrelated local changes stay untouched; no push is authorized.

## Initial implementation inventory

This table records the pre-change investigation at `ace5628`; the implementation checkpoint below records what has since changed.

| Boundary | Existing implementation | Required change |
| --- | --- | --- |
| Source | `src/raw.rs`, `src/scan.rs`: metadata active area within developed dimensions; native development explicitly uses `user_flip = 0` | Keep development and active-area origin unchanged. Do not silently start applying camera orientation metadata. |
| Durable state | `src/project.rs`: schema 3; shared `Project.roll`, stable-ID `Source.photo`, strict validation/unknown-field rejection | Separate geometry from numerical processing settings; persist roll flips and source-ID-owned photo composition with identity defaults and an agreed compatibility policy. |
| Viewport/navigation | `src/viewport.rs`, `src/editor_navigation.rs`: uniform scale/pan and axis-aligned source rectangle | Separate persistent composition from transient Fit/zoom/pan; map through composition coordinates. |
| Picker | `Geometry`, `PickDrag`, `Command::Pick`, `Scan::{reference,roll_neutral,photo_neutral}`: integer source rectangles and full-resolution measurements | Inverse-map interactions and forward-map overlays; never measure a displayed/resampled pixel or silently expand a rotated selection to its bounding box. |
| Provisional viewer | `src/viewer.rs`, `ImagePaint`: bounded RGB16 source proxy and rectangular UV range | Apply the shared orientation/composition without a separate UI-only convention. |
| Settled viewer | `src/refinement.rs`, `src/resample.rs`: `ViewRequest { area, output }`; tiled full-source processing and linear-target area averaging | Represent transformed output coverage and required source footprint, not just the source bounding rectangle. |
| Thumbnails | `src/editor_thumbnails.rs`: source/development proxy cache, source/target image cache, per-photo processing on paint | Reuse source proxies; apply composition and transformed aspect. Include geometry wherever it changes cached pixels or request/result validity. |
| Export | `Command::Export` in `src/editor.rs`; `src/export.rs` receives dimensions and pixel callback | Snapshot composition and grade together, derive composed dimensions and sample the original full-resolution source. Preserve destination safety. |

`raw::Rect` is an integer source region, not a sufficient arbitrary-rotation/crop representation. Avoid teaching each consumer a separate flip/rotation formula. A small shared geometry module is proposed; it should not depend on egui or introduce an alternative RAW/development path.

## Agreed foundation and remaining proposals

### Coordinate and orientation contract

The user approved the central coordinate vocabulary/utilities and requested implementation plus a final audit of existing mappings. Canonical terminology, pixel conventions, ownership and picker-view requirements live in the [geometry contract](../contracts/geometry.md), not duplicated here.

The discrete order is agreed: **active source → roll quarter-turn → roll flips → photo quarter-turn → photo flips**, with flips in each stage's resulting axes. The user explicitly approved shared roll turns for Hasselblad rolls. Roll orientation corrects scanning setup; unchanged individual photo orientation is applied afterward as an exception. Turn actions advance/decrement turns and swap that stage's flip flags: roll buttons turn the scanning baseline, photo buttons turn the final displayed result.

Retain independently editable roll/photo state; derive a canonical effective orientation for sampling. Exact flips/quarter-turns are pixel permutations with no interpolation. 90°/270° swap dimensions; 0°/180° do not. Identity must preserve existing output and picker behavior.

Implemented ownership: a separate roll orientation value in the project and photo orientation in each `Source` record, not added to the colour-only `RollSettings`/`PhotoSettings` pipeline. Shared roll orientation, including turns, also applies to newly imported photos; per-photo state remains stable through selection, removal/re-add and Save/Open. Registered reference sources use their own discrete photo orientation; unregistered sources use identity photo orientation, both with shared roll orientation.

### Crop, straightening and measurements

Store crop intent in declared original-source coordinates, never screen pixels or a proxy's dimensions. Keep the original active area immutable and preserve hidden pixels for later crop changes and measurements.

The user accepted a rectangular, no-padding frame, constrained to actual image coverage. They chose turn/flip actions that transform the finished composition and preserve selected content; straighten instead turns the image beneath the frame. They rejected source-aligned picker outlines: selections must stay canvas-aligned and average their actual inverse-mapped coverage, not a larger source bounding box. Only measured values persist. These decisions live in the [geometry contract](../contracts/geometry.md).

Lightroom-style interaction is agreed: image-under-frame dragging inside, edge/corner resize, outside-frame drag to straighten, aspect lock, Enter Apply and Escape restore. The subsequent approved UX redesign pins crop controls at the top, Fits full source on entry and keeps the editing magnification/placement steady through resizing/release. Rule-of-thirds guides are now implemented; dense guides remain deferred. Both affect neither sampling nor exports. Reference: [Julieanne Kost's Lightroom Classic crop/straighten guidance](https://jkost.com/blog/2024/08/crop-and-straighten-images-in-lightroom-classic.html); no Lightroom/Resolve automation was performed.

The integer checkpoint stores one source-anchored centre/native extent per photo, not screen coordinates or a competing source/output rectangle. It supports free/Current/Original/common locked aspects and exact source-edge cropping. Straightening now extends centre/extent state with an explicit source-plane basis angle about the stored source centre and real transformed sampling. Zero angle retains strict integer source edges and old serialization; a nonzero angle allows fractional source centres and validates all four corners. `raw::Rect` bounds iteration, never the sampled rotated shape.

Dmin/Dmax's uncropped transition is wired for this checkpoint. Gamma/WB retain composition and displayed grade. Fitted magnification is frozen while picker toolbar layout changes canvas size, and Dmin/Dmax restore the original viewport even after reference-source navigation. Gamma/WB preserve deliberate in-picker navigation; unchanged entry/exit restores the initial Fit state.

Read-only sampling investigation found developed RGB16 per-pixel access but only rectangular native Bayer diagnostics. Coverage-aware selection must update **both** averaging and diagnostic coverage. Row spans can reuse the existing native diagnostic API without measuring bounding-box-only sites; validate the actual boundary-weight/inclusion rule with scalar fixtures before wiring rotation. Native sensor-site indexing/margins remain the RAW bridge's responsibility.

### Sampling and compatibility

Do not interpolate negative/source RGB and then invert it: inversion and averaging do not commute. Implemented sampling boundaries:

- Flips/quarter-turns and integer-aligned crops: exact full-resolution pixel remapping.
- Settled preview and TIFF resampling: process each contributing full-resolution pixel, apply unchanged ACES into linear target RGB, then exact cell-intersection box-footprint averaging before target/presentation encoding. TIFF and settled preview share coverage/filter rules, with existing CPU/GPU precision limits retained.
- AP0 EXR resampling: filter processed scene-linear signed/HDR data, with AP1→AP0 conversion and no ACES/transfer/monitor transform. Never borrow TIFF pixels or clamp negatives.
- Interactive preview/thumbnails remain bounded approximations, clearly distinct from measurement/export sources.

Choose and validate the actual reconstruction/downsampling filter, source footprint and edge coverage before arbitrary rotation. Specify anti-aliasing at reduced scale and behavior at 100%/magnification, not just an interpolation name. Keep bounded tile/halo temporaries and one RGB16 source representation; no retained wide whole-source cache or CPU interactive fallback. Exact orientations should not acquire filtering changes as a side effect.

Geometry changes must invalidate affected settled work and prevent old generations installing. Source proxy identity should remain geometry-independent when composition is applied after proxy generation. Export requests must hold the same accepted roll/photo geometry snapshot as their processing snapshot, even if the UI changes later.

Current persistence uses main's schema/model 4. Optional roll/source orientation and crop default to identity/full source and are omitted when identity; supported pre-geometry camera documents retain their numerical state. Strict unknown fields/noncanonical turns reject. AP0/model-2/3 documents remain incompatible, without migration. Geometry does not redefine numerical/development/output identities. The schema-3 behavior recorded in historical checkpoints below is superseded by this integration.

## Remaining engineering choices

| Decision | Recommendation / tradeoff |
| --- | --- |
| Persistence integration | Optional geometry is integrated with schema/model 4 and accepted flat-field state; AP0 schemas/models remain rejected. |
| Straighten storage/order/pivot | Implemented source-plane basis angle about source centre; exact orientation transforms frame/basis together and preserves selected content. |
| Empty rotated corners | Constrain crop to actual source coverage; accepted. No padded/transparent exported corners. |
| Rotated selection sampling | Implemented positive cell-intersection area with weighted fitting and matching native diagnostic row spans; no bounding-box substitution. |
| Interaction and reset | Outside drag, −180…180° displayed-angle entry (0.1° drag step), numeric/slider/reset controls and thirds guides share the cancellable draft. Full-source editing stays steady across resize/release; dense guides remain deferred. |
| Filter and numerical acceptance | Exact box-footprint cell coverage is specified/tested independently, including a diamond mask and GPU tiled scalar agreement. Reduced scale integrates the full footprint; magnification retains native-grid settled output. Photographic/performance/device acceptance remains separate; processing gates are unchanged. |

## Implementation sequence

1. **Settle the exact-orientation slice.** Recheck the baseline and agree order, control semantics and persistence compatibility. Add the agreed geometry contract; add an ADR only for consequential choices needing durable rationale.
2. **Pure state and mapping.** Implement identity/defaults, dimensions, exact pixel-centre forward/inverse mapping and persistence validation. Test independently of RAW/native/UI dependencies. Keep viewport/picker context transient.
3. **Deliver flips/quarter-turns end to end.** Wire Roll/Photo controls, viewer/navigation, transformed picker overlays, provisional/settled rendering, thumbnails and snapshot-based TIFF/EXR export. Existing regional area averaging can be reused in source orientation for exact permutations, provided destination dimensions/order are explicit and equivalent. Stop for a verified checkpoint; a viewer-only implementation is not completion.
4. **Settle crop/straightening design.** Resolve the remaining storage/order/pivot, picker, valid-coverage and filter questions against the working discrete model. Integer crop can be a small checkpoint, but must not lock in an incompatible arbitrary-rotation representation.
5. **Deliver crop/straightening.** Add draft interaction/validation, transform-aware coverage and bounded resampling; integrate every existing consumer, Save/Open and export. Add scopes/undo hooks only to actual existing APIs, otherwise record the outstanding dependencies.
6. **Verify, audit and document.** Inspect every existing spatial transformation against the shared geometry system, including source/proxy/tile/flat-field, viewport/picker, presentation/refinement, thumbnail and export mappings. Consolidate duplicate definitions; record intentional boundary exceptions and outstanding integration rather than masking differences. Update contracts/architecture, record focused checks and acceptance limits here, and move this plan to `completed/` only when the agreed scope is accepted.

## Acceptance and verification plan

Use small synthetic asymmetric images: distinct labelled corners, off-centre markers, odd/non-square dimensions, one-pixel edges and nonzero active-area origins. Samples stay read-only and photographic outputs stay ignored. Run checks from this worktree root through `./scripts/cargo.sh`; use [the runbook](../build-and-probes.md), not historical test totals.

- **Mapping:** all roll/photo flip combinations × four quarter-turns, including equivalent effective orientations; dimensions, every pixel mapping, forward/inverse round-trips, repeat/reset operations and action order. Exact orientations must have zero sample-selection error.
- **Picking/navigation:** same original source pixels/RGB at Fit, Retina 100%, zoom and pan; transformed corner handles/cursors, move/resize, active-area edges and source bypass. Empty-area/rejected/cancelled picks change no processing state. Crop cannot make film-base calibration impossible.
- **Preview/thumbnail/export agreement:** asymmetric fixture positions and aspects agree across provisional and settled views, thumbnails, decoded sRGB/P3 TIFF and half/full AP0 EXR. Test visible-region refinement, geometry changes during work and stale-result rejection; no source redecode solely for orientation edits.
- **Crop/rotation:** exact integer crop, full-frame identity, minimum valid crop, fractional edges, positive/negative and near-quarter-turn angles, changed orientation after crop, invalid/NaN/infinite values and coverage boundaries. Check the chosen filter against scalar references, reduced-scale aliasing and seam-free tiled/halo handling. Scene-linear EXR retains signed/HDR values; target filtering occurs before encoding.
- **State/files:** identity load of pre-geometry supported projects under the agreed policy, exact Save/Open round-trip, stable source ownership, new imports under roll flips, scoped resets, Cancel restoring the saved/dirty baseline and failed Open retaining the current document. Export dimensions/metadata describe actual composed pixels; overwrite safeguards and unchanged RAW bytes remain covered.
- **Integration invariants:** geometry does not change calibration RGB, source fingerprints, development, response model, ACES or flat-field coordinates. Verify corrected and uncorrected sources once the separate flat-field work is integrated. Future scopes must cover the exported composition, not hidden full-source pixels; undo/recovery are not claimed while unwired.
- **Checks:** pure geometry/project tests first, then focused viewport/editor/thumbnail/GPU/refinement/export tests. Run formatting and strict relevant Clippy/build checks at integration checkpoints. Finally perform an unlocked-desktop real-source exercise of orientation, crop Apply/Cancel, transformed picks, Save/Open and export reopen; synthetic/readback success alone is not onscreen acceptance.

## Progress and handoff

### Historical exact-orientation checkpoint

- Added the [geometry contract](../contracts/geometry.md) and dependency-free `src/geometry.rs`: source/oriented/frame/raster point types, half-open source regions, exact nearest-centre source-raster mapping, independently editable orientation, exact composition/pixel permutation and affine texture adapters.
- `raw::Rect` now aliases the canonical source-region type. Source previews and project region validation use shared utilities. `viewport::CanvasLayout` names the UI boundary explicitly, stores its centre as a frame point, maps through composition, and keeps drag constraints/UI f32 boundary snapping separate from coordinate conversion. Picker resize cursors use canvas direction, not transformed source direction.
- Optional durable roll/photo orientation, independent controls/reset, load/New/Open/Save synchronization, working-photo return and immutable export snapshots are wired. Geometry controls are disabled while picking/measuring; accepted orientation controls Fit the new composition. Picker entry itself never Fits or resets navigation.
- Provisional and settled textures remain in source orientation; shared affine presentation maps either to the composed frame. Source-space tiled area averaging therefore stays unchanged for exact permutations; composition participates in settled generation/signature invalidation. Thumbnails reuse source/processed texture caches and apply current roll/per-source composition at paint time. TIFF/AP0 EXR callbacks permute full-resolution source pixels and swap output dimensions.
- No crop, arbitrary rotation, roll turns, general undo or flat-field implementation was copied/added. Dmin/Dmax's future uncropped-domain transition still needs wiring with crop/straightening. Gamma/WB keep their existing displayed grade and viewport; fitting equations are unchanged.

Fresh verification snapshot in this worktree on local Apple Silicon/Metal, using `./scripts/cargo.sh` and `CC=/usr/bin/clang CXX=/usr/bin/clang++`:

- Dependency-free geometry unit tests: 5 passed; project/film-project integration tests: 11 passed.
- `test --offline --features editor --lib`: 76 passed, including orientation/picker navigation, temporary-reference return, source ownership, export snapshot and disabled/scoped reset coverage.
- Three focused `--features editor --test viewer` checks passed: exact oriented presentation across all 64 roll/photo combinations per target, existing CPU/target/presentation agreement, and tiled fractional area averaging. No large-buffer/device acceptance check was rerun.
- `test --offline --features export --test geometry_export --test export`: 5 passed, including composed sRGB/P3 TIFF and half/full AP0 EXR reopen dimensions/pixel layout.
- Strict `clippy --offline --features editor --all-targets -- -D warnings`, formatting and debug editor binary build passed. No release build, real-RAW or unlocked-desktop acceptance is claimed.
- Initial project compilation failed because inherited `CC` named absent `/opt/homebrew/bin/gcc-15`; explicit documented Apple Clang fixed it. An initial new GPU test failed to compile due to an array/vector test-fixture conversion; repaired before the successful checks. No gate was weakened.

Dependencies/toolchain are read through ignored worktree-local `.cache` symlinks to the original checkout's existing cache; build products are worktree-local. The independent flat-field work remained untouched; it has since been committed on `main` as recorded above. No commits/pushes or sample changes.

### Transformation alignment audit

Initial exact-slice inspection:

| Existing mapping | Alignment / intentional boundary |
| --- | --- |
| Metadata active area | Source-coordinate region and nonzero origin retained; native `user_flip = 0` and sensor-site mapping unchanged. Metadata orientation is not silently applied. |
| Source proxy | `Scan::preview_source` uses canonical `SourceRaster::source_pixel`; continuous raster mapping and extreme integer arithmetic have reference tests. |
| Full-source tiles / settled preview | Tiles retain original source origin; `ViewRequest.area/output` describe a source-space raster. Post-ACES linear-target area averaging is unchanged, then exact orientation is applied in presentation. Crop/rotation coverage is not implemented. |
| Viewport and pickers | Composition forward/inverse mapping is shared; typed frame centre, explicitly constrained half-open source selection, canvas-direction handles and exact-orientation round-trip tests. UI f32 snapping only resolves boundary round-trip noise, not processing precision. |
| Presentation and thumbnails | Shared six-component affine UV adapter; GPU readback verifies exact source indices. Thumbnails do not cache oriented pixels, so orientation intentionally stays out of source/image-cache identity. |
| Export | Same composition maps output centres to original pixels; independently expected TIFF/EXR reopen fixture and command snapshot test cover layout and dimensions. |
| Flat-field (separate original checkout) | Existing `SourceGeometry` still has its own integer source-raster/shader definition. Reconcile it with `SourceRaster` and test CPU/GPU pixel centres when that work is integrated; do not claim completed alignment now. |

The whole-feature audit remains an explicit closure task after crop/straightening and flat-field integration. One independent read-only exact-slice review found no actionable defects; it did not rerun tests and retained the real-RAW/onscreen and deferred-integration limits. Final diff inspection and `git diff --check` passed; a local whitespace/relative-link/anchor check passed across 10 changed/new Markdown files and 86 links/anchors.

### Integer-crop checkpoint

- The user ran the exact-orientation build and reported “Works great, continue.” They subsequently accepted finished-composition turns/flips, constrained source coverage, Lightroom-style draft interaction and canvas-aligned measurement selections.
- Added source-ID-owned optional centre/extent crop state with strict integer-edge/active-area validation; New/Open/selection/Save and full-frame normalization are wired. Changed-source acceptance rejects incompatible crop coverage before mutating the record.
- `Composition` keeps full-active-source orientation distinct from its selected source region and translated frame origin. Crop/turn/flip mappings drive provisional/settled presentation, thumbnail aspect/UVs and immutable full-resolution TIFF/AP0 EXR snapshots. The exact slice adds no interpolation or colour/development change.
- Crop mode reveals/dims the surrounding processed source, moves the image beneath the frame and resizes all eight edge/corner handles through shared mappings. Free/Current/Original/common ratios, swap aspect, Reset draft, Apply/Cancel and Enter/Escape are wired. Drafts never dirty the document or enter file/export snapshots. Geometry reset clears photo orientation/crop; Reset crop leaves orientation/colour intact.
- Dmin/Dmax reveal uncropped oriented source while preserving physical magnification and original source anchor; the working viewport/crop is restored. Picker context also freezes Fit during toolbar/canvas-size changes. Gamma/WB keep crop/grade and preserve intentional in-picker navigation. Source proxy/cache identities remain geometry-independent.
- Ordinary navigation is suspended during draft primary interaction; explicit magnification controls remain. At this historical checkpoint, crop entry and resize release Fit with a handle margin; superseded by the stable source-view redesign below. Straightening/outside drag, fractional framing and guides are intentionally not exposed. No independent flat-field integration or whole-feature audit is claimed.

Fresh integer-crop verification snapshot in this worktree on local Apple Silicon/Metal, through `./scripts/cargo.sh` with `CC=/usr/bin/clang CXX=/usr/bin/clang++`:

- Dependency-free `test --offline --lib geometry::` and `crop::`: 7 passed in total.
- `test --offline --features editor --lib`: 88 passed, including all crop edge/corner/ratio interactions, canvas-direction drag cursors, actual Crop-button/Enter/Escape events, draft/dirty-state isolation, anchored uncropped picking and real picker-toolbar canvas-size changes. Gamma/WB exit also retains deliberate in-picker navigation.
- `test --offline --features project --test project --test film_project`: 12 passed; crop Save/Load/source ownership, legacy omission, invalid coverage/fractional/empty state and atomic incompatible changed-source rejection are covered.
- Three focused `--features editor --test viewer` checks passed: `oriented_presentation_matches_exact_source_pixel_mapping` now covers all 64 discrete combinations per target in full, cropped and crop-editor extended-source mappings; `tiled_linear_area_average_matches_scalar_fractional_footprints` and `reusable_gpu_viewer_matches_cpu_targets_and_extended_srgb_presentation` retain their existing gates. Source-exterior pixels discard rather than stretching edge pixels. No large-buffer/device acceptance check was rerun.
- `test --offline --features export --test geometry_export --test export`: 5 passed; independent full/cropped sRGB/P3 TIFF and half/full signed/HDR AP0 EXR reopen fixtures agree exactly.
- Final diff inspection and `git diff --check` passed; local Markdown whitespace/relative-link/anchor validation passed across 10 changed/new documents and 87 links. Formatting, strict `clippy --offline --features editor --all-targets -- -D warnings` and debug `build --offline --features editor --bin antitone` passed. The runnable binary is `.worktrees/flip-crop-rotate/target/debug/antitone` from the original root.
- Initial crop compilation needed an explicit two-element array annotation. Headless control/event fixtures needed scrolling to the offscreen Geometry section and normal panel-layout warm-up before comparing stable image bounds. The actual Fit/canvas-size transition was repaired in shared picker context, not masked by relaxed precision assertions. No processing/colour gate was weakened.

No release build, agent-run photographic/onscreen crop acceptance or independent combined-feature review is claimed. Arbitrary-angle/fractional framing, polygon diagnostics, guides, flat-field integration and the whole-feature audit remain. No commits/pushes or sample edits.

### Shared roll-turn checkpoint

The user explicitly approved Roll → Scan orientation Turn left/right for Hasselblad scanning, with roll orientation first and unchanged per-photo exceptions afterward. Added `RollOrientation` and shared discrete-stage forward/inverse mappings/dimension calculation; crop coverage remains anchored to the same original source pixels. Roll reset remains separate from photo/crop/colour resets; geometry controls stay disabled during picking and crop drafts.

Schema 3 preserves flat roll `horizontal`/`vertical` fields and adds optional canonical `turns`; missing/zero turns preserve the old flip-only representation exactly. Identity remains omitted. Nonzero turns are rejected by older flip-only readers, not silently ignored. Source photo orientation, numerical models, calibration, development and output identities are unchanged. New photos, temporary references, thumbnails and immutable export snapshots all use the shared roll baseline without redecode or interpolation.

Fresh local Apple Silicon/Metal snapshot through `./scripts/cargo.sh`, with Apple Clang environment as above:

- `test --offline --lib`: 8 passed; independent source-pixel fixtures/round-trips/crop/UV mappings cover all 256 roll/photo orientation combinations and reversible roll turn actions.
- `test --offline --features editor --lib`: 91 passed; actual Roll button/reset/disabled-pick events, unchanged crop/photo/grade, inherited new-source turns, temporary-reference return, viewport/crop handle mappings and immutable export snapshots are covered.
- `test --offline --features project --test project --test film_project`: 13 passed; literal legacy flip-only documents, all roll-turn/flip save/reopen combinations, exact old representation, older-reader rejection and malformed/unknown turns are covered.
- Focused `--features editor --test viewer oriented_presentation_matches_exact_source_pixel_mapping` passed across all 256 combinations per target in full/cropped/extended-source mappings. Existing presentation tolerance is unchanged.
- `test --offline --features export --test geometry_export --test export`: 5 passed; independent full/cropped TIFF and half/full AP0 EXR layouts now cover all four roll turns followed by photo exceptions.
- Formatting, strict all-target editor Clippy and debug editor build passed. An initially unboxed `Ready` test message failed to compile; repaired the fixture to match the existing message API before the successful runs. No runtime fallback or gate change.

No real-Hasselblad/onscreen acceptance, main/flat-field integration, commits or pushes are claimed. Straightening and its sampling remain the next feature slice.

### Straightening and actual-coverage checkpoint

The user reported the roll-turn build working and authorized straightening. Implemented:

- Source-plane `Crop.angle` about the source centre, strict four-corner validation, old zero-angle serialization/defaults and source-ID-owned save/load. Exact orientation transforms the stored frame/basis together, preserving selected source coverage under turns/flips. Full-frame normalization compares actual framing, not its bounding box.
- Outside-frame dragging plus signed displayed-angle entry/reset in the transient crop draft. Straightening shrinks about the original pivot; if even a one-pixel frame cannot fit there, the proposal is rejected. Move/resize remains frame-aligned, constrained and cancellable. Snapped extents retain the opposite resize anchor. File/export/picker/grade blocking is unchanged.
- Shared `Coverage` convex-cell intersection weights and `FrameRaster` destination footprints. GPU-only settled reconstruction processes bounded RGB16 tiles through unchanged processing/ACES before averaging linear target RGB. Exact geometry retains its old rectangular kernel. Provisional/thumbnail proxies stay bounded approximate affine presentations and geometry-independent caches.
- Composed TIFF adapters batch contributing source pixels in bounded strips, render each through ACES, then filter linear target RGB before encoding. AP0 EXR filters signed/HDR scene-linear data without ACES/clamping. Production and diagnostic raster APIs retain destination safety. Exact composed wrappers are covered directly by legacy full/cropped reopen expectations.
- Canvas-aligned picker coverage in native frame units; compensated full-resolution means and weighted neutral fitting. Original exact source rectangles retain their existing path. Matching native diagnostics use positive-contributing-cell row spans, including boundary cells but excluding bounding-box-only sites; the RAW bridge and CFA/margin indexing are unchanged. Dmin/Dmax bypass crop/angle with the same source-anchor/magnification return policy; Gamma/WB retain framing/grade.

Fresh verification snapshot in this worktree on local Apple Silicon/Metal, through `./scripts/cargo.sh` with `CC=/usr/bin/clang CXX=/usr/bin/clang++`:

- `test --offline --lib`: 13 passed (coverage masks/spans, source-basis round trips, valid corners, extreme aspect/minimum-size/pivot constraints and unchanged exact mappings).
- `test --offline --features editor --lib`: 100 passed, including outside drag/reflected turns, rotated move/resize/opposite anchors, transactional angle Apply/Cancel, full-resolution canvas coverage and anchored uncropped picker return.
- `test --offline --features project --test project --test film_project --test processing --test film_response`: 13 project and 28 scalar processing/response tests passed, including zero-angle omission, rotated save/load/invalid state and weighted-vs-repeated neutral fitting.
- `test --offline --features export --test geometry_export --test export`: 6 passed, including production exact composed wrappers, rotated TIFF filtering **after** ACES versus the explicitly wrong before-ACES average, and half/full signed/HDR AP0 EXR reopen.
- `test --offline --features editor --test affine_resample`: 1 passed across seven angles (−89°…89°), reflected geometry, native/reduced/one-pixel outputs, fractional centres and tile seams; linear signed/HDR values match scalar cell coverage within `2e-5` on these compact fixtures. Three focused existing `--test viewer` checks also passed: all 256 exact orientation combinations, tiled rectangular fractional averaging and CPU/extended-sRGB presentation agreement, with unchanged gates.
- Formatting, strict all-target editor Clippy, debug editor build and `git diff --check` passed. Markdown whitespace/relative-link/anchor checking passed across 10 changed/new documents and 87 links. Binary remains `target/debug/antitone` in this worktree; no release build or large photographic/device/resource acceptance was run.

Initial fixture/API compile errors were repaired without gate changes. A resize test exposed unrounded midpoint movement; fixed the shared snapped-centre calculation. An extreme-aspect test exposed adding the one-pixel minimum after shrinking; the fitting constraint now includes the minimum and rejects an impossible pivot instead of moving it. A full-frame picker test caught inverse round-off producing negative numeric-control ranges; UI frame bounds now snap only arithmetic round-off at integer edges, leaving sampling coverage unchanged. The new-angle acceptance replaces the old test's unknown-angle expectation with an unknown-basis check; zero-angle representation remains covered.

One independent read-only review of the new straightening slice found only a missing exact composed-wrapper regression test; addressed by routing the existing exact fixtures through the production wrappers. It did not perform real-source/native-diagnostic/onscreen/resource acceptance and did not integrate flat-field. No repeat review, commits or pushes.

No agent-run real-RAW/native-diagnostic or photographic/onscreen straightening acceptance is claimed. Compact synthetic/Metal checks do not establish broad-device, memory-pressure or physical-monitor acceptance.

### Crop UX redesign checkpoint

The user reported disorganized controls, shrink/expand zoom snapping and an object-resize-looking frame, then approved a dedicated top panel, stable full-source edit view and crop marquee/thirds styling. Lightroom's documented aspect/reposition/straighten/overlay workflow informed this design; the available documentation did **not** establish its exact automatic zoom/refit behavior. This is not claimed as an exact Lightroom zoom clone.

- At this checkpoint (before the unified Composition panel below), compact **Crop & straighten** entry above both tabs opens a pinned, grouped Aspect/Angle panel with slider/numeric/reset, portrait/landscape swap, optional thirds, dimensions and Reset / Cancel / Apply. Other edit/output actions remain blocked; tab switching does not change the draft/pipeline. The compact inactive entry preserves grade-control reachability at existing narrow heights.
- `Viewport::layout_crop` Fits the full active source projection on entry/explicit Fit source. Painted layouts retain physical scale/source placement through resize and release; image-under-frame moves and straighten retain the screen frame/pivot. Apply Fits the accepted composition; Cancel restores the exact starting viewport. Explicit zoom remains available. Offscreen draft frames do not construct a settled raster or schedule refinement.
- Contrasting thin border, corner brackets and short edge ticks replace filled square handles; hit targets/constraints remain. Optional thirds follow the frame and change no processing, measurements or exports. Durable geometry, sampling, grade and RAW development are unchanged.

Fresh local Apple Silicon/Metal snapshot from this worktree through `./scripts/cargo.sh`, with `CC=/usr/bin/clang CXX=/usr/bin/clang++`:

- `test --offline --features editor --lib`: **105 passed**. New regressions cover full-source reachability/one-drag expansion across turns, no release zoom/placement snap, rotated resize and numeric-angle screen pivot, pinned controls at 270/315-point widths, brackets/thirds geometry independence, actual 100%/Fit source events with an offscreen frame, and Apply Fit/Cancel restoration. Existing dirty-state, scoped reset, picker/viewport, file and grade-control reachability tests pass.
- Formatting, strict `clippy --offline --features editor --all-targets -- -D warnings`, debug `build --offline --features editor --bin antitone` and `git diff --check` passed. Markdown whitespace/relative-link/anchor validation passed across 10 changed/new documents and 87 links. No export/shader changes in this UX slice; preceding reconstruction/export snapshots above are historical, not rerun here.
- Initial formatting caught a preset tuple delimiter. The first library run exposed grade/response controls clipped by an oversized inactive crop card; reduced it to a compact header and reran the existing assertions unchanged. No numerical gate was weakened.

No photographic/onscreen crop UX acceptance, release build, main/flat-field integration, commits, pushes or sample changes are claimed.

### Composition action/layout checkpoint

The user reported the crop redesign “much better”, identified misleading checked flip axes after turns, and approved momentary flip buttons, one top photo Composition panel and Scan orientation first in Roll controls.

- Composition contains photo Turn left/right, Flip horizontal/vertical, Crop and crop-only reset. Its scoped header reset clears photo orientation/framing only. Crop mode uses the same panel for its existing cancellable draft; orientation actions remain unavailable until Apply/Cancel. Scan orientation retains its independent reset and acts before photo exceptions.
- Follow-up: the user rejected seeing both scopes together. Tabs now precede the controls; Photo shows only pinned Composition, Roll shows only pinned Scan orientation. Tab switching preserves draft, accepted state, physical viewport and preview signature; crop controls return on Photo without cancelling the draft.
- Buttons toggle the existing stage's displayed-axis flip fields; turn actions still swap axes. No geometry, persistence/schema, grade, sampling or source changes. The pre-action orientation snapshot now precedes the top panel so its accepted photo actions continue to Fit correctly.
- Local Apple Silicon/Metal snapshot through `./scripts/cargo.sh` with `CC=/usr/bin/clang CXX=/usr/bin/clang++`: `test --offline --features editor --lib` **108 passed** after the tab-visibility follow-up. Actual tab-click tests verify mutually exclusive panels and unchanged image/viewport/pipeline/draft; narrow layout checks cover 270/315-point controls. Added actual UI action tests independently verify displayed pixel permutations, crop/grade/roll isolation, repeatable flip actions, busy/picker/draft blocking and source-owned persistence. Existing normal/narrow layout checks now assert Composition and Scan orientation ordering; scoped resets and Apply/Cancel remain covered.
- Formatting, strict all-target editor Clippy, debug editor build and `git diff --check` passed; Markdown whitespace/relative-link/anchor validation passed across 10 documents and 87 links. The numeric-save fixture now scrolls to Exposure beneath the taller pinned panel; its pending-text/snapshot assertions are unchanged. No processing gates were weakened. Export/shader checks are the preceding historical snapshots, not rerun for this UI-only slice.

No new onscreen/photographic acceptance, release build, main/flat-field integration, commits, pushes or sample changes are claimed.

### Main integration approval

The user approved committing this checkpoint, rebasing onto current main `5efb302` (native-camera inversion/model/schema 4, following flat-field `c0b664a`), integrating all existing spatial features with the shared coordinate system, verifying and updating main locally. This supersedes the earlier non-committing integration limit. No push is authorized. Main has unrelated local `docs/feedback.md` and untracked `.pi/` changes; preserve them and leave other worktrees untouched.

Integration must retain main's accepted native-camera processing and camera basis/model/schema 4, not restore AP0 inversion or schema-3 migration. Adapt weighted fitting/coverage diagnostics, corrected preview/refinement/thumbnails/export snapshots and field-support editing to canonical original-source mappings. Reconcile GPU field nearest-centre geometry with `SourceRaster`; verify corrected and uncorrected composed paths with independent fixtures. Preserve bounded RGB16 tiles, ACES/output rules and numerical gates.

### Integrated native-camera / flat-field checkpoint

- Weighted coverage measurements use corrected native-camera RGB, not AP0. Roll-neutral fitting retains the actual response and `inverse(camera_to_AP1) * white` target; WB processes each contributing pixel before fitting in AP1. Runtime correction adjusts the calibration floor once; persisted RGB/base floor stay unchanged. Exact rectangular picks retain their existing path, and rotated diagnostics visit only positive-contributing original-source spans.
- GPU `SourceGeometry` delegates nearest-centre sampling to canonical `SourceRaster`, retaining shader/range validation. Sparse previews, thumbnails and every refinement tile evaluate the field at original source pixel centres. Frame bounds only limit iteration; gains are never looked up in crop/frame-local coordinates. The equivalent shader adapter remains separately verified. Padded partial tiles contribute no outside-bound pixels.
- Flat-field support editing temporarily uses untransformed full-source composition and integer source rectangles through the shared canvas mapping. It cannot start during a crop draft; invalid nonrectangular support is rejected before a worker command. Failure/Cancel restores accepted composition, working view and inspection state and clears the temporary canvas layout. The wrapped picker header from main is preserved.
- Preview/refinement, thumbnail field/camera identities and export snapshots retain native-camera model/schema 4 and accepted correction. Composed exports correct/process each original pixel before filtering; TIFF filters after unchanged ACES, EXR retains signed/HDR AP1→AP0. Persistence round-trip covers rotated composition and an unchanged accepted field/map snapshot together. Old AP0 schemas/models still reject, without migration.

Fresh local Apple Silicon/macOS/Metal snapshot from this worktree through `./scripts/cargo.sh`, with `CC=/usr/bin/clang CXX=/usr/bin/clang++`:

- `test --offline --all-features`: **205 passed, 3 explicitly ignored**; the library subset is 126 passed/1 ignored. New nonuniform corrected/uncorrected rotated-tile tests cover nonzero origins, fractional pivots, reflections/turns, native/reduced/one-pixel outputs, tile seams/padded tails and both targets within the unchanged `2e-5` gate. Synthetic TIFF/AP0 EXR reopen tests cover exact/rotated corrected and uncorrected native-camera processing. Weighted fitting verifies unequal native camera white. Existing native-camera, project, response/GPU, presentation and large-Retina buffer tests pass.
- `test --offline --lib`: **22 passed** without desktop features. Formatting, strict `clippy --offline --all-features --all-targets -- -D warnings` and debug editor binary build pass. Final diff/whitespace inspection passes; Markdown relative links/anchors pass across 10 changed documents and 91 links.
- Initial compile failures were stale geometry/source/view fixtures and API adapters; repaired to current camera/schema/coverage interfaces. The first combined run caught a lost wrapped picker header and an incomplete synthetic flat-preview fixture; restored main's layout behavior and completed the fixture without weakening assertions. One integration-specific independent static review found no actionable defects; it did not rerun tests or review the prior geometry internals again.

This closes the existing-feature spatial audit: metadata/CFA indexing intentionally stays native; source proxies/tiles/fields share original-source mappings; transient drag constraints remain separate; viewport/picks use composition/coverage; presentation/thumbnails use shared affine adapters; refinement and exports use shared frame footprints. No separate colour/orientation equations or new retained wide source cache were introduced.

Ignored real-RAW/train/editor/full-size export acceptance was not run. No new photographic/onscreen, physical-input/monitor, broader-device/memory-pressure or release-build acceptance is claimed. Dense guides remain deferred; samples and unrelated worktrees are unchanged. No push.

**Delivery:** local checkpoint/rebase/main integration is authorized; preserve unrelated `docs/feedback.md` and `.pi/` changes and do not push. **Next action after delivery:** agree the photographic straightening/native-diagnostic acceptance exercise. This is a separate check, not an automatic new implementation request.
