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

[Operation controllers](../src/editor_feedback.rs) retain the typed failure separately from status text. Acknowledgement or a matching successful result retires it; merely starting another job, updating unrelated status or receiving stale/cancelled results does not. Presentation returns acknowledgement only. Other loading/import/analysis/background messages and progress remain on their existing surfaces; [broader feedback work](plans/completion.md#operation-feedback) is separate. No notification bus or status-prefix severity inference is used.

## Source inspection is a view

Developed-source inspection bypasses inversion and photographic grading. Its exposure adjustment affects that source preview only. It does not alter RAW development, calibration, fitting, exports or generated filmstrip thumbnails, and it is not saved in the project.

Normal inspection retains accepted composition. Temporary calibration/reference tools may reveal another source domain, then restore the working view. Hidden inspection retains its transient EV; the positive ignores it.

Missing calibration forces source-only presentation, not an explicit inspection choice. When a successful Dmin/Dmax pick completes the first usable pair, the editor returns to the positive. Inspection deliberately enabled with usable calibration is restored instead; failed/cancelled picks preserve calibration and the required source-only view.

## Fast feedback is not authoritative data

The editor uses several image representations for different jobs:

| Representation | Purpose |
| --- | --- |
| Full developed `Scan` | Full-resolution measurement, refinement and export |
| Camera-RGB proxy | Approximate live processing, thumbnails and scope input |
| Settled regional image | Full-source-derived reconstruction of the visible region |
| Cached pending preview | Temporary display while the selected RAW prepares |
| Embedded camera JPEG | Filmstrip-only navigation placeholder |

A pending preview can show the current grade from a retained compatible RGB16 proxy, but it is not a ready `Scan`. Picks, crop editing, refinement, scopes and exports wait for full preparation. A memory miss retains the loading placeholder.

An embedded JPEG is different again: it is already camera-rendered and can show different framing/colour. Its Camera badge makes that distinction visible. It never becomes the selected processing image or a source for inversion, correction, measurement or export. Developed thumbnail completion supersedes it; authoritative RAW errors take precedence.

Filmstrip selection follows the current source identity, not pixel readiness: its outline remains during cold loading, JPEG placeholders and failure. Cards expose their full filename and selected state to accessibility. Busy operations, crop drafts and reference-support edits disable selection and keyboard focus; ordered-roll Dmin/Dmax source selection remains available when not busy.

## Responsiveness and optional caches

Keep reusable source proxies rather than redeveloping a RAW for every grade change. Proxies are uncorrected camera data, so current grading, correction and composition can be applied without treating an old thumbnail as a baked photograph.

Visible missing thumbnails get priority over optional offscreen prefetch. Selected work stays ahead of background development, and cache pressure may reduce retention or suspend optional work—not processing precision or accepted output quality. File revisions are cache freshness checks, [not photographic identity](projects.md#compatibility-is-about-the-processing-basis).

GPU infrastructure is shared, but mutable image storage, revision and processing signatures belong to each image/job. Reusing a renderer must not make the selected photograph alias a thumbnail's processing state. [Architecture](architecture.md#ownership-and-execution) describes the worker boundaries.

## Build UI from shared presentation

Use the existing component families rather than copying their painting or interaction logic:

- [Presentation](../src/editor_ui.rs): application header/branding, labelled rows, text actions and viewer tool groups.
- [Hierarchy](../src/editor_ui_hierarchy.rs): sections, disclosure/reset and stage tabs.
- [Decisions](../src/editor_ui_dialogs.rs): shared modal framing, actions and dismissal.
- [Feedback](../src/editor_ui_feedback.rs): explicit severity, acknowledged messages and technical details.
- [Cards](../src/editor_ui_cards.rs): contact-sheet geometry, captions and supplied-state artwork.
- [Icons](../src/editor_icons.rs): semantic actions and artwork.
- [Controls](../src/editor_controls.rs): photographic values, precision and calibration/balance adapters.

Presentation receives display-ready state and returns responses. Controllers still own availability, IDs, accepted/draft state, resets and dispatch. Cards receive selection/availability, and their caption supplies the unelided semantic filename; they do not own images, selection, virtualization or workers. Pinned section titles expose label semantics without disclosure/focus behavior. The fixed viewer bar hosts tool identity and Apply/Cancel without resizing the canvas; only selection/frame artwork belongs over the photograph.

### Colour roles

Colour expresses interaction state, not feature ownership. Presentation installs the editor theme once; ordinary egui widgets and the shared components inherit its selection, active-state and link colours. Choose semantic variants such as a primary action rather than choosing amber or overriding widget visuals in a feature controller.

Custom interaction painting reads the applicable inherited role from `ui.visuals()` or `ui.style().interact(&response)`, rather than copying brand RGB values. Contact-sheet fills and image wells deliberately stay neutral; photographic wheels and scope plots retain colours that represent image data. These treatments belong to their shared component or visualization owner. Add a genuinely needed role or variant there, not as a feature-local palette.

Crop grids and native menus remain specialized. Extend a shared family for a real variant; do not create a parallel widget/state framework to accommodate a hypothetical one.

## Working on this area

Start with [editor orchestration](../src/editor.rs), then the feature owner: [viewer](../src/editor_viewer.rs), [crop](../src/editor_crop.rs), [analysis](../src/editor_analysis.rs), [correction](../src/editor_flat_field.rs), [thumbnails](../src/editor_thumbnails.rs), [pending previews](../src/editor_selected_preview.rs) or [camera placeholders](../src/editor_embedded_preview.rs).

Use both component and composed-event checks from [development](development.md#shared-editor-ui-checks). They cover precision, transaction, input ownership and geometry—not complete accessibility or physical feel. [Interaction follow-up](plans/inspection-and-interaction.md) tracks remaining wheel semantics and unsettled UX choices.
