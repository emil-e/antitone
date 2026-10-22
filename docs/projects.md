# Projects, identity and saved state

A project is a saved description of an accepted grade, not a copy of the RAW photographs or a cache of rendered pixels. Reopening it should restore the same processing choices without quietly estimating them again.

New projects and Save As use the `.antitone` extension; the document contents are JSON. Supported `.json` and `.antitone.json` projects remain openable, and Save updates them in place.

## Separate sources from the filmstrip

Each registered source has a stable ID and owns its Photo settings, composition and committed Dust repairs. Filmstrip order is a separate list of those IDs: removing a photo from that list does not erase its record or invalidate a stored calibration value. Even an empty roll retains its camera basis, calibration, correction and Roll settings, and adding a previously removed compatible RAW restores its existing source record. [Selection/removal](editor.md#select-and-remove-roll-photos) is controlled by the editor.

A bare-light reference can be registered without appearing as a photograph. Dmin/Dmax themselves are RGB values: their continued use does not depend on reopening the photo from which they were picked. Paths locate external files — a path or stable ID is not proof of photographic capture identity.

## Save accepted choices, not working machinery

The project retains calibration, Roll and Photo grading, source/development identity, orientation/crop, committed repair instructions, accepted correction, source-owned Analyze support and output settings — the choices that determine its processing. The optional [spatial metadata](spatial-metadata.md) payload stores a reusable descriptive cache separately from accepted state: preparation alone does not dirty the project or enter undo history. Ready matching metadata can be saved without accepting Analyze or Auto-crop; invalid or unsupported derived records are discarded without changing accepted settings. Auto-crop still saves only its explicitly applied result.

It does not retain active/multi-selection, canvas navigation, source-inspection exposure, picker return context, tool drafts, GPU objects or caches. This distinction defines dirty state: accepted project changes are compared with the last successful Save/Open snapshot, and session-only [undo/redo](editor.md#undo-and-redo) preserves that baseline. Looking around, selecting a photo or cancelling a draft does not make a project modified; Auto-crop margin changes are drafts until Apply and never dirty project geometry. Export scope, filenames, format and target overrides belong to the export operation; the last destination folder is an application preference. Choosing another TIFF target in Export leaves the project's Rendering target unchanged.

## Embed the accepted look, not a catalog reference

The project optionally owns one immutable [creative-look asset](colour-and-output.md#creative-looks-precede-output-rendering). Built-ins and imports both store resolved numeric samples with their domain, dimension, contract and provenance — not `.cube` text, a source pathname or a catalog reference. Catalog upgrades, missing source files and deleted derived caches therefore cannot silently change a saved look; choosing a newer built-in is an explicit accepted edit.

Pending preparation, errors, the session loaded-LUT comparison list and GPU caches are transient; saving embeds only the accepted look. Malformed, corrupt or unsupported assets reject instead of dropping the look, and older strict readers reject unknown fields rather than displaying a different appearance.

## Source-owned local adjustments

The format optionally stores an ordered collection of source-anchored local adjustments. Stable IDs and a retained allocator support renaming/reordering without changing identity; deletion does not reuse IDs. Strict validation rejects unknown fields, malformed geometry and unsupported processing instead of silently dropping a grade. Immutable processing snapshots contain Global settings and ordered local values — not names, IDs or allocator history — so metadata edits participate in saved/dirty state without invalidating GPU processing.

## Source-owned Dust repairs

Each source retains its committed binary source-coordinate masks, stable repair IDs and non-reusing allocator, enabled state, provenance and automatic/manual donor policy. Filmstrip removal does not erase repairs; compatible reimport restores them, and composition changes do not rewrite mask coordinates.

Masks use exact maximal row-major runs inside the unchanged declared rectangle; serialization remains that run sequence, and shared immutable buffers are conservatively charged to each owner by logical retention budgets. Validation shares the canvas's per-repair bounds, rejects overlap and invalid donor geometry, and checks disabled records too. There is one project file without a document-wide size or repair-count budget; buffered I/O and same-directory failure-atomic writes retain destination safety.

The explicit repair contract identifies both the healer and its ungraded-AP1 input stage, so saved instructions reproduce the same result for the same source and interpretation. Unknown contracts reject rather than silently disabling or reinterpreting repairs. Automatic donors remain automatic: reopening deterministically recomputes donor metadata and repaired f32 values; evaluated pixels, resolved donors and GPU resources are never stored. Pending or no-donor repairs remain saveable instructions — Open restores them lazily rather than healing every offscreen source before admission. Completed TIFF/AP0 EXR consume the accepted recipe independently of viewing history, and rendering readiness never changes dirty state.

## Compatibility is about the processing basis

Calibration makes sense only in the camera basis and development in which it was defined. A roll therefore retains its camera model and camera-to-AP1 matrix, along with fixed development WB and backend identity; every source and reference must be compatible, and removing old records does not reset the retained basis.

The supported format is **schema 6 with processing model 4**. Unsupported schemas, repair contracts, old AP0 grades and incompatible development are rejected, not migrated or reinterpreted. Missing supported optional grade fields preserve existing appearance through explicit defaults; unknown fields reject. A source records its developed precision floor (including shading amplification), while project calibration retains the separate native one-code base floor; each corrected source records a lens-development identity, and a flat reference requires an exact lens-development match. DNG development has a separate SDK/backend contract with canonical fixed RGB WB and per-source interpretation identity — SDK rejection never falls back to LibRaw, and older LibRaw-developed DNG records are explicitly incompatible. Earlier development contracts reject explicitly: this is incompatibility detection, not migration.

Antitone does not hash the RAW file to establish photograph identity. Selected development metadata identifies its interpretation; cheap file revisions tell disposable caches when to refresh, but do not establish capture identity. Missing or incompatible **ordered** sources reject Open; unavailable nonordered records remain preserved without blocking it. Recovery and relinking UI are not implemented.

## Prepare before replacing

Preparing a candidate never changes worker-global source state — there is no active-source slot. Successful replacement retires the old owner/visit; failure preserves them, including the previous document and display even if the user had agreed to discard its unsaved edits. Open validates the candidate, checks sources and prepares the first photo before replacing the live document; the workflow/admission model behind this is [the architecture's](architecture.md).

Save captures accepted choices, validates them, writes a same-directory temporary and installs it with no-clobber or explicit replacement semantics. Its document-owned continuation accepts the written baseline without waiting for another paint. New/Open/Close use Save/Discard/Cancel protection, and pending numeric edits are processed before file actions. This protects an existing file from ordinary operation failures, not every crash or power-loss scenario.

## Working on this area

[Project types](../src/project.rs) own format, validation and compatibility identifiers. [Document](../src/editor_document.rs) and [file actions](../src/editor_files.rs) own staged editor transitions. [Preferences](../src/editor_preferences.rs) own independent UI persistence.

Exact serialization details belong to those types and [project tests](../tests/project.rs), [repair persistence tests](../tests/dust_project.rs), [camera compatibility tests](../tests/camera_project.rs), [film tests](../tests/film_project.rs) and [correction tests](../tests/flat_field_project.rs). Use [development checks](development.md#feature-specific-checks) before proposing recovery or source-identity changes.