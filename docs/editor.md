# Editor state and interaction

The editor connects photographic settings, image tools and asynchronous work. Its central rule is simple: **a view may be temporary, but accepted photographic state must remain coherent**.

Roll, Photo and Rendering are editing scopes, not separate pipelines. Roll owns shared scan setup and grading; Photo owns individual grading and composition; Rendering selects the common preview/TIFF target. Changing tabs changes available controls, not the image's processing.

The editor uses a neutral dark workspace with amber identity and active-state accents. The shared application header presents the film-edge wordmark without making it an interactive control; File/Export availability remains controller-owned. [Branding assets](../assets/branding/README.md) explain the artwork and regeneration.

## Accepted state and drafts

A crop, picker or reference-area edit has temporary interaction state. Apply promotes a validated proposal; Cancel restores the working context. Tabs and outside clicks do not implicitly accept or discard a draft, and Save/Export never consume half-applied geometry.

Tools deliberately use different views. Dmin/Dmax expose uncropped oriented source so film margins are reachable. A bare-light reference exposes untransformed source. Gamma/WB picks retain the working photograph's composition and displayed grade, while their numerical fitting uses its declared measurement inputs.

Picker drags choose resize, move or new selection from the press position and keep that operation until release, independent of how far the first movement travels.

Those views must not overwrite the working Photo's accepted crop or settings. Starting a precision pick preserves navigation; temporary source selection preserves a separate return context. [Geometry](geometry.md) explains the shared source/frame/canvas mappings.

In crop editing, handles resize the frame and dragging inside moves the image beneath it. Hovering outside the frame, away from its handles, uses a curved double-headed OS cursor oriented tangentially around the crop centre. A straightening drag retains that cursor, oriented around its original pivot, until release—even when the pointer moves inside the frame. Popovers, disabled input and leaving the canvas restore the ordinary cursor.

## Numeric edits own their input

A displayed number is not necessarily a rounded stored value. Controls must preserve precise values while the user reads or redraws them, and only a deliberate edit may change processing.

Pending text commits before a stage switch, Analyze snapshot or file action. Escape cancels text editing before it can cancel an image tool. Popovers, numeric controls and toolbar gestures own input ahead of the canvas, so one action cannot also pan, pick or apply a draft.

Colour wheels normally position the marker under the pointer. Hold Shift while dragging for ten-times finer motion from the current value. Changing Shift during a drag keeps the marker continuous: normal-speed motion resumes from its adjusted position until release, rather than snapping back under the pointer. A new ordinary gesture returns to direct positioning.

Disabled controls must be genuinely read-only. In particular, focus loss must not commit old pending text when an operation makes the control unavailable. Parameter domains and complete GPU preflight decide whether a proposed edit is acceptable; rejected edits restore the previous grade rather than switching to another processing path.

## Decisions are not operation failures

A decision asks permission before changing or replacing something; a failure explains why an attempted operation did not complete. Save/Discard, project/output replacement and Analyze's Photo reset use the shared [decision dialog](../src/editor_ui_dialogs.rs). Save/Discard is a routine choice about keeping edits and has no severity banner. Project/output replacement and Analyze's Photo reset warn before replacing existing work; their warning icon, written severity and heading provide meaning without relying on colour alone. A required choice is not itself a warning, and none of these confirmations is an operation failure.

Decision dialogs are centred in the editor with consistent padding, wrapped explanatory text and right-aligned natural-size actions separated from the text by a little extra space. Copy names the consequence and what is preserved; destination paths remain readable context. At narrow widths, actions stack rather than clip. Controllers supply the copy and stable IDs, retain staged paths/continuations and dispatch the returned choice. Presentation never saves, exports, resets or validates a request itself.

These decisions block editor input. Cancel receives initial focus; Escape cancels after any open popover has had its turn. Outside clicks do not dismiss or accept the decision. There is no dialog-wide Enter-to-confirm binding: an explicitly focused action retains ordinary button keyboard activation. Closing the application still goes through the document transition, not an implicit dialog acceptance. Native file choosers remain native, and help/source-inspection popovers are not decision dialogs.

[Feedback presentation](../src/editor_ui_feedback.rs) distinguishes information, warning and error with a named severity, different glyph and restrained colour. Flat-field reference/correction failures, project-save failures and TIFF/EXR export failures use acknowledged messages; a rejected non-integer reference area is a warning, not a failed correction. The explanation states what did not happen and what remains intact. Full diagnostic details are selectable behind a compact, subdued **Details** disclosure without a section divider. Its expansion is transient, with bounded scrolling for long diagnostics; keyboard focus remains clearly visible.

These messages use the same centred shell as decisions. OK or Escape acknowledges the message without retrying, accepting a draft or changing the document; popovers retain Escape precedence and outside clicks do not dismiss. Editor input is blocked until acknowledgement. A pending Save/Discard decision is retained but waits behind the failure message, so the failed save cannot be hidden under another prompt.

[Operation controllers](../src/editor_feedback.rs) retain the typed failure separately from status text. Acknowledgement or a matching successful result retires it; merely starting another job, updating unrelated status or receiving stale/cancelled results does not. Presentation returns acknowledgement only. Other loading/import/analysis failures remain passive status rather than acknowledged messages; [remaining coverage and native acceptance](plans/completion.md#operation-feedback) are separate. No notification bus or status-prefix severity inference is used.

## Working feedback follows what remains usable

Working feedback answers two questions: **what is happening, and what can I still do?** It is neutral activity, not a warning, error or request for approval. Choose its prominence from the operation's actual input restrictions, not the editor's `busy` flag alone.

| Work | Presentation and retained use |
| --- | --- |
| RAW import, project Open/New/Save, correction application | A centred working modal after a brief delay: a short title and progress bar, without paths or implementation detail. The document transaction already blocks the workspace; short jobs avoid flashing a dialog. |
| Export | Quiet status-bar activity labelled “Exporting…”, without a floating panel or format-specific copy. Editing and source switching remain blocked, but canvas inspection, stage switching and preferences remain usable. |
| Selected RAW preparation | A large spinner centred on the empty canvas, without visible explanatory text. A compatible cached pending preview stays visible when available. Neither that preview nor a camera JPEG becomes a ready processing source. |
| Reference preparation and picker measurement | Local tool/viewer feedback and a quiet activity strip; no extra global dialog. |
| Analyze | Real sampling counts, then indeterminate activity. Progress and its genuine Cancel action stay in the activity strip across every stage. |
| Refinement, thumbnails, camera previews and scopes | Existing in-place image/card/scope feedback. Optional background work must not seize focus or create global notification noise. |

The fixed-height activity strip keeps the viewer geometry stable. It shows the active operation rather than incidental status text; long passive status remains available through its tooltip and accessibility text. Working modals yield to acknowledged failures. They have no OK, close or backdrop-dismiss action, and Escape does not pretend to cancel them. Native file choosers remain separate.

Dialogs use a moving indeterminate bar when duration is unknown; local activity uses a spinner. Import/Open report completed-file counts while reading/checking files, then return to indeterminate preparation. Analyze reports sampled-frame counts similarly. These fractions describe counted work, not a percentage of the entire job; reaching the final file does not finish validation or promote the document. Do not invent time estimates, native/GPU progress or Cancel actions. Analyze cancellation is cooperative: “Cancelling analysis…” retains a spinner and removes Cancel until the worker's terminal reply; afterward “Analysis cancelled” is passive text without a spinner.

[Task activity](../src/editor_activity.rs) is the foreground integration point. Begin a typed task using the controller's existing request ID; its definition supplies concise copy, progress eligibility, surface and any genuine cancellation action. Report counted or indeterminate work through the same API, then finish the matching request on success, failure or send failure:

```rust
self.tasks.begin(Task::Import(id));
self.tasks.report(TaskRequest::Document(id), TaskProgress::Counted { completed, total });
self.tasks.finish(TaskRequest::Document(id));
```

Request keys scope IDs by operation: a photo-load result cannot finish an export that happens to use the same number. Stale reports and finishes are ignored; completed counts return to indeterminate without finishing the task. Beginning new activity replaces only its presentation, so the controller must supersede the old execution first. Cancellation similarly has two responsibilities: the controller signals the real worker, then marks the matching task `cancelling`. That state removes measured progress and Cancel, ignores late counts and stays visible until matching completion. It does not infer cancellation from status text or processing generations.

The task slot and mutable progress state are private. Analyze uses this same lifecycle; its separate pending request retains only execution identity and validation snapshots. Controllers still own availability, worker dispatch, generation checks and atomic promotion. [Working presentation](../src/editor_ui_progress.rs) renders supplied task views across modal/inline/compact surfaces and the centred viewer spinner, returning a request-bound cancellation action for the controller to validate. New foreground work should extend this path, not add its own activity fields or progress window. Optional background work remains local; there is no task executor, registry or parallel event system.

## Source inspection is a view

Developed-source inspection bypasses inversion and photographic grading. Its exposure adjustment affects that source preview only. It does not alter RAW development, calibration, fitting, exports or generated filmstrip thumbnails, and it is not saved in the project.

Normal inspection retains accepted composition. Temporary calibration/reference tools may reveal another source domain, then restore the working view. Hidden inspection retains its transient EV; the positive ignores it.

Missing calibration forces source-only presentation, not an explicit inspection choice. When a successful Dmin/Dmax pick completes the first usable pair, the editor returns to the positive. Inspection deliberately enabled with usable calibration is restored instead; failed/cancelled picks preserve calibration and the required source-only view.

## Photo-only fullscreen inspection

The fullscreen action beside the zoom controls, or F, turns the current photograph into a full-display inspection view. The host makes the existing window borderless and covers its current display without moving into a native fullscreen Space. It retains the same managed image renderer. An already-native-fullscreen window stays native fullscreen; macOS's ordinary fullscreen button remains separate from photo-only presentation.

Shared zoom/source controls float over the image, then fade after inactivity. Moving near the bottom or pressing Tab reveals them; hovering over the bar, dragging a control, keyboard focus, numeric editing and open popovers keep them visible. Moving or panning elsewhere over the photograph does not reveal chrome. A hidden bar leaves no invisible canvas hitbox. F/Escape remain available while it is hidden, after text and popovers have had their turn.

Image tools are suspended, not applied or cancelled. A controller-owned snapshot retains workspace navigation and source inspection; it is restored on the frame after the host acknowledges window restoration. The host owns prior window geometry/decorations and application menu/Dock presentation, restoring presentation when focus leaves and on exit. OS-adjusted window bounds cannot strand the controller in a pending exit. Accepted photographs, drafts and independent workspace preferences are unchanged. [Remaining native and physical acceptance](plans/inspection-and-interaction.md#layout-and-controls) is separate from supplied-viewport fixtures.

## Fast feedback is not authoritative data

The editor uses several image representations for different jobs:

| Representation | Purpose |
| --- | --- |
| Full developed `Scan` | Full-resolution measurement, refinement and export |
| Camera-RGB proxy | Approximate live processing, thumbnails and scope input |
| Settled regional image | Full-source-derived reconstruction of the visible region |
| Cached pending preview | Temporary display while the selected RAW prepares |
| Embedded camera JPEG | Filmstrip-only navigation placeholder |

A pending preview can show the current grade from a retained compatible RGB16 proxy, but it is not a ready `Scan`. Picks, crop editing, refinement, scopes and exports wait for full preparation. A memory miss retains the loading placeholder. Normal photo preparation does not scan full pixels for an illumination/support proposal. Those hints are calculated on worker demand for Analyze or picture-area editing, while flat-reference preparation calculates its proposal explicitly. Delayed picture suggestions cannot overwrite a drawn region or survive picker cancellation/source replacement.

An embedded JPEG is different again: it is already camera-rendered and can show different framing/colour. Its Camera badge makes that distinction visible. It never becomes the selected processing image or a source for inversion, correction, measurement or export. Developed thumbnail completion supersedes it; authoritative RAW errors take precedence.

## Select and remove roll photos

Selection is temporary workspace state, separate from each source's accepted Photo settings. The active photo always belongs to the selected set and has a lighter neutral card background, including the caption area, as well as the selection outline. Cards expose their full filename, selection and active-photo description to accessibility; these states do not depend on pixel readiness.

- Plain click makes that photo active. If it was already selected, the selection is retained; otherwise it becomes the sole selection. Arrow navigation uses the same rule.
- Cmd-click toggles membership without changing the active photo unless it is deselected. Then a selected successor becomes active, or the last selected predecessor. Cmd-click on the sole selected photo does nothing.
- Shift-click selects the contiguous range from the last plain-click anchor and makes that anchor active.
- Shift–Cmd–A reduces the selection to the active photo, which becomes the new range anchor. It does not unload or reload the photograph and leaves photographic state unchanged. Text editing, popups and tool drafts retain input ownership.

Use **Remove selected photos…** in a selected card's context menu, or Delete/Backspace outside text editing and popups. Removal requires confirmation because there is no undo. It changes only filmstrip membership: RAW files, retained source settings, calibration and correction remain untouched. After removal, the active position is the old index in the shortened roll, clamped to its last photo—not necessarily the next surviving identity. An empty roll has no active photo and can be saved, reopened and repopulated without resetting its retained camera basis or Roll settings.

Selected-photo preparation keeps mouse/keyboard navigation available: a newer active photo retires its predecessor's waiter and ignores stale Ready messages. Removal still waits for foreground work to finish. Other busy operations, crop drafts and reference-support edits disable selection and keyboard focus. Ordered-roll Dmin/Dmax browsing remains available when not busy, but is temporary sampling context: it does not change the working multi-selection or its anchor. Removal cannot accept or cancel a tool draft. Pending previews remain non-authoritative, so picks, crop and export still wait for Ready.

## Responsiveness and optional caches

Retain demanded immutable full-resolution camera developments and uncorrected proxies within fixed optional byte limits. A compatible native hit avoids LibRaw development; correction, grade and crop remain independent consumer state. Settled reconstruction still processes original-resolution pixels for the current view, so a native hit is not a cached final rendering.

Visible missing thumbnails get priority, then unvisited roll sources receive eager proper CPU/disk thumbnail work. One [native priority worker](../src/editor_development.rs) serves selected/reference/Analyze and thumbnail development, using the established high thread budget for every source. Foreground precedes visible thumbnails, then eager demand; arbitration occurs between synchronous unpack/development calls. Compatible document-scoped in-flight work joins without a duplication timeout. Thumbnail admission is nonblocking, so newly visible misses can precede an eager job's next native phase. Cancelling a GPU ticket releases visible admission without discarding already-owned proper CPU/disk demand. That work does not wait for grading idle or require optional GPU space, and continues CPU-only while fullscreen hides the strip. Offscreen GPU construction still yields to refinement and needs spare retention space. Completed attempts are tracked separately from residency, preventing GPU eviction from restarting whole-roll RAW generation; visible demand materializes from CPU/disk data when available. Selected work stays ahead of background development. The Preferences menu controls CPU/GPU optional cache storage in whole MiB, including zero, independently of project/grade state. Chosen limits are saved when first resolved or edited; relaunching on different hardware preserves them. Reset deliberately selects new defaults from installed RAM and confirmed GPU topology/capacity, with explicit fallbacks for unavailable facts. [Default policy and units](../src/editor_cache_limits.rs) are retention choices, not predictions of safe allocation.

Pressure warnings drop unprotected optional native/proxy/thumbnail ownership and suspend offscreen work without rewriting chosen limits. Selected/visible essentials and active resources may exceed the optional limits. Native temporaries, allocator/driver backing and other mandatory work are not covered; lower cache limits do not establish supported minimum hardware or guarantee instant reclamation, no swapping or recoverable allocation failure. The CPU limit governs one document-scoped native/proxy LRU; foreground loads reuse compatible full developments with independent correction contexts. Candidate documents keep separate zero-budget optional ownership until acceptance. [Broader real-use/resource acceptance](plans/visual-feedback.md) remains separate; [native scheduling and bounded switching diagnostics](native-development.md#editor-scheduling) describe the cold-selection tradeoff and retained-hit measurements. Different-source foreground misses may wait behind an unrelated non-preemptible native call; software queue tests do not establish better roll throughput or latency. File revisions are cache freshness checks, [not photographic identity](projects.md#compatibility-is-about-the-processing-basis).

GPU infrastructure is shared, but mutable image storage, revision and processing signatures belong to each image/job. Reusing a renderer must not make the selected photograph alias a thumbnail's processing state. [Architecture](architecture.md#ownership-and-execution) describes the worker boundaries.

## Build UI from shared presentation

Use the existing component families rather than copying their painting or interaction logic:

- [Presentation](../src/editor_ui.rs): application header/branding, labelled rows, text actions and viewer tool groups.
- [Viewer framing](../src/editor_ui_viewer.rs): docked/auto-hiding controls and full-canvas presentation, separate from navigation and native window state.
- [Hierarchy](../src/editor_ui_hierarchy.rs): sections, disclosure/reset and stage tabs.
- [Decisions](../src/editor_ui_dialogs.rs): shared modal framing, actions and dismissal.
- [Feedback](../src/editor_ui_feedback.rs): explicit severity, acknowledged messages and technical details.
- [Tasks](../src/editor_activity.rs): typed foreground begin/report/cancelling/finish lifecycle and coherent feedback policy.
- [Working](../src/editor_ui_progress.rs): supplied task-view progress/activity and optional cancellation, using the surface appropriate to retained usability.
- [Cards](../src/editor_ui_cards.rs): contact-sheet geometry, captions and supplied-state artwork.
- [Icons](../src/editor_icons.rs): semantic actions and artwork.
- [Controls](../src/editor_controls.rs): photographic values, precision and calibration/balance adapters.

Presentation receives display-ready state and returns responses. Controllers still own availability, IDs, accepted/draft state, resets and dispatch. Cards receive selected/active/availability state, and their caption supplies the unelided semantic filename; they do not own images, selection, virtualization or workers. Pinned section titles expose label semantics without disclosure/focus behavior. In the workspace, the fixed viewer bar hosts tool identity and Apply/Cancel without resizing the canvas; only selection/frame artwork belongs over the photograph. Photo-only inspection uses the shared floating-bar variant rather than moving tool transactions onto the image.

### Typography roles

Reading text should be quiet; short instrument markings give the workspace its camera-body character. The editor embeds [IBM Plex Sans](../assets/fonts/README.md): Text for ordinary controls, explanations, filenames and numeric entry; Medium for stage tabs, active viewer-tool names and compact zoom/source-EV readouts. The Arimo wordmark remains separate artwork. Default glyph fallbacks and monospace text are retained.

[Presentation](../src/editor_ui.rs) installs the fonts during editor startup and owns `instrument_label`, `instrument_readout` and `TextAction::readout`. Use these roles rather than naming fonts or overriding feature-local styles. Instrument labels are uppercase with restrained spacing; readouts retain their supplied formatting without extra tracking. Do not apply the instrument role to every heading, button or number. Ordinary `strong` text changes semantic emphasis, not font weight.

Stage tabs share equal breathing room around their labels rather than equal widths, so Rendering has room beside its chevron at the narrow inspector size. Their full bar remains fixed-width and fixed-height; accessible names, stage order and keyboard activation do not depend on uppercase lettering. Typography does not change numeric precision, disabled snapshots, processing or persistence.

### Colour roles

Colour expresses interaction state, not feature ownership. Presentation installs the editor theme once; ordinary egui widgets and the shared components inherit its selection, active-state and link colours. Choose semantic variants such as a primary action rather than choosing amber or overriding widget visuals in a feature controller.

Custom interaction painting reads the applicable inherited role from `ui.visuals()` or `ui.style().interact(&response)`, rather than copying brand RGB values. Contact-sheet fills and image wells deliberately stay neutral; photographic wheels and scope plots retain colours that represent image data. These treatments belong to their shared component or visualization owner. Add a genuinely needed role or variant there, not as a feature-local palette.

Crop grids and native menus remain specialized. Extend a shared family for a real variant; do not create a parallel widget/state framework to accommodate a hypothetical one.

## Working on this area

Start with [editor orchestration](../src/editor.rs), then the feature owner: [viewer](../src/editor_viewer.rs), [crop](../src/editor_crop.rs), [analysis](../src/editor_analysis.rs), [correction](../src/editor_flat_field.rs), [thumbnails](../src/editor_thumbnails.rs), [pending previews](../src/editor_selected_preview.rs) or [camera placeholders](../src/editor_embedded_preview.rs).

Use both component and composed-event checks from [development](development.md#shared-editor-ui-checks). They cover precision, transaction, input ownership and geometry—not complete accessibility or physical feel. [Interaction follow-up](plans/inspection-and-interaction.md) tracks remaining wheel semantics and unsettled UX choices.
