# Projects, identity and saved state

A project is a saved description of an accepted grade, not a copy of the RAW photographs or a cache of rendered pixels. Reopening it should restore the same processing choices without quietly estimating them again.

New projects and Save As use the `.antitone` extension; the document contents are JSON. Supported `.json` and `.antitone.json` projects remain openable, and Save updates them in place.

## Separate sources from the filmstrip

Each registered source has a stable ID and owns its adjustment layers, composition and committed Dust repairs. Filmstrip order is a separate list of those IDs. Removing a photo deletes its source record and analysis support; adding the RAW again registers a new source. An accepted flat-field correction keeps its reference source as the one allowed nonordered record, marked reference-only until the correction is cleared. Scan-based projects also retain one project-level acquisition setup: the shared development basis. [Selection/removal](editor.md#select-and-remove-roll-photos) is controlled by the editor.

A bare-light reference can be registered without appearing as a photograph. Dmin/Dmax themselves are RGB values: their continued use does not depend on reopening the photo from which they were picked. Paths locate external files — a path or stable ID is not proof of photographic capture identity.

## Save accepted choices, not working machinery

The project retains its input interpretation and stage-specific settings, Roll grading and adjustment layers, source/development identity, orientation/crop, committed repair instructions, accepted correction, source-owned Analyze support and output settings — the choices that determine its processing. The optional [spatial metadata](spatial-metadata.md) payload stores a reusable descriptive cache separately from accepted state: preparation alone does not dirty the project or enter undo history. Ready matching metadata can be saved without accepting Analyze or Auto-crop; invalid or unsupported derived records are discarded without changing accepted settings. Auto-crop still saves only its explicitly applied result.

Completed [Digital starting exposures](processing.md#digital-starting-brightness) accompany Save as recorded per-photo baselines. Save captures only resolved values; unvisited photos stay lazy. Open restores them before normalized range validation. Pixel eviction, file-revision changes and decoder changes do not re-meter a saved baseline: recalculation would add another exposure change rather than restore the original pixels. Source compatibility checks still apply. Photos without a recorded value resolve one on demand; malformed or unsupported metadata is discarded.

It does not retain active/multi-selection, canvas navigation, source-inspection WB/exposure, picker return context, tool drafts, GPU objects or caches. This distinction defines dirty state: accepted project changes are compared with the last successful Save/Open snapshot, and session-only [undo/redo](editor.md#undo-and-redo) preserves that baseline. Looking around, selecting a photo or cancelling a draft does not make a project modified; Auto-crop margin changes are drafts until Apply and never dirty project geometry. Export scope, filenames, format and target overrides belong to the export operation; the last destination folder is an application preference. Choosing another TIFF target in Export leaves the project's Rendering target unchanged.

Film Emulation belongs to project output state and undo history. Its optional saved record carries the model contract and all controls, including custom values while disabled. Absent settings retain the clean plate; unsupported contracts and malformed values reject. Image recipes capture these settings with the persistent source identity used for the grain seed.

## Embed the accepted look, not a catalog reference

The project optionally owns one immutable [creative-look asset](colour-and-output.md#creative-looks-precede-output-rendering). Built-ins and imports both store resolved numeric samples with their domain, dimension, contract and provenance — not `.cube` text, a source pathname or a catalog reference. Catalog upgrades, missing source files and deleted derived caches therefore cannot silently change a saved look; choosing a newer built-in is an explicit accepted edit.

Pending preparation, errors, the session loaded-LUT comparison list and GPU caches are transient; saving embeds only the accepted look. Malformed, corrupt or unsupported assets reject instead of dropping the look, and older strict readers reject unknown fields rather than displaying a different appearance.

## Source-owned adjustment layers

The format stores each source's ordered adjustment layers with global, linear or radial coverage. New photos start with one ordinary global layer; an empty stack is valid. Stable IDs and a retained allocator support renaming/reordering without changing identity; deletion does not reuse IDs. Strict validation rejects unknown fields, malformed geometry and unsupported processing instead of silently dropping a grade. Immutable processing snapshots contain ordered layer values — not names, IDs or allocator history — so metadata edits participate in saved/dirty state without invalidating GPU processing.

## Source-owned Dust repairs

Each source retains its committed binary source-coordinate masks, stable repair IDs and non-reusing allocator, enabled state, provenance and a stored donor offset. Removing the photo removes its repairs with the rest of the source record; undo restores them. Reimporting the same RAW creates a new source without the old repairs. Composition changes do not rewrite mask coordinates.

Masks use exact maximal row-major runs inside the unchanged declared rectangle; serialization remains that run sequence, and shared immutable buffers are conservatively charged to each owner by logical retention budgets. Validation shares the canvas's per-repair bounds, rejects overlap and invalid donor geometry, and checks disabled records too. There is one project file without a document-wide size or repair-count budget; buffered I/O and same-directory failure-atomic writes retain destination safety.

The explicit repair contract identifies both the healer and its ungraded-AP1 input stage, so saved instructions reproduce the same result for the same source and interpretation. Unknown contracts reject rather than silently disabling or reinterpreting repairs. Each repair stores one donor offset, shared by its components. Reopening heals deterministically from that offset and never searches; the best-effort donor search runs only when a support is created or changed, and its result is stored. Evaluated pixels and GPU resources are never stored. Accepted repairs remain saveable before healing; strokes awaiting donor search are transient. Open restores accepted instructions lazily rather than healing every offscreen source before admission. Completed TIFF/AP0 EXR consume the accepted recipe independently of viewing history, and rendering readiness never changes dirty state.

## Compatibility is about the processing basis

Calibration makes sense only in the camera basis and development in which it was defined. Negative therefore retains an explicit scan setup: one camera model and camera-to-AP1 matrix, developer contract and accepted flat-field map. WB is not part of that basis: Negative always develops with unity WB, while Digital uses each source's as-shot WB. Scan sources share the basis; an enabled map additionally requires exact reference-development compatibility. Digital allows mixed cameras. New projects start in Negative; an unconfigured Negative project can switch the whole batch to Digital during import when a source is incompatible with its scan basis, independently of where that source occurs. Partial anchors, nondefault interpretation settings or an accepted correction prevent automatic switching.

Crossing input stages redevelops the ordered photos for the target stage's WB before accepting the new interpretation. It also clears the accepted flat-field correction and its nonordered reference record, because the map belongs to the stage in which it was measured. The dormant scan basis remains. Switching to Digital makes that basis inactive: it neither corrects Digital pixels nor constrains its sources. Returning to Negative redevelops with unity WB, rebuilds the scan basis from the new decode identities and validates the active sources against it. Incompatible mixed sources must be resolved first. Stage-specific Negative settings are discarded only after confirmation; shared grades and composition remain accepted choices.

The supported format is **schema 13 with processing model 5**. Unsupported schemas, repair contracts and incompatible development are rejected rather than migrated; unknown fields reject.

A source records its developed precision floor (including shading and WB-normalization amplification), while project calibration retains the separate native one-code base floor; each corrected source records a lens-development identity, and a flat reference requires an exact lens-development match. DNG development has a separate SDK/backend contract with canonical fixed RGB WB and per-source interpretation identity — SDK rejection never falls back to LibRaw.

Antitone does not hash the RAW file to establish photograph identity. Selected development metadata identifies its interpretation; file revisions captured at Import/Open validate disposable persisted caches. Open RAWs are treated as immutable for the session; reopening captures identity again. Missing or incompatible **ordered** sources reject Open; a missing flat-field reference does not block opening the project because the accepted map remains usable. Files containing other nonordered records are pruned on open to ordered photos and the accepted flat-field reference. Recovery and relinking UI are not implemented.

## Prepare before replacing

Preparing a candidate never changes worker-global source state — there is no active-source slot. Successful replacement retires the old owner/visit; failure preserves them, including the previous document and display even if the user had agreed to discard its unsaved edits. Open validates the candidate, checks sources and prepares the first photo before replacing the live document. Unresolved Digital sources defer numerical range admission until lazy preparation; the selected source must pass before replacement; the workflow/admission model behind this is [the architecture's](architecture.md).

Save captures accepted choices, validates them, writes a same-directory temporary and installs it with no-clobber or explicit replacement semantics. Its document-owned continuation accepts the written baseline without waiting for another paint. New/Open/Close use Save/Discard/Cancel protection, and pending numeric edits are processed before file actions. This protects an existing file from ordinary operation failures, not every crash or power-loss scenario.

## Working on this area

[Project types](../src/project.rs) own format, validation and compatibility identifiers. [Document](../src/editor/document.rs) and [file actions](../src/editor/files.rs) own staged editor transitions. [Preferences](../src/editor/preferences.rs) own independent UI persistence.

Exact serialization details belong to those types and [project tests](../tests/project.rs), [repair persistence tests](../tests/dust_project.rs), [camera compatibility tests](../tests/camera_project.rs), [film tests](../tests/film_project.rs) and [correction tests](../tests/flat_field_project.rs). Use [development checks](development.md#feature-specific-checks) before proposing recovery or source-identity changes.