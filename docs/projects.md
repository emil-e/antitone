# Projects, identity and saved state

A project is a saved description of an accepted grade, not a copy of the RAW photographs or a cache of rendered pixels. Reopening it should restore the same processing choices without quietly estimating them again.

New projects and Save As use the `.antitone` extension; the document contents are JSON. Supported `.json` and `.antitone.json` projects remain openable and Save updates them in place. Save As suggests the `.antitone` name without changing the format or processing schema.

## Separate sources from the filmstrip

Each registered source has a stable ID and owns its Photo settings, composition and committed Dust repairs. Filmstrip order is a separate list of those IDs. Removing a photo from that list does not erase its record or invalidate a stored calibration value. Even an empty roll retains its camera basis, calibration, correction and Roll settings. Adding a previously removed compatible RAW restores its existing source record and Photo settings; [selection/removal](editor.md#select-and-remove-roll-photos) is controlled by the editor.

A bare-light reference can be registered without appearing as a photograph. Dmin/Dmax themselves are RGB values: their continued use does not depend on reopening the photo from which they were picked. A saved flat-field map similarly remains usable without its nonordered reference; editing it is a different operation.

Paths locate external files. Relative paths resolve beside the project document, and Save As rebases them for its new location. A path or stable ID is not proof of photographic capture identity.

## Save accepted choices, not working machinery

The project retains calibration, Roll and Photo grading, source/development identity, orientation/crop, committed repair instructions, accepted correction, source-owned Analyze support with shared/exception provenance and compact retained frame evidence (and older shared explicit picture geometry) and output settings. These are the choices that determine its processing.

Retained frame measurements are source metadata, not instructions to install a crop. The explicit [Auto-crop action](editor.md#auto-crop-is-an-explicit-accepted-edit) interprets them independently of sampling approval and saves its result as the source-owned crop/angle. The current measurement lifetime still follows accepted sampling areas: editing picture statistics clears the automatic measurements, and resetting sampling support removes them. Independently requested/transient evidence is follow-up work, not an implicit new saved acceptance state.

It does not retain active/multi-selection, canvas navigation, source-inspection exposure, picker return context, tool drafts, the live Auto-crop target set/original proposals/margin percentage/overlay visibility, per-photo tool warning reasons or drag gesture, GPU objects or caches. Auto-crop margin changes are drafts until Apply; they never dirty or temporarily replace project geometry. Save/Export wait for Apply or Cancel, as with manual Crop. Reopening restores applied crops, not an adjustable batch. Full-scan inspection and explicit filmstrip draft framing never become stored/output composition. Inspector expansion and scope/filmstrip sizing are application preferences saved independently; they are not part of a photographic grade.

This distinction defines dirty state. Accepted project changes are compared with the last successful Save/Open snapshot. Session-only [undo/redo](editor.md#undo-and-redo) preserves that baseline: returning to the written project becomes clean, while Save As rebases historical source locators without undoing the destination change. Looking around, selecting a photo or cancelling a draft does not make a project modified. Analyze review areas remain drafts until a successful analysis installs them with the grade. Cancelling or failing the operation saves neither. Applying an older shared explicit picture area is still an accepted geometry change without an immediate reanalysis.

## Embed the accepted look, not a catalog reference

The project optionally owns one immutable [creative-look asset](colour-and-output.md#creative-looks-precede-output-rendering). Both built-ins and imports store resolved numeric samples with their domain, dimension, colour/interpolation contract, content identity and provenance. This is bounded compressed numeric data, not `.cube` text, a source pathname, GPU storage or the whole catalog. The streaming file reader bounds each decoded JSON string before materializing it, including the embedded base64 payload; source collections and numeric repair-run arrays have no document-wide cap. Missing look state in older supported projects means true None.

Built-in ID/version and generator identity explain origin; they are not instructions to regenerate on Open. Catalog upgrades, missing source files and deleted derived caches cannot silently change a saved look. Choosing a newer built-in is an explicit accepted edit. Unknown/old built-in provenance can reopen from valid embedded samples without retaining its old model implementation. Malformed, corrupt or unsupported assets reject instead of dropping the look; older strict readers likewise reject the new field rather than displaying a different appearance.

Pending preparation, errors, the editor-session loaded-LUT comparison list and GPU caches are transient. Saving embeds only the accepted look, not unused comparison entries. Accepted selection/None participates in dirty state; cancelled/failed preparation does not. Save As carries the same numeric asset without rebasing an imported filename. Newly resolved built-ins take their names from the catalog; opening older embedded assets preserves their stored names and samples while the editor displays the current catalog name by source ID. Export retains the accepted immutable snapshot even if selection changes while writing. The embedded `input_range` policy is `clip`: finite values outside the stored window use its boundary, without rewriting samples or adjusting the grade. Earlier prototype look records omitted that field and load with the same clipping policy; their numeric content identity is unchanged. Unknown policies reject. Nonfinite data or GPU failures can still report a rendering error without changing the stored selection.

## Source-owned local adjustments

The format optionally stores an ordered collection of source-anchored local adjustments. Stable IDs and a retained allocator support renaming/reordering without changing identity; deletion does not reuse IDs. Strict contract/record validation rejects unknown fields, malformed geometry and unsupported processing instead of silently dropping a grade.

Immutable processing snapshots contain Global settings and ordered local values, not names, IDs or allocator history. Metadata still participates in saved/dirty state, but does not invalidate GPU processing. Editing-target selection, handles, overlays and mask drafts remain transient. Normal Open/Save/source/correction/Analyze/export admission checks the complete document against the editor GPU before accepting it, including offscreen/non-filmstrip grades and inactive records. Nonempty collections require a calibrated processing basis and a GPU device; unsupported candidates are rejected without replacing accepted state. Preflight retains no offscreen images or published grades. [Broader photographic, interaction and device acceptance](plans/local-adjustments.md) remains separate.

## Source-owned Dust repairs

Each source retains its committed binary source-coordinate masks, stable repair IDs and non-reusing allocator, enabled state, provenance and automatic/manual donor policy. Deleting or resetting repairs retains allocator history, including in an empty collection. Filmstrip removal does not erase repairs; compatible reimport restores them. Composition changes do not rewrite mask coordinates.

Masks use exact maximal row-major selected-index runs inside the unchanged declared rectangle. In memory, immutable run buffers are shared by recipes and history; replacing a mask does not alter an older snapshot. Serialization remains the same run sequence, and logical retention budgets conservatively charge the shared payload to each owner. Holes and disconnected supports survive without contour approximation or decoded offscreen rasters. Validation shares the canvas's per-repair marked-pixel/processing-area bounds, rejects overlap and invalid source/manual-donor geometry, and checks disabled records too. There is one project file without a document-wide size, repair-count or decoded-mask budget; buffered I/O and same-directory failure-atomic writes retain destination safety.

The explicit repair contract freezes both the healer and its ungraded-AP1 input stage. Unknown contracts reject, not silently disable or reinterpret repairs. Automatic donors remain automatic: reopening deterministically recomputes donor metadata and repaired f32 values for the same immutable source and interpretation. Evaluated pixels, resolved donors, failures, requests and GPU resources are never stored. Pending or no-donor repairs remain saveable instructions; Open restores them lazily through the existing backend rather than healing every offscreen source before admission.

Released accepted edits participate in ordinary dirty comparison and Save/Discard/Cancel protection. Selection, cancelled drafts and evaluation do not. Successful Save/Save As baselines only the written snapshot, preserving later edits and compatible evaluations. Failed Open preserves the live document and display; accepted replacement retires old work. Existing whole-document development and local-grade admission remains authoritative, not a requirement to render every saved repair.

Completed TIFF/AP0 EXR and proper positive thumbnails consume the accepted source recipe, including repairs, independently of viewing history. Export prepares applicable repairs asynchronously and fails safely if they cannot be applied; pending/failed thumbnail work is explicitly provisional/unavailable. Save/Open still preserve instructions without forcing offscreen healing, and rendering readiness never changes dirty state. Project-wide [undo/redo](editor.md#undo-and-redo) restores instructions rather than evaluated pixels; [broader photographic/resource acceptance](plans/dust-repair.md) remains separate.

## Compatibility is about the processing basis

Calibration makes sense only in the camera basis and development in which it was defined. A roll therefore retains its camera model and camera-to-AP1 matrix, along with fixed development WB and backend/development identity. Every source and reference must be compatible; removing old records does not reset the retained basis.

The supported format is **schema 6 with processing model 4**. Unsupported schemas, repair contracts, old AP0 grades and incompatible development are rejected, not migrated or reinterpreted. Required per-source repair state includes empty collections; there is no schema-5 migration or repairs-disabled recovery path. Missing supported optional grade fields preserve existing appearance through explicit defaults; unknown grade fields reject rather than being silently discarded. Open rejects an incompatible saved output-transform identity and preflights the saved processing against current GPU domains. Exact fields and identifiers belong to the deserializers and tests.

RAW contents are not hashed. Changing bytes alone does not invalidate a grade if the source remains loadable and development-compatible. Cheap file revisions tell disposable caches when to refresh; they do not establish capture identity or photographic equivalence. [The identity decision](decisions/no-raw-content-hashing.md) explains why those questions remain separate.

Missing or incompatible **ordered** sources reject Open. Unavailable nonordered records remain preserved without blocking it. Recovery and relinking/source-change UI are not implemented.

## Prepare before replacing

Open validates the candidate, checks sources and prepares the first photo before replacing the live document. Candidate preparation holds the accepted working view rather than clearing its support; only accepted replacement retires that source authority. Failure retains the previous document and display—even if the user had agreed to discard its unsaved changes.

New/Open/Close use Save/Discard/Cancel protection. Pending numeric edits are processed before file actions. A cancelled or failed Save prevents the requested transition, and stale worker replies cannot complete a different document's operation.

Save validates before writing a same-directory temporary and installing it with no-clobber or explicit replacement semantics. The saved baseline changes only after success. This protects an existing file from ordinary operation failures, not every crash or power-loss scenario.

## Working on this area

[Project types](../src/project.rs) own format, validation and compatibility identifiers. [Document](../src/editor_document.rs) and [file actions](../src/editor_files.rs) own staged editor transitions. [Preferences](../src/editor_preferences.rs) own independent UI persistence.

Exact serialization details belong to those types and [project tests](../tests/project.rs), [repair persistence tests](../tests/dust_project.rs), [camera compatibility tests](../tests/camera_project.rs), [film tests](../tests/film_project.rs) and [correction tests](../tests/flat_field_project.rs). Use [development checks](development.md#feature-specific-checks); see the [unfinished persistence work](plans/durable-projects.md) before proposing recovery or source-identity changes.
