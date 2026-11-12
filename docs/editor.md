# Editor state and interaction

The editor connects photographic settings, image tools and asynchronous work. Its central rule is simple: **a view may be temporary, but accepted photographic state must remain coherent**.

Roll, Photo and Rendering are editing scopes, not separate pipelines. Roll selects Negative or Digital interpretation and owns shared grading plus Negative's scan setup; Photo owns individual grading and composition — its turn/flip setting starts from the orientation the camera recorded, and saved projects keep their stored setting; Rendering owns Film texture, LUT, Film & paper and the preview target, which also initializes rendered export. Changing tabs changes available controls, not the image's processing. [Architecture](architecture.md) describes how these scopes are owned and disposed.

The editor uses a neutral dark workspace with amber identity and active-state accents. The shared application header presents the film-edge wordmark and project identity without making the wordmark an interactive control. [Branding assets](../assets/branding/README.md) explain the artwork and regeneration.

## Application menus and settings

On macOS, native AppKit menus provide the conventional Antitone, File, Edit, View, Window and Help groups. File → **Add Scans…** adds camera RAW source references without moving or modifying originals; Export belongs under File. Settings, About and local workflow help open focused in-editor panels. Settings controls optional CPU/GPU retention in whole MiB, including zero and explicit reset — see [Responsiveness and optional caches](#responsiveness-and-optional-caches); those limits are not a process-memory cap.

The [shared menu model](../src/platform/menu.rs) supplies actions, grouping and shortcuts to native and egui presentation; the [host](../src/platform/host.rs) owns native menu lifetime and forwards activations through winit's event-loop proxy, restoring a minimized editor so panels do not open invisibly. The editor revalidates menu activations against their input, decision and operation constraints; an active tool alone does not prevent activating another. Text-targeting Edit actions receive pending input before focus changes. File actions are unavailable during an unfinished control edit; finish or cancel it before invoking Save. The egui fallback uses the same commands and state when native menus are not installed; only the macOS host is implemented.

### Native quit negotiation

Dock and system Quit enter the same unsaved-change guard as menu Quit: the editor reports live close continuations, and busy work or active drafts decline the request without accepting or discarding anything. The [termination bridge](../src/platform/termination.rs) uses AppKit's deferred decision — winit owns its delegate, so the bridge adds only the missing optional decision method — and coalesces repeated termination requests while a decision is pending.

## Accepted state and drafts

The accepted project is the saved-state and history authority. An edit owns a working project or document candidate; rendering observes its project weakly. Acquisition fails while another edit is live; actions can instead retire their unfinished control owner before starting. Features validate their values before committing; the edit publishes the version with history. Dropping the owner withdraws its preview without restoring an old accepted snapshot. Photo edits belong to the visit; Roll and Rendering edits belong to the document. Readers and history share immutable project versions, with copy-on-write mutation.

Crop, picker and reference-area edits have temporary interaction state. Apply submits the proposal; accepted state changes only after validation. Cancel discards the draft. Tabs and outside clicks do not implicitly accept or discard those drafts. Local gradients instead preview a captured gesture and accept a validated release. Save/Export never consume half-applied geometry.

Tools deliberately use different views to reach what they measure: Dmin/Dmax expose uncropped oriented source so film margins are reachable; a bare-light reference exposes untransformed source; gamma/WB picks retain the working photograph's composition while their fitting uses declared measurement inputs. Choose the photo before opening a calibration or neutral picker; ordinary picking stays on that photo. Calibration and WB capture their source, numerical inputs and edit target when opened. Apply ends the picker and its temporary view, returning the selected source coverage. The feature then awaits a separate cancellable [measurement activity](../src/editor/measurement.rs), validates the result and records its edit. These UI actions retain their caller task separately in the photo visit; photo departure, a new interaction or a document transaction retires that continuation, including waits after measurement. Workers retain their captured inputs independently through cleanup. Normal source-view preferences remain independent. [Architecture](architecture.md) explains the execution and ownership model.

Fixed tool shelves sit below the stage tabs. Roll contains shared orientation, Analyze and Auto-crop; Photo contains turn/flip, Crop and Dust, with a persistent Layers inspector section. Active controls appear above ordinary inspector sections; clicking an active Crop/Auto-crop cancels its draft, and opening Crop/Auto-crop/Dust cancels gradient interaction without changing layer selection or accepted grades. Central activation replaces the unfinished interaction without accepting its draft. Interactive tools can be replaced; working modals reserve the interaction until completion or cancellation acknowledgement. [Analyze's roll review](roll-analysis.md#1-choose-what-to-sample) keeps browsing and exception edits separate from accepted support and grading.

Tools are modes with a beginning and an end. The [tool host](../src/editor/tool.rs) owns photo-scoped Crop, Dust, measurement picking and gradient creation; batch Auto-crop, Analyze and [reference setup](flat-field.md#choose-the-illuminated-support) belong to the document. Batch tools survive browsing. Hosting moves the box out for a callback and lends ordinary Editor access; an execution marker prevents restoring an ended, replaced or departed interaction. Activation replaces the current tool directly, and `end_tool()` ends whichever tool is current without synthesizing a button action. Tools compose their drawers from shared presentation components and own their input and painting. The footer receives identity, status and standard actions; complex interaction belongs in the drawer. Tools can supply filmstrip framing and warnings through its ordinary presentation inputs. One stable canvas layout serves the pass.

Tools compose [viewer controls, canvas layout and image painting](../src/editor/canvas.rs) with [navigation](../src/editor/navigation.rs) directly. Each tool chooses concrete source or photograph controls; the shared viewer returns navigation and action responses to its owner. Crop and batch Auto-crop keep private viewports for temporary framing; Dust and working-image picks share normal photo navigation. Source/native picks and reference setup own temporary navigation. Tools pass a locally consumed egui response to navigation while retaining the original gesture/release response. Dust resizes its brush before navigation and handles strokes afterward; Crop omits navigation during its own drag. Fit targets the chosen viewport. Ordinary input, grading, layer and rendering edits wait until the active tool ends; tool-owned controls, navigation and replacement remain available outside working modals. Apply commits geometry; Cancel discards the draft without undoing previously accepted grading. Crop shares typed edit data with Auto-crop through `Rc<RefCell<CropEdit>>`, separately owning its viewport and task handle. Rendering follows effective image demand. The photo's [contextual mask editor](../src/editor/mask_editor.rs) owns selected-mask handles and unfinished refinement, independently of tool modes.

Crop continuously centers the frame during handle resizing, retaining screen-space pan and physical magnification. Pointer motion accounts for both moving screen edges so the grabbed handle follows the cursor, subject to source coverage, aspect constraints and pixel rounding. Fit crop is explicit. Option/Alt resizes around the crop center; Shift temporarily preserves the gesture-start aspect, and both combine.

### Input-stage changes

The Roll input selector owns one scoped project edit, including any awaited confirmation before discarding Negative settings or accepted correction. Acceptance records the interpretation once without acquiring RAW headers, pixels or GPU readiness. Missing or incompatible photographs report failures when consumed; they do not undo the selected stage. [Projects](projects.md#compatibility-is-about-the-processing-basis) describes the saved development facts and compatibility checks.

Project versions retain their interpretation's acquired scan basis across Undo/Redo; accepting a new stage starts a fresh scope. The photo visit retains its selected source, viewport and layer workspace across reinterpretation. Captured-input owners — tools, pickers, source loading and view products — retire together, so earlier results cannot present as the new interpretation. Their physical work and compatible keyed cache entries can finish independently. Consumers request resources through [captured source inputs](../src/source/input.rs), which own lazy resolution and preparation rather than requiring each feature to reconstruct that protocol.

### Gamma colour pivot

The pin beside gamma's neutral picker opens the ordinary area picker. Apply captures evidence; Cancel preserves an existing anchor. The pin is outlined while choosing and filled when locked; clicking it cancels selection or releases the lock. Capture and release are transient, outside dirty state and history. The document retains accepted evidence across photo visits, retiring it when its source leaves or its upstream measurement basis changes. Crop, view, look and output changes preserve it.

Manual gamma controls display their proposal while the shared GPU response runs off-thread. The photograph uses a coherent prepared gamma/WB pair; completing the gesture accepts both together with one history entry. The document-owned edit retains its calculation across browsing and tabs; retirement removes its publication rights. Replay restores accepted values without compensating again. [Processing](processing.md#picked-colour-gamma-pivot) owns the measurement and numerical model.

### Auto-crop is an explicit accepted edit

Photo's **Auto-crop** button proposes the current photo's crop and straightening into the manual Crop draft, then permits ordinary refinement. Roll's **Auto-crop roll** and the filmstrip context menu propose crops for a whole set, independently of browsing or selection.

[Shared preparation](spatial-metadata.md) can infer nominal picture bounds from credible edges and common dimensions. Missing film alone does not veto that frame; unresolved geometry or conflicting picture evidence abstains and keeps previous crops. Photos that merely fall outside the captured selection are not failures; abstentions carry a warning cue with their specific reason. [Geometry](geometry.md#automatic-framing-consumes-measurements-not-sampling-support) explains the evidence boundary and inward margin.

The [batch tool](../src/editor/auto_crop.rs) owns its preparation waiter, draft, navigation and margin gestures; shared region work remains independently document-owned. Apply validates and records the complete candidate synchronously; rejection keeps the review open. Margin edits leave accepted crops and dirty state unchanged, and stale targets reject the batch rather than silently shrinking its scope. Only applied crops persist; the margin is reused within this editor instance, not saved as a project setting.

### Provisional dust cleanup

Dust is direct canvas work, not a per-repair Apply/Cancel transaction. An unmodified click or stroke on free image content creates a source-pixel mask; release starts its donor search. Clicking an existing support or outline selects it; Shift adds, Option removes, Delete deletes that repair rather than removing the photo. Dragging a donor adjusts its stored offset; ordinary wheel input resizes the brush while Cmd/Ctrl-wheel zooms.

Mask admission is based on marked pixels and a bounded processing area, not a fixed side length. If a held stroke exceeds the budget, the draft and brush turn red and release discards the whole rejected stroke, preserving existing repairs rather than committing a truncated prefix.

Dust owns candidate search and reads the image graph's original-grid **unrepaired neutral AP1** tap: correction and input interpretation are upstream; Roll/layer grading and emulation are downstream. Rendering uses that same tap for [healing image work](../src/gpu/repairs/definition.rs), with common execution owning solving and reuse. Each repair stores one donor offset shared by its components. Healing is deterministic from that offset, and reopening never searches. Calculation identity follows neutral inputs and repair instructions, independently of downstream grading and viewport demand.

A released new or changed mask stays pending while a short, best-effort search finds one donor or rejects an impossible placement. Only the completed choice becomes an accepted repair and undo action; its donor is adjustable even if healing fails. The hosted [Dust interaction](../src/editor/dust.rs) owns drafts, input, overlays and [stroke task](../src/editor/dust.rs) handles; searches capture inputs and typed edit data separately from their task owner. Pending geometry follows task completion. Navigating away or ending Dust discards unfinished strokes without changing accepted repairs. Save, Export, document replacement and Undo/Redo are unavailable until pending strokes resolve. Search and healing use [scoped cancellation](architecture.md#cooperative-cancellation).

Accepted repair instructions participate in ordinary [project saving and dirty protection](projects.md#source-owned-dust-repairs). Rendering captures effective mask/donor instructions without retaining the interaction; closing or replacing Dust leaves the photo's rendering demand intact. Viewer feedback follows the current requested/completed graph, not a separate repair cache; camera-proxy placeholders do not establish completed healing. TIFF/EXR export consumes the accepted set, including before the photo has been viewed.

## Colour selection prepares before acceptance

Rendering orders **Film texture**, **LUT**, **Film & paper**, then **Target**. Film texture groups coupled image-MTF/grain and separate halation. Its presets apply starting values while preserving enablement; edited values display Custom rather than retaining a preset identity.

The **LUT** section imports working-space 3D tables before **Film & paper**. Negative and Paper select the synthetic stock and print medium: [Spektrafilm](../assets/spektrafilm/README.md) supplies colour treatments; the [scalar B&W model](../src/looks/monochrome.rs) uses independently sourced materials. Negative None bypasses synthetic film formation, not source inversion. Paper None requires no selected negative. Starting a negative from bypass chooses Portra Endura for colour or Ilford Multigrade RC Deluxe for B&W. Both sections work on Digital and interpreted negatives.

Cross-family printing is unsupported by these providers, but all materials remain selectable. A family switch asks before clearing the negative or choosing a matching default paper. Cancel leaves accepted settings and earlier preparation intact. One [document-owned preparation](../src/editor/look.rs) checks the confirmed treatment and device limits before accepting both material choices as one undoable edit. During preparation, that section can replace its pending choice; the other section waits. Cancelling preparation retires acceptance rights while running work retains its inputs through cleanup. Failure or cancellation preserves both accepted assets.

Stock/media names retain upstream attribution. Imported names remain as saved. Successful imports enter the session comparison list across document changes; identical numeric payloads share an entry. [Project storage](projects.md#embed-accepted-colour-assets) preserves the accepted numeric assets without relying on that list or the [derived cache](../assets/spektrafilm/README.md).

## Undo and redo

The Edit menu and Cmd–Z / Shift–Cmd–Z traverse one chronological history for the whole project. Roll, Photo and Rendering edits share it, including calibration, correction, accepted Analyze results and project-only photo removal. Navigation itself is not recorded; undoing a Photo edit activates its photo.

A continuous gesture is one action; numeric text commits, resets and commands are separate actions. Undo/Redo restores the retained accepted project version and advances history synchronously, without revalidation or resource preparation. Live edits reject conflicting history commands. An admitted restoration retires the current tool's unfinished interaction before replacing accepted state; compatible photo navigation, acquired facts and resources remain available. Unapplied draft gestures are not history entries. Cancelled, rejected and unchanged edits add no entries; new accepted edits discard redo. The implementation retains up to 100 actions in memory through shared immutable project versions — not RAW pixels or rendered images. History is neither stored in the project file nor restored on relaunch.

## Numeric edits own their input

A displayed number is not necessarily a rounded stored value. Controls preserve precise values while the user reads or redraws them; only a deliberate edit may change processing. Slider and wheel gestures preview values until completion; numeric text commits on Enter or ordinary focus loss. Incomplete numeric text stays private to its widget until completion. Moving between numeric fields completes the outgoing field; each completed field is independently undoable. Scope departure abandons an unfinished edit; ordinary focus loss can complete it before departure. Escape cancels text editing before it can cancel an image tool.

Colour wheels drag relative to the accepted value; holding Shift gives ten-times finer motion without a jump when Shift changes. Popovers, numeric controls and toolbar gestures own input ahead of the canvas, so one action cannot also pan, pick or apply a draft.

## Dialogs return answers to their workflows

Notifications and decisions use one [dialog host](../src/editor/dialogs.rs). Features submit presentation and await a typed answer when their continuation needs one. Captured targets and subsequent actions stay in the feature's async workflow, retained by its document, photo visit or file-action owner. The host presents one dialog at a time until an explicit user answer; later submissions wait. Dropping a response waiter or succeeding on a later attempt does not erase an unread dialog.

A decision asks permission before changing or replacing something; a failure explains why an attempt did not complete. Save/Discard and replacement choices use shared [decision presentation](../src/editor/ui/dialogs.rs). [Notice presentation](../src/editor/ui/feedback.rs) supplies named severity, glyph, restrained colour and expandable diagnostic Details. Required choices are not labelled operation failures. Analyze folds its Photo-reset consequence into its sampling-area review. Other import/analysis failures remain passive status rather than acknowledged messages.

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

New installs an empty document without resource preparation. Input-stage acceptance is synchronous after any confirmation. History restores accepted choices immediately; source and correction consumers prepare asynchronously afterward, with failures local to those consumers. Selected-photo preparation remains nonblocking. Opening accepts [project metadata](projects.md#prepare-before-replacing) before loading photos; correction-resource failures appear in the [flat-field section](flat-field.md#save-the-accepted-result). Save/Export are unavailable until control edits, active tools and working modals finish; blocked requests are not replayed. Save's document-owned write retains its modal input restriction without retiring photo work or suspending source completion. Its caller awaits the write's result separately for Save/Discard/Cancel continuation. [Projects](projects.md#prepare-before-replacing) owns persistence validation and publication. Shared region preparation owns its feedback independently of the tools awaiting its result.

[Activities](../src/editor/activity.rs) are scoped handles carrying status, progress and cancellation feedback. Their owners retain them through completion or cancellation acknowledgement; destruction removes the feedback. Worker reporters and UI snapshots observe weakly.

[Workspace input](../src/editor/workspace_input.rs) observes scoped reservations held by modal activities, queued dialogs, Export forms, application panels and guarded file continuations. Each owner reserves input immediately and releases only its own reservation on destruction, independently of feedback delay. Workspace commands consult this shared authority rather than enumerating modal features. A modal's own controls route separately; input reservation neither grants edit authority nor prevents captured resource work from completing.

Cancel signals the workflow's cooperative cancellation token. Export and Analyze retain their handles until acknowledgement; cancelling feedback replaces counts and the Cancel action. [Working presentation](../src/editor/ui/progress.rs) renders modal or local feedback. Modal activities take precedence over local feedback; within either class the newest live activity is shown.

## Export photographs

File → **Export…** writes Selected photos or All photos in filmstrip order. Choose an existing destination folder, TIFF or EXR, and format-specific output settings. TIFF offers sRGB/Display P3 with 16-bit RGB and lossless Deflate; EXR has fixed scene-linear AP0 and half/full float precision. [Colour and output](colour-and-output.md#tiff-and-exr-serve-different-purposes) explains the different rendering semantics.

Filenames use the source stem with an optional suffix. Keep both is the non-destructive default, Skip leaves existing files alone, and Replace requires explicit batch confirmation. The last accepted folder is an application preference. Export choices and target overrides do not edit the project or dirty state.

The [export action](../src/editor/export.rs) captures accepted recipes only for the requested photos, then processes full-resolution inputs one photo at a time without visiting them. Acquisition and execution validate each photo's actual interpretation, correction, numerical domain and format-specific resources; unrelated photos do not participate in rendering admission. Cancel stops subsequent work and may stop the current photo at existing safe checkpoints; an in-flight native/encoder call can finish before acknowledgement. The modal retains cancelling feedback, and Escape/outside clicks do not dismiss running work. Completion summarizes exported, skipped, failed and remaining photos, with diagnostics and Show in Finder for completed output. Individual failures leave other photos exportable; published files are never rolled back as a batch.

## Source inspection is a view

Developed-source inspection uses the same automatically lens-corrected development as the positive, bypassing inversion and photographic grading. Negative's **Viewing WB** defaults to Daylight, with As shot available for comparison; Digital already develops as-shot. The exposure control adjusts brightness relative to the [source-display baseline](colour-and-output.md#source-inspection-has-its-own-exposure-convention). Both controls apply to the canvas and source-preview thumbnails; calibrated filmstrip cards retain their positive grade. These viewing choices are transient: calibration, fitting, grades and exports still use the underlying developed pixels.

Source and positive caches have distinct identities: switching modes reacquires the appropriate source rather than grading an inverted positive as source light. Missing Negative calibration forces source-only presentation; completing a usable Dmin/Dmax pair returns to the positive. Digital is ready without anchors.

## Immutable adjustment-layer routing

Adjustment layers are recipe values, not rendered pixels: selected and cached previews, thumbnails and scopes share complete processing snapshots, and each image owns its output/preparation state. Graph nodes capture calibration, correction, camera interpretation, Roll WB and ordered layer values where those arguments apply. Request freshness follows complete intent; unchanged upstream definitions remain reusable after downstream edits. [Processing](processing.md#one-complete-image-recipe) owns capture and graph identity.

The Layers section creates global, linear and radial layers above the selection and displays processing order bottom to top. One set of photographic controls targets the selected layer, named in the Layers header even when the list is collapsed. Strength, gradient inversion and radial Feather appear below the list. Empty stacks and ineffective controls remain visible but greyed out; the initial global layer has the same selection, reordering and deletion behavior as any other layer.

The creation icons enter a one-shot drawing tool, identified in its drawer and viewer bar with Cancel available. Linear drawing starts at full effect and fades toward the release position; radial drawing starts at the centre. A valid released stroke creates and selects a new adjustment with grading immediately available and no Apply step. A rejected creation stays armed for retry.

Existing gradients are directly editable through selected-mask handles; grading controls and refinement remain available without entering a mode. The photo-owned editor presents accepted geometry at rest and a draft during each gesture. Release accepts a validated edit; Escape discards the unfinished gesture. Changing the selected layer, leaving the photo or opening a tool retires that draft. Drags outside handles navigate. Contextual artwork hides during tools, when Layers is collapsed or while adjusting values, preserving the coverage preference; active refinement keeps its guides. Creation and refinement share [geometry and painting](../src/editor/mask_editor.rs).

Rows drag to reorder with an insertion line, double-click renames inline, and Delete/Backspace on the Photo stage removes the selected layer through a stale-safe confirmation.

## A stable working view while grading

`WorkingView` retains its actual captured request and related detail/underlay products for positive and source viewing. Quality-aware photographic graphs acquire signed linear AP1 from immutable developed-source leases; correction and inversion process original camera pixels before footprint filtering. Fixed-view grading can reuse those values when their definitions and storage cover the requested calculation. [Processing](processing.md#physical-view-processing-is-an-approximation) and [geometry](geometry.md#process-before-averaging) distinguish arithmetic agreement from sampling quality.

Same-layout calculations can finish coherently while subsequent demand coalesces to the latest capture. Superseded sampling layouts retire at safe checkpoints instead of delaying the newest viewport. The photo visit owns camera preparation and admitted camera pixels independently of its view; layout, grading and inspection changes preserve them, while source or visit replacement retires them. Pressure releases camera storage and lets views use bounded source execution. Views observe preparation readiness without retaining its pixels. Detail and underlay advance together; resident grading can progress independently of blocked source acquisition. Each view owns its acquisition and publication rights, while submitted work retains its resources through retirement. Navigation replaces support asynchronously from originals rather than recursively resampling a preview. A coarse whole-composition underlay fills exposed regions while detail is missing. [Architecture](architecture.md) owns the workflow and admission model.

Tools compose the [canvas surface](../src/editor/canvas.rs) with the photograph or their own prepared image. The photo's [view state](../src/editor/canvas_view.rs) retains its render products; Flat owns its independent reference renderer and navigation, including before a photo is imported. The canvas records the actual painted `RenderedView`, mapping and size for scopes, navigation readouts and Look-error reporting; a retained photograph cannot supply errors for a different displayed image. Repaired crop and precision previews use the same bounded original-coordinate acquisition as the working view; a cold repaired preview shows updating feedback until suitable support is ready. Crop editing maps the whole native-axis source beneath its affine draft, retaining content outside the frame.

The **Original-resolution reference** toggle requests original-sample GPU grading reconstructed at the physical view. Precision tools also present their completed original-sample images through the shared renderer, with native rectangular or affine framing as requested. These are purpose-bound products: leaving a tool or changing its inputs rejects stale results. Comparison never silently sharpens normal viewing; completed references enter optional retention after comparison ends.

## Photo-only fullscreen inspection

The fullscreen action beside the zoom controls, or F, makes the current photograph a full-display inspection view: the host makes the existing window borderless without entering a native fullscreen Space. Shared zoom/source controls float over the image and fade after inactivity; moving near the bottom or pressing Tab reveals them, and a hidden bar leaves no invisible canvas hitbox. Crop's temporary view and interaction are suspended, preserving its draft and viewport for return. Reference and Analyze source views keep separate fullscreen navigation and suspend area editing and artwork, preserving their drafts, docked views and the working photograph.

## Fast feedback is not authoritative data

The editor uses several image representations for different jobs:

| Representation | Purpose |
| --- | --- |
| Full developed `Scan` | Full-resolution measurement, refinement and export |
| Camera-RGB proxy | Initial/pending feedback and uncalibrated/source-inspection thumbnails |
| Proxy-derived neutral AP1 thumbnail | Positive navigation cards with current grading, look and framing |
| Filtered view-sized linear AP1 | Stable positive/source input at physical display density |
| Accepted target-linear view | Shared viewer/scope input; never graded or composed again |
| Original-sample regional result | Explicit same-position comparison or precision-tool refinement |
| Cached pending preview | Temporary display while the selected RAW prepares |
| Embedded camera preview | Filmstrip navigation placeholder and Digital brightness reference |

A pending preview shows the current grade from a compatible camera proxy while navigation, grading and geometry editing remain available. Precision consumers capture their source and edit target, then acquire the required original data asynchronously; preview pixels never become measurement or export inputs. An embedded preview is different again — already camera-rendered, with its **Camera** badge making that distinction visible. Its colour-managed luminance can seed [Digital starting brightness](processing.md#digital-starting-brightness), but RAW pixels still supply inversion, correction, measurements and exports. Completed current-recipe thumbnails supersede the camera image.

## Select and remove roll photos

Selection is temporary workspace state, separate from each source's accepted Photo settings. The active photo always belongs to the selected set and has a lighter neutral card background plus the selection outline.

- Plain click makes that photo active, retaining or replacing the selection; arrow navigation uses the same rule.
- Cmd-click toggles membership without changing the active photo; Shift-click selects the contiguous range from the last plain-click anchor; Shift–Cmd–A reduces the selection to the active photo.

**Remove selected photos…** (a selected card's context menu, or Delete/Backspace outside text editing) requires confirmation and participates in undo. It deletes each selected photo's source record and analysis support; RAW files stay on disk and project calibration and output remain. Undo restores the removed records. The next active position is the old index in the shortened roll rather than the next surviving identity.

## Responsiveness and optional caches

Optional reuse of immutable full-resolution developments, uncorrected proxies and decoded camera references shares a byte-bounded CPU store; [retention settings](#retention-settings-and-pressure) control its limit in whole MiB, including zero. [Async acquisition](architecture.md#source-acquisition-and-retention) separates consumer leases from cache ownership. A compatible source hit avoids development, while correction, grade and crop remain independent consumer state. File identity is captured at Import/Open for persisted-cache validity, [not photographic identity](projects.md#compatibility-is-about-the-processing-basis).

### Scheduling and readiness


Two [native owners](../src/source/loader.rs) at most serve selected/reference/Analyze and thumbnail development: each keeps worker-local mutable RAW handles and runs one synchronous unpack/development call at a time. Foreground work precedes visible misses, then eager work for unvisited sources; a pending foreground demand can start on a free owner, while background work starts only while no foreground demand exists. Cancelling a GPU ticket releases visible admission without discarding already-owned CPU/disk work. CPU/disk preloads and GPU-ready cards have separate completion states, allowing idle work to render an offscreen card later. [Native scheduling](native-development.md#editor-scheduling) explains the thread policy and its latency tradeoff. Thumbnail and camera-preview acquisitions suspend asynchronously; [architecture](architecture.md#source-acquisition-and-retention) describes their shared acquisition and scoped retention.



### Positive thumbnail reuse

Positive navigation cards capture the complete [photographic intent](processing.md#one-complete-image-recipe). Camera-proxy graph quality interprets their small raster, omits original-coordinate repairs, and applies grading, texture, look and output target. This favors responsive browsing over original-sample fidelity. Use the main viewer or exports to inspect Dust repairs rather than inferring their accuracy from a small navigation card.

Camera preview display and Digital preparation share bounded JPEG/RGB8 extraction and decoding, including in-flight work. ICC profiles or explicit colour labels, including the matching DNG preview IFD, establish interpretation. The document's recorded starting EV survives pixel eviction; a Digital positive waits for that value before consuming shared proxy pixels.

Cards retain actual camera arguments and calculated values. Each update authors its desired graph: matching stored calculations supply it, while changed interpretation or correction can recompute from resident camera data. Document replacement releases cards; disk-cached proxies provide reuse across opens. While updating, the previous card remains visible if source, correction, framing and target match; that presentation choice does not establish mathematical reuse. Replacements retire older outputs for that source; shared look allocations are charged once across cards.

### Retention settings and pressure

Settings controls CPU/GPU optional retention in whole MiB, including zero, independently of project/grade state; [current defaults and units](../src/editor/cache_limits.rs) describe the choices. Pressure warnings drop unprotected optional ownership and suspend offscreen work without rewriting chosen limits; selected/visible essentials may exceed the optional limits. Visible missing cards remain protected even at zero optional retention, and a retained CPU source proxy alone does not suppress camera-preview loading until the processed image is available. Cached presentation products and optional references consume part of the GPU allowance; its remaining configured share supplies calculated-image retention. Required inputs, scratch and active consumers have independent admission/lifetimes; [architecture](architecture.md) describes those boundaries.

## Build UI from shared presentation

Use the existing component families rather than copying their painting or interaction logic:

- [UI](../src/editor/ui.rs): the shared entry point; [theme](../src/editor/ui/theme.rs) owns colours and typography, [application](../src/editor/ui/application.rs) owns header/branding and panels.
- [Inspector](../src/editor/ui/inspector.rs), [parameter rows](../src/editor/ui/parameter.rs), [balance](../src/editor/ui/balance.rs): section hierarchy, row layout and value widgets.
- [Viewer framing](../src/editor/ui/viewer.rs), [tools](../src/editor/ui/tools.rs), [actions](../src/editor/ui/actions.rs), [icons](../src/editor/ui/icons.rs), [cards](../src/editor/ui/cards.rs): shared framing and interaction components.
- [Decisions](../src/editor/ui/dialogs.rs), [feedback](../src/editor/ui/feedback.rs), [working](../src/editor/ui/progress.rs): shared modal, severity and progress surfaces; [activities](../src/editor/activity.rs) own their lifetimes.

Features compose `editor::ui` components through body closures and ordinary egui layout, owning availability, accepted/draft state and dispatch. [ParameterRow](../src/editor/ui/parameter.rs) receives an explicit identity, label, bounds and value options together at its call site; shared spacing and gestures stay inside the component. [BalanceControl](../src/editor/ui/balance.rs) composes those rows with a wheel, while [white balance](../src/editor/white_balance.rs) owns its section framing and actions. Non-numeric controls use the fixed-name inspector row where they share that layout, and ordinary themed egui controls inside it.

Shared value widgets edit local values and return interaction responses; features apply those values to their own working projects. Each completed control edit is independently undoable. Inspector dropdowns fill the section width, with truncation rather than value-dependent sizing.

### Inspector density and value grammar

The inspector uses its own spacing scale, tighter than the rest of the application: [Inspector](../src/editor/ui/inspector.rs) owns the inspector row height, item gap, section gap, slider height and numeric width, and `inspector_spacing` applies them to the column. Sections separate with a separator and a short gap; a section header carries its disclosure, its changed cue, a reset and related section tools. Anything added to the inspector should take its height and gaps from these constants rather than choosing its own. Consecutive value rows use a tighter gap than sections: their tracks read as one block only when the neighbouring rows are closer than the separators around them.

A parameter is one row: a fixed-width name column, the track, then an editable value with its unit. The shared name column makes every track start at the same x, and names are right-aligned inside it so each sits against the control it labels; a unit label shortens the track's right end, and a name longer than the column truncates rather than buying track. Reset is a name gesture rather than a control — double-click or Option-click the name — backed by a **Reset to default** item in the value's context menu; a section reset still covers its whole group. Keep the row to those three parts: a per-row icon competes with the value for the narrow column.

Dragging a track is absolute, except when the press begins with Shift: that press holds the accepted value and the remaining travel applies at one-tenth rate, matching the numeric field's fine adjustment. Keyboard steps follow the same fraction.

White balance and gamma balance share one body: the two axis rows, and a label-free hue wheel at the right end of the block, with the eyedropper and section reset in the header. The axes use the same one-line row as every other parameter; the wheel gives up a little diameter so those tracks stay usable. The wheel is a correction surface rather than a measurement device, so it sizes with the panel width and drags relatively from the press origin — Shift gives ten-times finer motion and a double-click resets the balance. Precision and stepping stay with the axis rows and their numeric fields.

Scopes sit at the top of the column with their own disclosure, plot menu and drag handle. Their height is a persisted ratio of the column width and the default is deliberately modest, so the stage controls stay reachable; the handle restores that default on double-click, and the vectorscope's plot is square, so its circle follows the shorter side.

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

Start with [editor orchestration](../src/editor.rs), [Photo-stage composition](../src/editor/photo_ui.rs), [scoped control edits](../src/editor/editing.rs) and [tool ownership and replacement](../src/editor/tool.rs), then the feature owner: [viewer](../src/editor/viewer_ui.rs), [picking](../src/editor/picking.rs), [measurement](../src/editor/measurement.rs), [crop](../src/editor/crop_editor.rs), [auto-crop](../src/editor/auto_crop.rs), [analysis](../src/editor/analysis.rs), [correction](../src/editor/flat.rs), [thumbnails](../src/editor/thumbnails.rs), [pending previews](../src/editor/selected_preview.rs) or [camera placeholders](../src/editor/thumbnails/embedded.rs).

Use both component and composed-event checks from [development](development.md#shared-editor-ui-checks).