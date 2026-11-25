# Editor state and interaction

The editor connects photographic settings, image tools and asynchronous work. Its central rule is simple: **a view may be temporary, but accepted photographic state must remain coherent**.

Roll, Photo and Rendering are editing scopes, not separate pipelines. Roll selects Negative or Digital interpretation and owns shared grading plus Negative calibration and optional flat-field correction; Photo owns individual grading and composition — its turn/flip setting starts from the orientation the camera recorded, and saved projects keep their stored setting; Rendering owns Film texture, LUT, Film & paper and the preview target, which also initializes rendered export. Changing tabs changes available controls, not the image's processing. [Architecture](architecture.md) describes how these scopes are owned and disposed.

The editor uses a neutral dark workspace with amber identity and active-state accents. The shared application header presents the film-edge wordmark and project identity without making the wordmark an interactive control. [Branding assets](../assets/branding/README.md) explain the artwork and regeneration.

## Application menus and settings

On macOS, native AppKit menus provide the conventional Antitone, File, Edit, View, Window and Help groups. File → **Add Scans…** adds camera RAW source references without moving or modifying originals; Export belongs under File. Settings, About and local workflow help open focused in-editor panels. Settings controls optional CPU/GPU retention in whole MiB, including zero and explicit reset — see [Responsiveness and optional caches](#responsiveness-and-optional-caches); those limits are not a process-memory cap.

The [shared menu model](../src/platform/menu.rs) supplies actions, grouping and shortcuts to native and egui presentation; the [host](../src/platform/host.rs) owns native menu lifetime and forwards activations through winit's event-loop proxy, restoring a minimized editor so panels do not open invisibly. The editor revalidates menu activations against their input, decision and operation constraints; an active tool alone does not prevent activating another. Text-targeting Edit actions receive pending input before focus changes. File actions capture accepted values; unfinished numeric text stays with its widget. The egui fallback uses the same commands and state when native menus are not installed; only the macOS host is implemented.

### Native quit negotiation

Dock and system Quit enter the same unsaved-change guard as menu Quit: the editor reports live close continuations, and reserved input or live scoped edits decline the request without accepting or discarding anything. The [termination bridge](../src/platform/termination.rs) uses AppKit's deferred decision — winit owns its delegate, so the bridge adds only the missing optional decision method — and coalesces repeated termination requests while a decision is pending.

## Accepted state and drafts

The accepted project is the saved-state and history authority. An edit owns a working project or document candidate; rendering observes its project weakly. Acquisition fails while another edit is live. Controls use short validated updates; undo grouping spans a gesture without retaining mutation authority. Features validate authored data and targets before committing; Roll controls and layer value/list edits remain available when calibration or rendering is unavailable. The edit publishes the version with history. Dropping the owner withdraws its preview without restoring an old accepted snapshot. Photo edits belong to the visit; Roll and Rendering edits belong to the document. Readers and history share immutable project versions, with copy-on-write mutation.

Crop, picker and reference-area edits have temporary interaction state. Apply submits the proposal; accepted state changes only after validation. Cancel discards the draft. Changing inspector tabs preserves these drafts; browsing to another photo retires photo-owned drafts without accepting them. Local gradients accept a validated release. Save/Save As captures the accepted project; Export captures accepted recipes on confirmation. Private drafts and pending searches do not participate in capture or unsaved-change decisions. New/Open/Close/Quit protect accepted changes through Save/Discard/Cancel; successful departure retires unfinished proposals without applying them. Cancelled transitions and failed Open preserve surviving owners, subject to ordinary pointer interruption. Later accepted edits remain live without rewriting the written baseline or frozen export request.

Measurement pickers add selection artwork to the current image, preserving source/positive mode, colour assets, composition and navigation. **View source** and navigation remain available while picking; Apply and Cancel keep those viewing choices. Measurements use original samples at their declared processing stage, independently of the displayed pixels. Calibration and WB capture their source, numerical inputs and edit target on Apply. A bare-light reference instead displays its independently prepared source. The feature then awaits a separate cancellable [measurement activity](../src/editor/measurement.rs), validates the result and records its edit. These UI actions retain their caller task separately in the photo visit; photo departure, reinterpretation, a new interaction or document replacement retires that continuation, including waits after measurement. Workers retain their captured inputs independently through cleanup. Normal source-view preferences remain independent. [Architecture](architecture.md) explains the execution and ownership model.

A rule beneath the stage tabs marks the active scope. Roll's tool shelf contains shared orientation, the stacked-frame Auto-crop action and an emphasized Analyze icon for negatives. Photo contains per-photo orientation, Crop and Dust, with a persistent Layers section. The active tool drawer sits below the shelf and above ordinary controls; clicking an active Crop/Auto-crop cancels its draft, and opening Crop/Auto-crop/Dust cancels gradient interaction without changing layer selection or accepted grades. Central activation replaces the unfinished interaction without accepting its draft. Interactive tools can be replaced; working modals reserve the interaction until completion or cancellation acknowledgement. [Analyze's roll review](roll-analysis.md#1-choose-what-to-sample) keeps browsing and exception edits separate from accepted support and grading.

Tools are modes with a beginning and an end. The [tool host](../src/editor/tool.rs) owns photo-scoped Crop, Dust, measurement picking and mask creation/refinement; batch Auto-crop, Analyze and [reference setup](flat-field.md#choose-the-illuminated-support) belong to the document. Analyze and batch Auto-crop retain cross-photo proposals there, while the visit owns their current-photo navigation and gestures. Browsing retires photo-scoped interactions; document tools retain their reference, review or proposals across visits. Hosting moves the box out for a callback and lends ordinary Editor access; an execution marker prevents restoring an ended, replaced or departed interaction. Activation replaces the current tool directly, and `end_tool()` ends whichever tool is current without synthesizing a button action. Tools compose their drawers from shared presentation components and own their input and painting. The footer receives identity, status and standard actions; complex interaction belongs in the drawer. Tools can supply filmstrip framing and warnings through its ordinary presentation inputs. One stable canvas layout serves the pass.

Tools compose [viewer controls, canvas layout and image painting](../src/editor/canvas.rs) with [navigation](../src/editor/navigation.rs) directly. Each tool chooses concrete source or photograph controls; the shared viewer returns navigation and action responses to its owner. Crop and batch Auto-crop keep private viewports for temporary framing; Dust and measurement picks share normal photo navigation; reference setup owns independent navigation. Tools pass a locally consumed egui response to navigation while retaining the original gesture/release response. Dust resizes its brush before navigation and handles strokes afterward; Crop omits navigation during its own drag. Fit targets the chosen viewport. Ordinary grading, layer and Rendering controls remain available alongside tools; working modals reserve input. Input-stage and orientation commands wait until the tool ends. Apply commits geometry; Cancel discards the draft without undoing previously accepted grading. Rendering follows effective image demand.

Crop continuously centers the frame during handle resizing, retaining screen-space pan and physical magnification. Pointer motion accounts for both moving screen edges so the grabbed handle follows the cursor, subject to source coverage, aspect constraints and pixel rounding. Fit crop is explicit. Option/Alt resizes around the crop center; Shift temporarily preserves the gesture-start aspect, and both combine.

### Input-stage changes

The Film negative / Digital selector sits below the tool shelf and drawer. Flat-field correction, Calibration, Gamma & balance and Toe & shoulder appear only for negatives; White balance and Exposure are shared. Switching input owns one scoped project edit, including any confirmation before discarding negative settings or accepted correction. Acceptance records the interpretation once without acquiring RAW headers, pixels or GPU readiness. Missing, corrupt or changed photographs report failures when consumed; they do not undo the selected stage. [Projects](projects.md#source-interpretation-and-identity) describes saved per-source developments and identity checks.

Undo/Redo restores accepted interpretation settings and correction. Reinterpretation keeps the selected photo and starts a fresh visit, resetting zoom and the Layers workspace. Tools, pickers, source loading and view products from the old visit retire together, so earlier results cannot present as the new interpretation. Their physical work and compatible keyed cache entries can finish independently. Consumers request resources through [captured source inputs](../src/source/input.rs), which own lazy resolution and preparation rather than requiring each feature to reconstruct that protocol.

### Gamma colour pivot

**Hold midtone colour** is on by default for a calibrated negative. The pin toggles compensation for future Gamma & Balance edits without changing the grade or history. Each gesture captures the selected photo when it first changes gamma; browsing does not change the grade or retarget a pending edit.

Manual gamma controls display their proposal while the shared GPU response runs off-thread. The photograph retains its accepted gamma/WB pair until the document-owned pivot publishes its result in a short mutation, provided the calculation's input settings and Roll WB still match. Its calculation survives browsing and tabs without reserving editing authority; unrelated accepted changes remain intact. Continuous adjustments share one history entry. Replay restores accepted values without compensating again. [Processing](processing.md#gamma-colour-pivot) owns the measurement and numerical model.

### Auto-crop is an explicit accepted edit

Photo's **Auto-crop** button requests a crop and straightening suggestion for the current photo. The [manual Crop tool](../src/editor/crop_editor.rs) owns the request and uses the result to seed its draft for refinement. Ending Crop retires the request; only Apply accepts the crop. Roll's **Auto-crop roll** proposes crops for the whole roll, independently of browsing or selection.

[Shared preparation](spatial-metadata.md) can infer nominal picture bounds from credible edges and common dimensions. Missing film alone does not veto that frame; unresolved geometry or conflicting picture evidence abstains and keeps previous crops. Photos that merely fall outside the captured selection are not failures; abstentions carry a warning cue with their specific reason. [Geometry](geometry.md#automatic-framing-consumes-measurements-not-sampling-support) explains the evidence boundary and inward margin.

The [batch tool](../src/editor/auto_crop.rs) owns its preparation waiter and draft; the visit owns its current-photo navigation and margin gesture. Shared region work remains independently document-owned. Apply validates and records the complete candidate synchronously; rejection keeps the review open. Margin edits leave accepted crops and dirty state unchanged, and stale targets reject the batch rather than silently shrinking its scope. Only applied crops persist; the margin is reused within this editor instance, not saved as a project setting.

### Provisional dust cleanup

Dust is direct canvas work, not a per-repair Apply/Cancel transaction. An unmodified click or stroke on free image content creates a source-pixel mask; release starts its donor search. Clicking an existing support or outline selects it and focuses the canvas. Shift adds to the mask, Option removes from it, and Delete/Backspace deletes the selected repair while the canvas has focus. Dragging a donor adjusts its stored offset; ordinary wheel input resizes the brush while Cmd/Ctrl-wheel zooms.

Mask admission is based on marked pixels and a bounded processing area, not a fixed side length. If a held stroke exceeds the budget, the draft and brush turn red and release discards the whole rejected stroke, preserving existing repairs rather than committing a truncated prefix.

Dust owns candidate search and reads the image graph's original-grid **unrepaired neutral AP1** tap: correction and input interpretation are upstream; Roll/layer grading and emulation are downstream. Rendering uses that same tap for the [healing stage](../src/photographic/stages/healing.rs), with common evaluation supplying inputs and managing reuse. Each repair stores one donor offset shared by its components. Healing is deterministic from that offset, and reopening never searches. Calculation identity follows neutral inputs and repair instructions, independently of downstream grading and viewport demand.

A released new or changed mask stays pending while a short, best-effort search finds one donor or rejects an impossible placement. Only the completed choice becomes an accepted repair and undo action; its donor is adjustable even if healing fails. The hosted [Dust interaction](../src/editor/dust.rs) owns drafts, input, overlays and [stroke task](../src/editor/dust.rs) handles; searches capture inputs and typed edit data separately from their task owner. Pending geometry follows task completion. Navigating away or ending Dust discards unfinished strokes without changing accepted repairs. Successful document departure also retires unfinished searches. Undo/Redo can proceed; a completed search publishes only if its captured neutral inputs and repair target still match. Search and healing use [scoped cancellation](architecture.md#cooperative-cancellation).

Accepted repair instructions participate in ordinary [project saving and dirty protection](projects.md#source-owned-dust-repairs). Rendering captures effective mask/donor instructions without retaining the interaction. Failed healing leaves the underlying pixels unchanged in previews and exports, preserving the accepted repair for later editing. Dust alone queries the same healing calculations and marks failed outlines in the error colour. Closing or replacing Dust retires its feedback queries while the photo's rendering demand remains intact. Export consumes the accepted set, including before the photo has been viewed.

## Colour selection prepares before acceptance

Rendering orders **Film texture**, **LUT**, **Film & paper**, then **Target**. Film texture groups coupled image-MTF/grain and separate halation. Its presets apply starting values while preserving enablement; edited values display Custom rather than retaining a preset identity.

The **LUT** section imports working-space 3D tables before **Film & paper**. Negative and Paper select the synthetic stock and print medium: [Spektrafilm](../assets/spektrafilm/README.md) supplies colour treatments; the [scalar B&W model](../src/looks/monochrome.rs) uses independently sourced materials. Negative None bypasses synthetic film formation, not source inversion. Paper None requires no selected negative. Starting a negative from bypass chooses Portra Endura for colour or Ilford Multigrade RC Deluxe for B&W. Both sections work on Digital and interpreted negatives.

Cross-family printing is unsupported by these providers, but all materials remain selectable. A family switch asks before clearing the negative or choosing a matching default paper; Cancel preserves accepted settings and earlier preparation. [LUT](../src/editor/look/lut.rs) and [Film & paper](../src/editor/look/film_and_paper.rs) own independent document-scoped preparations, leaving the workspace usable. Each validates its asset and device limits before a short edit of the current accepted project; Negative/Paper accept together as one action. A live edit conflict rejects the selection without retiring that edit's owner. Replacement or cancellation retires only that stage's acceptance rights; physical work retains its inputs through cleanup. Normal document departure retires pending choices. Failure or cancellation preserves accepted assets.

Stock/media names retain upstream attribution. Imported names remain as saved. Successful imports enter the session comparison list across document changes; identical numeric payloads share an entry. [Project storage](projects.md#embed-accepted-colour-assets) preserves the accepted numeric assets without relying on that list or the [derived cache](../assets/spektrafilm/README.md).

## Undo and redo

The Edit menu and Cmd–Z / Shift–Cmd–Z traverse one chronological history for the whole project. Roll, Photo and Rendering edits share it, including calibration, correction, accepted Analyze results and project-only photo removal. Navigation itself is not recorded; undoing a Photo edit activates its photo.

A continuous gesture is one action; numeric text commits, resets and commands are separate actions. Undo/Redo restores the retained accepted project version and advances history synchronously, without revalidation or resource preparation. Live edits reject conflicting history commands. Restoration preserves tools and their drafts within the surviving photo/document scopes; source or interpretation changes still retire affected scopes. Features validate retained targets before acceptance. Unapplied draft gestures are not history entries. Cancelled, rejected and unchanged edits add no entries; new accepted edits discard redo. The implementation retains up to 100 actions in memory through shared immutable project versions — not RAW pixels or rendered images. History is neither stored in the project file nor restored on relaunch.

## Numeric edits own their input

A displayed number is not necessarily a rounded stored value. Controls preserve precise values while the user reads or redraws them; only a deliberate edit may change processing. Slider and wheel changes are accepted as they arrive and grouped into one undo action; numeric text commits on Enter or ordinary focus loss. Incomplete numeric text stays private to its widget until completion. Moving between numeric fields completes the outgoing field; each completed field is independently undoable. Unfinished text follows its widget's lifetime; ordinary focus loss can complete it before the widget leaves. Escape cancels text editing before it can cancel an image tool.

Colour wheels drag relative to the accepted value; holding Shift gives ten-times finer motion without a jump when Shift changes. Popovers, numeric controls and toolbar gestures own input ahead of the canvas, so one action cannot also pan, pick or apply a draft.

## Dialogs return answers to their workflows

Notifications and decisions use one [dialog host](../src/editor/dialogs.rs). Features submit presentation and await a typed answer when their continuation needs one. Captured targets and subsequent actions stay in the feature's async workflow, retained by its document or photo visit. The host presents one dialog at a time until an explicit user answer; later submissions wait. Dropping a response waiter or succeeding on a later attempt does not erase an unread dialog.

A decision asks permission before changing or replacing something; a failure explains why an attempt did not complete. [Notice presentation](../src/editor/ui/feedback.rs) supplies named severity, glyph, restrained colour and expandable diagnostic Details. Required choices are not labelled operation failures. Analyze folds its Photo-reset consequence into its sampling-area review. Other import/analysis failures remain passive status rather than acknowledged messages.

## Working feedback follows what remains usable

Working feedback answers two questions: **what is happening, and what can I still do?** It is neutral activity, not a warning or a request for approval; its prominence comes from the operation's actual input restrictions.

| Work | Presentation and retained use |
| --- | --- |
| RAW import, project metadata Open/Save, correction estimation/validation | A centred working modal after a brief delay; the transaction already blocks the workspace |
| Export | A delayed working modal with finished-photo counts and Cancel; workspace, inspection and application panels stay blocked until completion or cancellation acknowledgement |
| Selected RAW preparation, including the first photo after Open | Nonblocking progressive display; cached previews remain usable while precise inputs prepare |
| Reference preparation, picker measurement | A delayed working modal; quick measurements complete without showing a dialog |
| Shared region preparation and running Analyze | A delayed working modal with counted or indeterminate progress and Cancel |
| Thumbnails, camera previews, scopes | Existing in-place image/card/scope feedback |

New installs an empty document without resource preparation. Input-stage acceptance is synchronous after any confirmation. History restores accepted choices immediately; source and correction consumers prepare asynchronously afterward, with failures local to those consumers. Selected-photo preparation remains nonblocking. Opening accepts [project metadata](projects.md#prepare-before-replacing) before loading photos; correction-resource failures appear in the [flat-field section](flat-field.md#save-the-accepted-result). Live scoped edits and reserved workspace input disable Save/Export; blocked requests are not replayed. Modal input reservations leave image requests, evaluation and presentation running. Save's document-owned write retains its input restriction without retiring photo work. Its caller awaits the write's result separately for Save/Discard/Cancel continuation. [Projects](projects.md#prepare-before-replacing) owns persistence validation and publication. Shared region preparation owns its feedback independently of the tools awaiting its result.

[Activities](../src/editor/activity.rs) are scoped handles carrying status, progress and cancellation feedback. Their owners retain them through completion or cancellation acknowledgement; destruction removes the feedback. Worker reporters and UI snapshots observe weakly.

[Workspace input](../src/editor/workspace_input.rs) observes scoped reservations held by modal activities, queued dialogs, Export forms, application panels and guarded file continuations. Each owner reserves input immediately and releases only its own reservation on destruction, independently of feedback delay. Workspace commands consult this shared authority rather than enumerating modal features. A modal's own controls route separately; input reservation neither grants edit authority nor prevents captured resource work from completing.

Escape invokes the active modal activity’s Cancel action, even before progress appears. Work without a Cancel action continues. Cancellation signals the workflow’s token; cancelling feedback replaces counts and the button until acknowledgement. [Working presentation](../src/editor/ui/progress.rs) renders modal or local feedback. Modal activities take precedence over local feedback; within either class the newest live activity is shown.

## Export photographs

File → **Export…** captures Selected photos or All photos in the displayed filmstrip order on confirmation. Choose an existing destination folder and a file format. JPEG and TIFF offer sRGB or Display P3. JPEG uses 8-bit RGB and adjustable quality; TIFF uses 16-bit RGB and lossless Deflate. EXR writes scene-linear AP0 in half or full float. [Colour and output](colour-and-output.md#render-and-encode-a-file) explains what each target renders.

Filenames use the source stem with an optional suffix. Keep both is the non-destructive default, Skip leaves existing files alone, and Replace requires explicit batch confirmation. The last accepted folder is an application preference. Export choices and target overrides do not edit the project or dirty state.

The [export action](../src/editor/export.rs) uses [shared photographic output](../src/editor/output.rs) to capture accepted recipes only for the requested photos, then processes full-resolution inputs one photo at a time without visiting them. Acquisition and execution validate each photo's actual interpretation, correction, numerical domain and resources for the selected target; unrelated photos do not participate in rendering admission. Cancel stops subsequent work and may stop the current photo at existing safe checkpoints; an in-flight native/encoder call can finish before acknowledgement. The modal stays open until cancellation is acknowledged. Completion summarizes exported, skipped, failed and remaining photos, with diagnostics and, on macOS, Show in Finder for completed output. Individual failures leave other photos exportable; published files are never rolled back as a batch.

## External edit sessions

An [external edit session](../src/editor/external_edit.rs) attaches a caller to an ordinary roll document. Opening one linked photograph restores the saved roll and selects that member; returning uses captured requested source IDs, independently of subsequent browsing or selection. The document scope owns caller access and return work. Import and history preserve that context; document replacement retires it. Accepted project snapshots contain editing state, not caller context.

[Host adapters](../src/editor/integrations.rs) resolve source access, save the project and publish the return. The shared coordinator captures accepted edits, requires a successful project save, then renders through shared photographic output and manages progress, cancellation and failure acknowledgement. Only the actual save result establishes the saved baseline; receipt publication does not mark edits saved. Ordinary file Save uses the normal project writer, independently of the external session.

The [Lightroom plug-in](../integrations/lightroom/README.md) uses an adapter that saves its canonical `.antitone` roll before rendering and publishes a JSON receipt after all requested images succeed. Each TIFF goes beside its requested original, independently of the project location. Save As can create a project copy without retargeting the canonical roll or TIFF folders. Completed files survive a later return failure; normal Export remains independent of the caller. Save Back captures accepted state, excluding private drafts and pending calculations; blocked requests require an explicit retry. Successful publication closes the Lightroom invocation only when ordinary document-close admission succeeds and no newer accepted edit remains; otherwise it keeps the document open for saving or discarding that work.

## Source inspection is a view

Developed-source inspection uses the same automatically lens-corrected development as the positive, bypassing inversion and photographic grading. Negative's **Viewing WB** defaults to Daylight, with As shot available for comparison; Digital already develops as-shot. The exposure control adjusts brightness relative to the [source-display baseline](colour-and-output.md#source-inspection-has-its-own-exposure-convention). Both controls apply to the canvas and source-preview thumbnails; calibrated filmstrip cards retain their positive grade. These viewing choices are transient: calibration, fitting, grades and exports still use the underlying developed pixels.

Source and positive caches have distinct identities: switching modes reacquires the appropriate source rather than grading an inverted positive as source light. Missing Negative calibration forces source-only presentation. Once calibration is usable, **View source** can select the positive; accepting calibration picks keeps the current viewing choice. Digital is ready without anchors.

## Immutable adjustment-layer routing

Adjustment layers are recipe values, not rendered pixels: selected and cached previews, thumbnails and scopes share complete processing snapshots, and each image owns its output/preparation state. Graph nodes capture calibration, correction, camera interpretation, Roll WB and ordered layer values where those arguments apply. Request freshness follows complete intent; unchanged upstream definitions remain reusable after downstream edits. [Processing](processing.md#one-complete-image-recipe) owns capture and graph identity.

The Layers section creates global, linear and radial layers above the selection and displays processing order bottom to top. One set of photographic controls targets the selected layer, named in the Layers header even when the list is collapsed. Layers resolves selection against the current stack, choosing the last layer when the target disappears and no target when the stack is empty. Strength, gradient inversion and radial Feather appear below the list. Empty stacks and ineffective controls remain visible but greyed out; the initial global layer has the same selection, reordering and deletion behavior as any other layer.

The creation icons and each gradient row's pencil enter the same photo-owned [mask tool](../src/editor/mask_editor.rs). The active pencil is amber; clicking it again ends editing. Global layers have no geometry action. Each row's power icon enables or bypasses the layer; bypass mutes its name and icon. Linear drawing starts at full effect and fades toward the release position; radial drawing starts at the centre. A valid release creates and selects a new adjustment, keeping the tool active for refinement and grading. A rejected creation stays armed for retry. A compact creation hint and errors sit above the ordinary sections; refinement uses the viewer's identity and Done action without a separate drawer.

The tool reads accepted geometry from the layer and retains only an unfinished gesture and its original target mask. Preview and release use current layer values; release changes only the target geometry, preserving later grading. Escape discards an unfinished refinement; Escape while idle or **Done** ends editing without undoing accepted changes. Changing the selected layer, leaving the photo or replacing the tool retires unfinished geometry. Handles and coverage follow tool activation, independently of Layers expansion or value adjustment. Drags outside handles navigate.

Rows drag to reorder with an insertion line. A drop moves the dragged layer relative to the target row in the current stack, preserving intervening edits. Double-click starts inline rename; Enter or focus loss commits it, and Escape cancels. Layers retains its selection and inline text independently of image-tool replacement. Click a layer row to select and focus it. Delete/Backspace requests deletion while the selected row has focus.

## A stable working view while grading

`WorkingView` captures intent, framing and target, retains detail/underlay products and presents coherent results. [Photographic requests](../src/photographic/graph/request.rs) author a display Value and acquisition layout for each frame the view chooses. [View requests](../src/viewer/view/rendering.rs) ask evaluation for drawable or pending images. Each [image product](../src/viewer/view/image.rs) owns its definition, region, pixels and pending commands. Evaluation chooses dependencies, fusion and reuse. Physical-view quality processes correction and inversion on originals before early AP1 reconstruction and grading. [Processing](processing.md#physical-view-processing-is-an-approximation) and [geometry](geometry.md#process-before-averaging) distinguish arithmetic agreement from sampling quality.

Same-layout calculations can finish coherently while subsequent demand coalesces to the latest capture. Superseded sampling layouts retire at safe checkpoints instead of delaying the newest viewport. Uploaded inputs use common evaluation and optional retention independently of photo visits; active products retain their required leases. Missing inputs use bounded source work. Detail and underlay advance together. While a new viewport needs source work, rendering can supply current-grade products at the previous frame without blocking on that acquisition. The viewer owns request retirement and publication; submitted work retains resources through completion. Navigation replaces coverage asynchronously from originals rather than recursively resampling a preview. A coarse whole-composition underlay fills exposed regions while detail is missing. [Architecture](architecture.md) owns the workflow and admission model.

A failed preview keeps usable previous pixels and reports the failure in the existing status bar, with diagnostics on hover. Editing remains available; a changed preview request starts a new attempt, and changing photos retires the failure. Pending replacements remain labelled as updating until current pixels arrive.

Tools compose the [canvas surface](../src/editor/canvas.rs) with the photograph or their own prepared image. The photo's [view state](../src/editor/canvas_view.rs) retains its render products; Flat owns its independent reference renderer and navigation, including before a photo is imported. The canvas records the actual painted `RenderedView`, mapping and size for scopes and navigation readouts. Photograph, Crop and sampling canvases request their chosen image through the same bounded rendering path. Crop editing maps the whole native-axis source beneath its affine draft, retaining content outside the frame.

Analyze's support review and independent reference views present original-sample source images through the shared renderer. Their owners follow captured source, composition, raster and target; leaving that demand or replacing its input scope rejects stale results. Measurement pickers retain ordinary physical-view rendering.

## Lightbox and single-photo views

Use the viewer bar or View → **Lightbox** (G) and **Single Photo** (E) to switch the canvas. Lightbox fills the available canvas width and shares the filmstrip's order, selection and preview types, including source previews before calibration. The viewer bar changes tile size without changing preview resolution. Filenames are available on hover.

Double-click a photo in either surface to open Single Photo. When a Lightbox tile has focus, arrows move by row and column; the filmstrip uses left and right arrows. Both surfaces scroll to reveal the active photo when it changes. Starting a canvas tool switches to Single Photo and stays there when the tool closes. Scopes measure the painted single-photo image.

## Fullscreen viewer

F or the fullscreen action enlarges the same feature-owned image, composition, viewport and interaction. The host makes the existing window borderless without entering a native fullscreen Space. The same viewer bar, including tool identity and Apply/Cancel/Done actions, floats and fades after inactivity; moving near the bottom or pressing Tab reveals it. A hidden bar leaves no invisible canvas hitbox. Crop handles, picking, reference/Analyze areas, Auto-crop margins, Dust and gradients remain interactive.

F or Escape returns to the workspace without accepting or discarding drafts. Navigation and viewing changes remain in their existing owners; explicit zoom retains physical magnification and the current source anchor across the resize. Tool completion or replacement leaves fullscreen active. Native restoration acknowledgement ends the window transition, not a photographic-state restoration. Source and document replacement retire their own image requests and interactions through the ordinary scopes.

## Fast feedback is not authoritative data

The editor uses several image representations for different jobs:

| Representation | Purpose |
| --- | --- |
| Full developed `Scan` | Full-resolution measurement, refinement and export |
| Camera-RGB proxy | Initial/pending feedback and uncalibrated/source-inspection thumbnails |
| Camera-proxy photograph | Positive navigation cards with current grading, look and framing |
| Physical-view calculation | Preview-quality photograph on the requested grid |
| Accepted target-linear view | Shared viewer/scope input; never graded or composed again |
| Original-sample regional result | Precision-tool refinement |
| Cached pending preview | Temporary display while the selected RAW prepares |
| Embedded camera preview | Filmstrip navigation placeholder and Digital brightness reference |

A compatible camera proxy supplies temporary feedback while the requested rendering prepares, even when it omits repairs. The photo and Flat canvases use the same [render request](../src/viewer/view/rendering.rs) as the working view: record when possible, otherwise produce on a worker. The canvas [owns the fallback](../src/editor/canvas_view.rs) and retires it when the preferred view is ready. Navigation, grading and geometry editing remain available. Precision consumers acquire original data asynchronously; preview pixels never become measurement or export inputs.

An embedded preview is already camera-rendered and carries a **Camera** badge. Its colour-managed luminance can seed [Digital starting brightness](processing.md#digital-starting-brightness), but RAW pixels still supply inversion, correction, measurements and exports. Completed current-recipe thumbnails supersede the camera image.

## Select and remove roll photos

Selection is temporary workspace state, separate from each source's accepted Photo settings. The visit owns the active photograph; its position follows the displayed order. The active photo always belongs to the selected set and has a lighter neutral card background plus the selection outline.

- Plain click makes that photo active, retaining or replacing the selection; arrow navigation uses the same rule.
- Cmd-click toggles membership without changing the active photo; Shift-click selects the contiguous range from the last plain-click anchor; Shift–Cmd–A reduces the selection to the active photo.

The filmstrip's [sort control](../src/editor/roll_order.rs) changes presentation only. Custom shows the saved sequence; File Name and Capture Time sort the same IDs and break ties using Custom. Arrow navigation, Shift-click ranges and export follow the displayed sequence. Sorting preserves the active photo, selection and range anchor.

A drag moves the selected photos as a block if it starts on a selected card, or one photo otherwise. A changed drop saves the displayed sequence with the move applied and switches to Custom. This replaces the previous Custom order as one undoable edit. Undo and Redo reveal the restored order in Custom.

Browsing replaces the visit only when the active photo changes. Selecting the active photo preserves its interaction and acquisition, including when source loading is pending or has failed. Adding scans to an existing roll preserves the active visit and selected input interpretation.

**Remove selected photos…** (a selected card's context menu, or Delete/Backspace with a filmstrip card focused) requires confirmation and participates in undo. It deletes each selected photo's source record and analysis support; RAW files stay on disk and project calibration and output remain. Undo restores the removed records. The next active position is the old displayed index in the shortened roll rather than the next surviving identity. Cancel preserves surviving drafts; confirmation retires the removed photo's work through its visit. Document-wide reviews retain their owners and validate current targets on Apply.

## Responsiveness and optional caches

Optional reuse of immutable full-resolution developments, uncorrected proxies and decoded camera references shares a byte-bounded CPU store; [retention settings](#retention-settings-and-pressure) control its limit in whole MiB, including zero. [Async acquisition](architecture.md#source-acquisition-and-retention) separates consumer leases from cache ownership. A compatible source hit avoids development, while correction, grade and crop remain independent consumer state. File identity is captured at Import/Open for persisted-cache validity, [not photographic identity](projects.md#source-interpretation-and-identity).

### Scheduling and readiness


Two [native owners](../src/source/loader.rs) at most serve selected/reference/Analyze and thumbnail development: each keeps worker-local mutable RAW handles and runs one synchronous unpack/development call at a time. Foreground work precedes visible misses, then eager work for unvisited sources; a pending foreground demand can start on a free owner, while background work starts only while no foreground demand exists. [Native scheduling](native-development.md#editor-scheduling) explains the thread policy and its latency tradeoff.

[Processed thumbnails](../src/thumbnails.rs) own the worker and processed image cache. The [editor thumbnail coordinator](../src/editor/thumbnails.rs) authors requests from the live document and shares its retention budget with camera previews. A `ThumbnailProvider` supplies images to roll views and reports their visible lookups to the document's scheduler. Each frame combines Lightbox and filmstrip demand once: the active visible Lightbox tile comes first, then other Lightbox tiles, then filmstrip-only requests. A photo visible in both shares its image and work.

Thumbnail and camera-preview work follows all document photos, including those never painted. An empty visible pass releases protection without suspending background preparation. Offscreen rendering uses available optional retention; CPU/disk work can finish without room. Camera-preview entries retain completion and measured image size after pixel eviction, avoiding extraction loops and permitting rewarming when room returns. Retiring a thumbnail GPU ticket preserves useful document-owned CPU/disk completion. [Architecture](architecture.md#source-acquisition-and-retention) describes their asynchronous acquisition and scoped publication.



### Positive thumbnail reuse

Positive navigation cards capture the complete [photographic intent](processing.md#one-complete-image-recipe). Camera-proxy graph quality interprets their small raster, omits original-coordinate repairs, and applies grading, texture, look and output target. This favors responsive browsing over original-sample fidelity. Use the main viewer or exports to inspect Dust repairs rather than inferring their accuracy from a small navigation card.

Camera preview display and Digital preparation share bounded JPEG/RGB8 extraction and decoding, including in-flight work. ICC profiles or explicit colour labels, including the matching DNG preview IFD, establish interpretation. The document's recorded starting EV survives pixel eviction; a Digital positive waits for that value before consuming shared proxy pixels.

Cards retain actual camera arguments and calculated values. Each update authors its desired graph: matching stored calculations supply it, while changed interpretation or correction can recompute from resident camera data. Document replacement releases cards; disk-cached proxies provide reuse across opens. When source, correction, framing and target match, a previous card stays visible and refresh waits briefly for the next request to stabilize. The refresh then proceeds in card priority order, coalescing intermediate grades without inspecting editing activity. Replacements retire older outputs for that source; shared look allocations are charged once across cards.

### Retention settings and pressure

Settings controls CPU/GPU optional retention in whole MiB, including zero, independently of project/grade state; [current defaults and units](../src/editor/cache_limits.rs) describe the choices. [Editor memory policy](../src/editor/memory.rs) composes the concrete stores' allowances. One editor-lived pressure task applies backoff independently of painting or document replacement. Warning pressure halves both optional retention budgets and pauses eager native development and processed-thumbnail work. Critical pressure sets both budgets to zero and retires thumbnail/camera-preview tickets. Neither changes the limits saved in Settings. Selected/visible essentials may exceed the optional limits. Visible missing cards remain protected even at zero optional retention, and a retained CPU source proxy alone does not suppress camera-preview loading until the processed image is available. Cached presentation products consume part of the GPU allowance; its remaining configured share supplies immutable-image retention, including uploaded inputs. Required inputs, scratch and active consumers have independent admission/lifetimes; [architecture](architecture.md) describes those boundaries.

## Build UI from shared presentation

Use the existing component families rather than copying their painting or interaction logic:

- [UI](../src/editor/ui.rs): the shared entry point; [theme](../src/editor/ui/theme.rs) owns colours and typography, [application](../src/editor/ui/application.rs) owns header/branding and panels.
- [Inspector](../src/editor/ui/inspector.rs), [parameter rows](../src/editor/ui/parameter.rs), [balance](../src/editor/ui/balance.rs): section hierarchy, row layout and value widgets.
- [Viewer framing](../src/editor/ui/viewer.rs), [tools](../src/editor/ui/tools.rs), [actions](../src/editor/ui/actions.rs), [icons](../src/editor/ui/icons.rs), [cards](../src/editor/ui/cards.rs): shared framing and interaction components.
- [Dialogs](../src/editor/ui/dialogs.rs), [feedback](../src/editor/ui/feedback.rs), [working](../src/editor/ui/progress.rs): shared modal layout, notices and progress presentation.

[Editor](../src/editor.rs) composes the workspace, orders competing shortcuts, and dispatches menu commands after controls have handled input. Features own their bindings and actions; focused controls handle their own keys in place.

Features compose `editor::ui` components through body closures and ordinary egui layout, owning availability, accepted/draft state and dispatch. [ParameterRow](../src/editor/ui/parameter.rs) receives an explicit identity, label, bounds and value options together at its call site; shared spacing and gestures stay inside the component. [BalanceControl](../src/editor/ui/balance.rs) composes those rows with a wheel, while [white balance](../src/editor/white_balance.rs) owns its section framing and actions. Non-numeric controls use the fixed-name inspector row where they share that layout, and ordinary themed egui controls inside it.

Shared value widgets edit local values and return interaction responses; features validate and accept changed values through short mutations. Completed inputs are independently undoable, with continuous gestures grouped as one action. Inspector dropdowns fill the section width, with truncation rather than value-dependent sizing.

Use [`Segments`](../src/editor/ui/segments.rs) for a short set of mutually exclusive choices. The compact layout uses icons with tooltips; the fill-width layout shows labels. Give each segment an accessible name and keep commands and popup actions outside the frame.

### Dialog bodies and actions

Supply modal content and typed actions separately to [`Dialog`](../src/editor/ui/dialogs.rs). It scrolls the body as needed and keeps the action footer visible. The footer follows macOS order: Cancel leads a row or sits last in a stack. Use primary emphasis for a recommended action and the destructive role for an action that discards work or replaces data. Initial focus and Escape are explicit and independent of visual emphasis.

### Inspector density and value grammar

The inspector uses its own spacing scale, tighter than the rest of the application: [Inspector](../src/editor/ui/inspector.rs) owns the inspector row height, item gap, section gap, slider height and numeric width, and `inspector_spacing` applies them to the column. Sections separate with a separator and a short gap; a section header carries its disclosure, its changed cue, a reset and related section tools. Anything added to the inspector should take its height and gaps from these constants rather than choosing its own. Consecutive value rows use a tighter gap than sections: their tracks read as one block only when the neighbouring rows are closer than the separators around them.

A parameter is one row: a right-aligned name, a flexible track and a fixed-width editable value. The compact name column aligns track starts across sections; longer names truncate. A unit is a muted suffix inside the value field, not a column. It stays out of the text while editing, and its presence does not move any column boundary. Reset is a name gesture rather than a control — double-click or Option-click the name — backed by a **Reset to default** item in the value's context menu; a section reset still covers its whole group. Keep the row to those three parts: a per-row icon competes with the value for the narrow column.

Dragging a track is absolute, except when the press begins with Shift: that press holds the accepted value and the remaining travel applies at one-tenth rate, matching the numeric field's fine adjustment. Keyboard steps follow the same fraction.

White balance and gamma balance share one body: the two axis rows, and a label-free hue wheel at the right end of the block, with the eyedropper and section reset in the header. The axes use the same parameter grid: the wheel shortens the tracks from the right without moving their starts. The wheel is a correction surface rather than a measurement device, so it sizes with the panel width and drags relatively from the press origin — Shift gives ten-times finer motion and a double-click resets the balance. Precision and stepping stay with the axis rows and their numeric fields.

[Scopes](scopes.md) sit above the stage tabs. The resize handle adjusts a persisted height-to-width ratio; double-clicking restores the default so the stage controls stay reachable.

The inspector reserves a band for the scroll bar because dense rows put values and header actions near the right edge, where a floating bar would take their pointer input.

Controllers and hosted tools use ordinary `&mut Editor` and raw `egui::Ui`; a separate borrowed [UiContext](../src/application.rs) supplies execution for continuations. A continuation can capture typed feature state directly rather than recover the current interaction through Editor. `spawn` defers its factory until the host polls outside painting. [Architecture](architecture.md) owns that execution model.

### Contextual status indicators

Use the named [contextual status pattern](../src/editor/ui/feedback.rs) for a local condition, not a feature-specific badge or global notification. `ContextualStatus` carries display-ready reason and level; cards and tool warning identities reuse the same primitive, and controllers own the condition and its retirement.

| Condition | Presentation |
| --- | --- |
| Pending work | Neutral activity; a previous accepted thumbnail awaiting replacement |
| Missing/abstained result | Warning with the specific reason: Auto-crop lacks frame evidence, Analyze lacks sampling support |
| Failed operation | Error with the specific reason: a repair mask reaches the edge of the processed area |
| Current/complete | No indicator; the presence of repairs is not a warning |

Error takes precedence over warning, then activity, and concurrent reasons combine into one indicator per surface. Give icon-only status a meaningful accessible name; retire each condition when its owning work completes.

### Tooltips and contextual help

A tooltip earns its place by answering a question about the hovered item that its visible label does not already answer. **No tooltip is the default for a self-explanatory labelled control.** Do not attach documentation to a widget just because the API accepts help text.

- **Useful content:** name an icon-only action, give its shortcut, clarify a non-obvious unit or effect, explain how an unavailable action becomes available, or reveal a full path/truncated value.
- **Keep it local and brief:** normally one short sentence. Start actions with a verb; describe the user's result rather than the implementation.
- **Delete repetition:** do not restate a visible label, stack generic input instructions on built-in widget help, or enumerate everything an action leaves unchanged.
- **Use the right home:** gesture lists belong in the relevant help popover; processing theory and cache guarantees belong in the owning guide, not a tooltip. A filmstrip card reveals its path, not preview internals or selection instructions.
- **Never hide a requirement:** task instructions, destructive consequences and failures must have visible presentation. Tooltips supplement a visible marker; they never become the sole indication that something failed.

Before adding help, ask: *What is unclear here? Does this text resolve it? Is this the right place? Can it be deleted or shortened?* Verify the composed feature as well as the shared component, including disabled state. Use the shared/egui tooltip path, not feature-local hover windows. The content policy follows [Apple's help guidance](https://developer.apple.com/design/human-interface-guidelines/offering-help) and [Fluent's tooltip guidance](https://fluent2.microsoft.design/components/web/react/core/tooltip/usage); [W3C's hover/focus guidance](https://www.w3.org/WAI/WCAG22/Understanding/content-on-hover-or-focus.html) is a behavioral checklist when working on the interaction itself.

### Typography roles

Reading text should be quiet; short instrument markings give the workspace its camera-body character. The editor embeds [IBM Plex Sans](../assets/fonts/README.md): Text for ordinary controls, explanations, filenames and numeric entry; Medium for stage tabs, active tool names and compact readouts. [Theme](../src/editor/ui/theme.rs) installs the fonts and owns `instrument_label` and `instrument_readout`; [text actions](../src/editor/ui/actions.rs) share the readout role. Use those roles rather than restyling per feature. The Arimo wordmark remains separate artwork; stage tabs share equal breathing room around their labels rather than equal widths.

### Colour roles

Colour expresses interaction state, not feature ownership. The shared UI theme installs the editor colours once; ordinary egui widgets and shared components inherit its selection, active-state and link colours. Custom interaction painting reads the applicable role from `ui.visuals()` or `ui.style().interact(&response)` rather than copying brand RGB values. Contact-sheet fills and image wells stay deliberately neutral; photographic wheels and scope plots keep colours that represent image data.

## Working on this area

Start with [editor orchestration](../src/editor.rs), [Photo-stage composition](../src/editor/photo_ui.rs), [control updates and mutation authority](../src/editor/editing.rs) and [tool ownership and replacement](../src/editor/tool.rs), then the feature owner: [viewer](../src/editor/viewer_ui.rs), [picking](../src/editor/picking.rs), [measurement](../src/editor/measurement.rs), [crop](../src/editor/crop_editor.rs), [auto-crop](../src/editor/auto_crop.rs), [analysis](../src/editor/analysis.rs), [correction](../src/editor/flat.rs), [filmstrip](../src/editor/filmstrip.rs), [pending previews](../src/editor/selected_preview.rs) or [camera placeholders](../src/editor/thumbnails/embedded.rs).

Use both component and composed-event checks from [development](development.md#shared-editor-ui-checks).