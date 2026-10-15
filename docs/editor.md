# Editor state and interaction

The editor connects photographic settings, image tools and asynchronous work. Its central rule is simple: **a view may be temporary, but accepted photographic state must remain coherent**.

Roll, Photo and Rendering are editing scopes, not separate pipelines. Roll owns shared scan setup and grading; Photo owns individual grading and composition; Rendering selects the common preview/TIFF target. Changing tabs changes available controls, not the image's processing.

The editor uses a neutral dark workspace with amber identity and active-state accents. The shared application header presents the film-edge wordmark without making it an interactive control; File/Export availability remains controller-owned. [Branding assets](../assets/branding/README.md) explain the artwork and regeneration.

## Accepted state and drafts

A crop, picker or reference-area edit has temporary interaction state. Apply promotes a validated proposal; Cancel restores the working context. Tabs and outside clicks do not implicitly accept or discard a draft, and Save/Export never consume half-applied geometry.

Tools deliberately use different views. Dmin/Dmax expose uncropped oriented source so film margins are reachable. A bare-light reference exposes untransformed source. Gamma/WB picks retain the working photograph's composition and displayed grade, while their numerical fitting uses its declared measurement inputs.

Picker drags choose resize, move or new selection from the press position and keep that operation until release, independent of how far the first movement travels.

Those views must not overwrite the working Photo's accepted crop or settings. Starting a precision pick preserves navigation; temporary source selection preserves a separate return context. [Analyze's roll review](roll-analysis.md#1-choose-what-to-sample) uses that model for a set of per-photo sampling drafts: browsing and exception edits do not change accepted support, grading or multi-selection. Only successful analysis promotes support together with its grade. Review activates a viewer tool rather than replacing the inspector: the Roll analysis section reveals its controls and any Photo-reset warning, while Analyze roll / Cancel remain in the viewer bar. Stage tabs stay available, with photographic controls read-only during the transaction. Positive-only scopes temporarily collapse in this source view without changing their saved preference; finishing review restores the previous stage and scope visibility. [Geometry](geometry.md) explains the shared source/frame/canvas mappings.

In crop editing, handles resize the frame and dragging inside moves the image beneath it. Hovering outside the frame, away from its handles, uses a curved double-headed OS cursor oriented tangentially around the crop centre. A straightening drag retains that cursor, oriented around its original pivot, until release—even when the pointer moves inside the frame. Popovers, disabled input and leaving the canvas restore the ordinary cursor.

## Numeric edits own their input

A displayed number is not necessarily a rounded stored value. Controls must preserve precise values while the user reads or redraws them, and only a deliberate edit may change processing.

Pending text commits before a stage switch, Analyze snapshot or file action. Escape cancels text editing before it can cancel an image tool. Popovers, numeric controls and toolbar gestures own input ahead of the canvas, so one action cannot also pan, pick or apply a draft.

Colour wheels normally position the marker under the pointer. Hold Shift while dragging for ten-times finer motion from the current value. Changing Shift during a drag keeps the marker continuous: normal-speed motion resumes from its adjusted position until release, rather than snapping back under the pointer. A new ordinary gesture returns to direct positioning.

Disabled controls must be genuinely read-only. In particular, focus loss must not commit old pending text when an operation makes the control unavailable. Parameter domains and complete GPU preflight decide whether a proposed edit is acceptable; rejected edits restore the previous grade rather than switching to another processing path.

## Decisions are not operation failures

A decision asks permission before changing or replacing something; a failure explains why an attempted operation did not complete. Save/Discard and project/output replacement use the shared [decision dialog](../src/editor_ui_dialogs.rs). Save/Discard is a routine choice about keeping edits and has no severity banner. Replacement warns before changing existing work. Analyze includes its Photo-reset warning in the sampling-area review instead of opening another dialog. The same shared warning icon, written severity and heading provide meaning without relying on colour alone. A required choice is not itself a warning, and none of these confirmations is an operation failure.

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
| Analyze preparation and estimation | Counted area-preparation or sampling work, then indeterminate validation. Progress and genuine Cancel stay in the activity strip. The intervening area review is an interactive draft, not running work. |
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

Developed-source inspection bypasses inversion and photographic grading, but retains camera-to-AP1 conversion and ACES. Its [default display normalization](colour-and-output.md#source-inspection-has-its-own-exposure-convention) is shared with uncalibrated developed thumbnails. The exposure control adjusts the selected source preview relative to that default; zero restores the default, not the unscaled RAW buffer. This manual adjustment does not alter RAW development, calibration, fitting, exports or generated filmstrip thumbnails, and it is not saved in the project.

Ordinary source viewing uses the same physical-detail/coarse-underlay lifecycle as the positive, including before calibration. Its cached linear AP1 is acquired without photographic inversion; accepted flat-field correction remains upstream, as in the original-source inspection path. Inspection EV and default display normalization apply downstream of that cache, so brightness edits retain detail and navigation can fill missing support. Source and positive caches have distinct identities; switching modes reacquires the appropriate source rather than grading an inverted positive as source light. Original refinement does not replace working detail on idle, and a remembered positive-reference toggle has no effect here. Precision tools retain their original bypasses and source inspection still suspends photographic scopes.

Normal inspection retains accepted composition. Temporary calibration/reference tools and Analyze review may reveal another source domain, then restore the working view. Analyze review uses uncropped, unoriented sources and developed source thumbnails; opening and picture support remain sampling overlays, not crop previews. Hidden inspection retains its transient EV; the positive ignores it.

Missing calibration forces source-only presentation, not an explicit inspection choice. When a successful Dmin/Dmax pick completes the first usable pair, the editor returns to the positive. Inspection deliberately enabled with usable calibration is restored instead; failed/cancelled picks preserve calibration and the required source-only view.

## Immutable local-grade routing

Selected and cached previews, thumbnails, scopes and refinement share complete processing snapshots, while each image owns its mutable output/preparation state. Roll WB, calibration/correction, camera basis, Global settings and ordered local values invalidate a grade; names, IDs, allocator history, view/target selection and inspection EV do not recreate its immutable constants. Compatible source caches can therefore survive a grade edit and paint current accepted settings.

Candidates validate before replacing processing state, including offscreen sources during roll/calibration changes. Refinement commands capture their input rather than reading later edits; ordinary invalidation retains the single-job admission token until completion, and stale generations cannot install. Inspection and global fitting keep their explicit bypasses. The normal editor still rejects nonempty local collections until [admission, target/list actions and gradient transactions](plans/local-adjustments.md) are ready; prospective routing tests do not expose a complete local feature.

## A stable working view while grading

`WorkingView` owns normal-view demand, revision, accepted presentation and idle readiness for positive and source viewing. Original-sample reconstruction is a separate purpose-bound service: explicit positive comparison or precision tools, never an idle normal-view state machine. Background GPU thumbnails follow accepted working-view readiness rather than waiting for an original reference.

The positive view acquires a **physical-view-sized signed/HDR linear AP1 source** from the existing backend's immutable developed-source lease. Correction and calibrated inversion still process original camera pixels before footprint filtering. At a fixed view, exposure, Roll/Photo WB, contrast and saturation reuse that same source. Colour grading uses stable f32 formulas with targeted compensated luminance accumulation; mask coordinates and original-source reference processing retain their precision. This is an approximate preview, not a new export authority. [Processing](processing.md#physical-view-processing-is-an-approximation) and [geometry](geometry.md#process-before-averaging) distinguish arithmetic agreement from sampling quality.

Navigation, accepted composition, physical display scale and upstream calibration/response/correction replace the source asynchronously. A single latest-request mailbox coalesces demand on the existing command worker; grading does not restart compatible acquisition. Both original support and destination filtering use bounded GPU units, including when a small original region fills a magnified display. Old imagery may be transformed during navigation, but replacements always come from originals, never recursive resampling of a preview. Source/view authority rejects obsolete completions.

A coarse whole-composition neutral AP1 underlay fills newly exposed areas while regional support is missing. It is acquired from originals alongside the first detail raster, with independent footprints in the same traversal—not from a rendered bitmap or a recursively resized preview. Compatible navigation cannot starve that first coverage acquisition. Detail and underlay use the same accepted source basis, grade and target; a single target-linear composite fills only missing detail, preserving valid black pixels and excluding genuinely unsupported coverage. Viewer and scopes consume that same composite. A matching fixed-view detail raster bypasses composition entirely. Startup and upstream changes must acquire whole-composition coverage even if the initial view is zoomed; both sources publish only when complete.

Grading is captured at frame boundaries, not completed by a worker that could repeatedly lose a race with continuous controls. One submitted colour result remains immutable until completion; subsequent input coalesces to the latest settings. Viewer and scopes share that accepted target-linear result. Target-only changes can rerender retained graded AP1, and unchanged requests submit no new photographic work. Initialization is marked only after submission through the shared host gate, not after creating an encoder. GPU bindings retain encoded/in-flight native versions; completion callbacks only signal readiness and repaint. No UI-thread completion/recycling wait is used. Navigation composition also coalesces while its previous submission is incomplete, rather than accumulating display-sized targets. During that short GPU backlog the retained accepted mapping may lag; scopes hold compatible traces until current-frame coverage is available.

The **Original-resolution reference** viewer toggle switches in place to original-sample GPU grading/ACES reconstructed at the same physical view. It requests original processing only while comparison is enabled, after editing pauses; until the current reference is ready, the working view remains visible with an explicit preparing state. Ordinary positive/source viewing does not prepare a hidden reference. Reference completion does not automatically sharpen or replace the normal working view, so release/idle does not change its detail level. Explicit comparison is a mandatory selected-view task. After comparison ends, completed compatible reference retention reserves the existing image-cache allowance; zero optional retention or memory pressure releases it. Unfinished comparison work is cancelled without admitting another job before its completion. Source inspection and precision tools keep their own bypasses.

The selected linear source, coarse coverage and their graded AP1/target rasters are mandatory working resources, separate from optional native/thumbnail retention. Navigation can additionally retain a physical-view composite; unchanged fixed-view grading does not allocate that composite. Building support, a completed mailbox result and encoded/in-flight old versions can overlap; one logical entry is not an aggregate physical-memory bound. GPU capacities are checked without reducing physical resolution. Full concurrent memory, photographic preview/export correspondence, broader devices and physical input/presentation remain [acceptance work](plans/view-working-set.md). Local tools/admission and GPU export migration are still paused; export continues to reconstruct originals through its existing path.

## Photo-only fullscreen inspection

The fullscreen action beside the zoom controls, or F, turns the current photograph into a full-display inspection view. The host makes the existing window borderless and covers its current display without moving into a native fullscreen Space. It retains the same managed image renderer. An already-native-fullscreen window stays native fullscreen; macOS's ordinary fullscreen button remains separate from photo-only presentation.

Shared zoom/source controls float over the image, then fade after inactivity. Moving near the bottom or pressing Tab reveals them; hovering over the bar, dragging a control, keyboard focus, numeric editing and open popovers keep them visible. Moving or panning elsewhere over the photograph does not reveal chrome. A hidden bar leaves no invisible canvas hitbox. F/Escape remain available while it is hidden, after text and popovers have had their turn.

Analyze review keeps its sampling controls visible and does not enter photo-only fullscreen. Other image tools are suspended, not applied or cancelled. A controller-owned snapshot retains workspace navigation and source inspection; it is restored on the frame after the host acknowledges window restoration. The host owns prior window geometry/decorations and application menu/Dock presentation, restoring presentation when focus leaves and on exit. OS-adjusted window bounds cannot strand the controller in a pending exit. Accepted photographs, drafts and independent workspace preferences are unchanged. [Remaining native and physical acceptance](plans/inspection-and-interaction.md#layout-and-controls) is separate from supplied-viewport fixtures.

## Fast feedback is not authoritative data

The editor uses several image representations for different jobs:

| Representation | Purpose |
| --- | --- |
| Full developed `Scan` | Full-resolution measurement, refinement and export |
| Camera-RGB proxy | Initial/pending feedback, thumbnails and inspection/tool previews |
| Filtered view-sized linear AP1 | Stable positive/source input at physical display density |
| Accepted target-linear view | Shared viewer/scope input; never graded or composed again |
| Original-sample regional result | Explicit same-position comparison or precision-tool refinement |
| Cached pending preview | Temporary display while the selected RAW prepares |
| Embedded camera JPEG | Filmstrip-only navigation placeholder |

Selected and pending GPU proxies retain one immutable compact camera source, independent of filmstrip processing. Their active-target AP1/output is created lazily rather than retaining two target-specific camera sources. Normal working-view acceptance releases that provisional output; tools or missing-view feedback can recreate it. Old encoded/in-flight consumers retain their own immutable resources through target changes and release.

A pending preview can show the current grade from a retained compatible RGB16 proxy, but it is not a ready `Scan`. Picks, crop editing, refinement, scopes and exports wait for full preparation. A memory miss retains the loading placeholder. Normal photo preparation does not scan full pixels for an illumination/support proposal. Analyze explicitly prepares roll-level spatial evidence on the worker. The older shared picture-area editor requests a local illumination hint only when needed; flat-reference preparation calculates its proposal explicitly. Delayed suggestions cannot overwrite a drawn region or survive cancellation/source replacement.

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

Retain demanded immutable full-resolution camera developments and uncorrected proxies within fixed optional byte limits. A compatible native hit avoids LibRaw development; correction, grade and crop remain independent consumer state. Original comparison/tool reconstruction still processes original-resolution pixels, so a native hit is not a cached final rendering.

Visible missing thumbnails get priority, then unvisited roll sources receive eager proper CPU/disk thumbnail work. One [native priority worker](../src/editor_development.rs) serves selected/reference/Analyze and thumbnail development, using the established high thread budget for every source. Foreground precedes visible thumbnails, then eager demand; arbitration occurs between synchronous unpack/development calls. Compatible document-scoped in-flight work joins without a duplication timeout. Thumbnail admission is nonblocking, so newly visible misses can precede an eager job's next native phase. Cancelling a GPU ticket releases visible admission without discarding already-owned proper CPU/disk demand. That work does not wait for grading idle or require optional GPU space, and continues CPU-only while fullscreen hides the strip. Offscreen GPU construction waits for accepted-view idleness, yields to explicitly requested original work and needs spare retention space. Completed attempts are tracked separately from residency, preventing GPU eviction from restarting whole-roll RAW generation; visible demand materializes from CPU/disk data when available. Selected work stays ahead of background development. The Preferences menu controls CPU/GPU optional cache storage in whole MiB, including zero, independently of project/grade state. Chosen limits are saved when first resolved or edited; relaunching on different hardware preserves them. Reset deliberately selects new defaults from installed RAM and confirmed GPU topology/capacity, with explicit fallbacks for unavailable facts. [Default policy and units](../src/editor_cache_limits.rs) are retention choices, not predictions of safe allocation.

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
