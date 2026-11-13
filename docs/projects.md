# Projects, identity and saved state

A project is a saved description of an accepted grade, not a copy of the RAW photographs or a cache of rendered pixels. Reopening it should restore the same processing choices without quietly estimating them again.

New projects and Save As use the `.antitone` extension; the document contents are JSON. Supported `.json` and `.antitone.json` projects remain openable, and Save updates them in place.

## Separate sources from the filmstrip

Each registered source has a stable ID and owns its adjustment layers, composition and committed Dust repairs. Filmstrip order is a separate list of those IDs. Removing a photo deletes its source record and analysis support; adding the RAW again registers a new source. Every source is an ordinary filmstrip photo; flat-field provenance is independent of this collection. Scan-based projects also retain the shared camera basis and accepted illumination correction. [Selection/removal](editor.md#select-and-remove-roll-photos) is controlled by the editor.

Estimating a flat field stores its numeric map and reference filename, without importing the reference as a photo. Dmin/Dmax themselves are RGB values: their continued use does not depend on reopening the photo from which they were picked. Paths locate external files — a path or stable ID is not proof of photographic capture identity.

## Save accepted choices, not working machinery

The project retains its input interpretation and stage-specific settings, Roll grading and adjustment layers, source/development identity, orientation/crop, committed repair instructions, accepted correction, source-owned Analyze support and output settings — the choices that determine its processing. [Spatial metadata](spatial-metadata.md) is reused within the open document and regenerated on demand after reopening. Preparation does not dirty the project or enter undo history. Auto-crop saves its explicitly applied result; Analyze saves accepted support and grading.

Successful interpretation-specific developments and [Digital starting exposures](processing.md#digital-starting-brightness) accompany Save as recorded per-photo facts. Save captures only resolved values; unvisited interpretations stay lazy. Development facts retain the WB, decoder and camera conversion needed for cold acquisition, separately from accepted controls and undo versions. Open restores these values as metadata; consumers validate their processing domain when used. Pixel eviction, file-revision changes and decoder changes do not re-meter a saved baseline: recalculation would add another exposure change rather than restore the original pixels. Source compatibility checks still apply. Photos without a recorded value resolve one on demand; malformed or unsupported metadata is discarded.

It does not retain active/multi-selection, canvas navigation, source-inspection WB/exposure, picker return context, active gamma-pivot evidence, tool drafts, GPU objects or caches. This distinction defines dirty state: accepted project changes are compared with the last successful Save/Open snapshot, and session-only [undo/redo](editor.md#undo-and-redo) preserves that baseline. Looking around, selecting a photo or cancelling a draft does not make a project modified; Auto-crop margin changes are drafts until Apply and never dirty project geometry. Export scope, filenames, format and target overrides belong to the export operation; the last destination folder is an application preference. Choosing another TIFF target in Export leaves the project's Rendering target unchanged.

Coupled grain/image-MTF and independently enabled halation belong to project output state and undo history. Each record carries its model contract and actual controls, including custom values while disabled. Grain stores Size, image sigma, Amount and Colour; the shared enable and ratio-preserving Size adjustment keep image response and grain coordinated. Presets populate these values without a saved preset identity. The UI derives an exact match or shows Custom.

## Embed accepted colour assets

The project has independent optional `lut` and `film_and_paper` assets, evaluated in that order. Each embeds its resolved LUT samples, domain, dimension, contract and provenance—not a source pathname or instructions requiring regeneration. Imported and provider-generated assets use the same numeric representation. Film & paper retains negative/media IDs and upstream/generator revisions as provenance, including when those IDs are absent from the current catalogue. Reopening executes the embedded numbers. Catalogue changes, deleted imported files and evicted derived caches cannot upgrade a saved appearance.

Pending preparation, errors, the session loaded-LUT list and GPU caches are transient; Save embeds only accepted assets. Malformed or unsupported assets reject. The two slots validate their source roles rather than silently swapping imported and material-derived treatments.

## Source-owned adjustment layers

The format stores each source's ordered adjustment layers with global, linear or radial coverage. New photos start with one ordinary global layer; an empty stack is valid. Stable IDs and a retained allocator support renaming/reordering without changing identity; deletion does not reuse IDs. Strict validation rejects unknown fields, malformed geometry and unsupported processing instead of silently dropping a grade. Immutable processing snapshots contain ordered layer values — not names, IDs or allocator history — so metadata edits participate in saved/dirty state without invalidating GPU processing.

## Source-owned Dust repairs

Each source retains its committed binary source-coordinate masks, stable repair IDs and non-reusing allocator, enabled state, provenance and a stored donor offset. Removing the photo removes its repairs with the rest of the source record; undo restores them. Reimporting the same RAW creates a new source without the old repairs. Composition changes do not rewrite mask coordinates.

Masks use exact maximal row-major runs inside the unchanged declared rectangle; serialization remains that run sequence, and shared immutable buffers are conservatively charged to each owner by logical retention budgets. Validation shares the canvas's per-repair bounds, rejects overlap and invalid donor geometry, and checks disabled records too. There is one project file without a document-wide size or repair-count budget; buffered I/O and same-directory failure-atomic writes retain destination safety.

The explicit repair contract identifies both the healer and its ungraded-AP1 input stage, so saved instructions reproduce the same result for the same source and interpretation. Unknown contracts reject rather than silently disabling or reinterpreting repairs. Each repair stores one donor offset, shared by its components. Reopening heals deterministically from that offset and never searches; the best-effort donor search runs only when a support is created or changed, and its result is stored. Evaluated pixels and GPU resources are never stored. Accepted repairs remain saveable before healing; strokes awaiting donor search are transient. Open restores accepted instructions lazily rather than healing every offscreen source before admission. Completed TIFF/AP0 EXR consume the accepted recipe independently of viewing history, and rendering readiness never changes dirty state.

## Compatibility is about the processing basis

Calibration makes sense only in the camera basis and development in which it was defined. Negative therefore retains an explicit scan setup: one camera model and camera-to-AP1 matrix, plus an optional accepted flat-field map. Its sources share that basis; Digital allows mixed cameras. Each source records its registered development and mode, independently of the project's current input intent. Negative requests unity WB; Digital initially requests each source's as-shot WB. Acquisition reuses a recorded development for the requested mode or resolves it lazily, without relabelling the registered baseline. Open and cold redevelopment reuse known balances instead of selecting new decoder defaults.

Switching stages accepts intent without reading RAWs or waiting for GPU resources. It clears stage-specific scan setup and correction; discarding Negative settings or an accepted correction requires confirmation. Shared grades, crops and repairs remain accepted. Each consumer acquires its captured photograph and interpretation; missing, corrupt or incompatible sources fail individually without rolling back the stage. A new Negative basis is established lazily and checked at consumption. New projects start in Negative; Import can select Digital for an unconfigured batch when a newly inspected source is incompatible. Partial anchors, nondefault interpretation settings or an accepted correction prevent that automatic choice.

The current format is **schema 20 with processing model 6**. This unreleased format has no migrations; unsupported schemas/models and unknown fields reject.

Reproducibility is best effort. Development stores recorded `white_balance`, an explicit LibRaw/DNG SDK backend and diagnostic pixel-producer provenance, including the decoder release and development revision. Source records also describe geometry and camera conversion. Open restores these records unchanged. Acquisition uses the current decoder and checks compatibility when a photo is needed; diagnostic release differences alone do not reject it. Decoder fixes and upgrades can change pixels, and runtime cache identity distinguishes current pixel producers.

Already-applied capture WB and sampling-precision estimates stay with runtime decoder metadata. Lens/opcode fingerprints and fixed colour-space labels are not saved. [Calibration and inversion](processing.md#two-anchors-define-the-response) operate on camera light and measured RGB anchors.

RAWs are assumed immutable; Antitone does not hash them to establish photograph identity. Best-effort file revisions captured at Import/Open validate disposable persisted caches. Missing or corrupt photos remain in the opened document with their IDs, paths, order and settings, even when every source is unavailable. Acquisition failures belong to those photos. Source records and filmstrip order must describe the same photos; nonordered records reject. Relinking UI is not implemented.

## Add photos

[Import](../src/editor/files.rs) owns a scoped document edit while it inspects new RAW headers and validates the private batch. Acceptance adds one undo entry; failure leaves accepted state unchanged. Existing paths are ignored, including repeats within a batch. Adding photos leaves the active visit and its loading work in place. An empty document selects its first photo through ordinary nonblocking loading; subsequent pixel or GPU failures stay local to that photo.

## Prepare before replacing

New installs an empty document directly. Open reads and validates project metadata privately, then replaces the document and resets its history and saved baseline. An unreadable or invalid project preserves the previous document and display, even after Discard. Successful replacement retires the old document and visit; physical workers retain their captured inputs through cleanup. The first photo uses the same nonblocking loader as later navigation. Its pixels, processing-domain checks and viewer resources do not gate metadata acceptance. Resource limits and renderability are checked by consumers; [architecture](architecture.md) explains worker ownership.

Save captures accepted choices and acquired facts, validates format and data integrity, then writes a same-directory temporary with no-clobber or explicit replacement semantics. It requires neither source services nor a GPU: device capacity and renderability do not limit the saved JSON. The document owns the write and accepts exactly its written baseline, preserving later edits. Save As rebases live and history locators only after success. A file-action caller independently awaits the result before continuing; retiring that caller does not cancel the document's write. New/Open/Close use Save/Discard/Cancel protection. File actions are unavailable during unfinished control edits; [edit completion](editor.md#numeric-edits-own-their-input) accepts the intended values before a subsequent Save captures them. This protects an existing file from ordinary operation failures, not every crash or power-loss scenario.

## Working on this area

[Project types](../src/project.rs) own format, validation and compatibility identifiers. [Document](../src/editor/document.rs) and [file actions](../src/editor/files.rs) own staged editor transitions. [Preferences](../src/editor/preferences.rs) own independent UI persistence.

Exact serialization details belong to those types and [project tests](../tests/project.rs), [repair persistence tests](../tests/dust_project.rs), [camera compatibility tests](../tests/camera_project.rs), [film tests](../tests/film_project.rs) and [correction tests](../tests/flat_field_project.rs). Use [development checks](development.md#feature-specific-checks) before proposing recovery or source-identity changes.