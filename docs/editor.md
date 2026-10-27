# Editor state and interaction

The editor connects photographic settings, image tools and asynchronous work. Its central rule is simple: **a view may be temporary, but accepted photographic state must remain coherent**.

Roll, Photo and Rendering are editing scopes, not separate pipelines. Roll selects Negative or Digital interpretation and owns shared grading plus Negative's scan setup; Photo owns individual grading and composition — its turn/flip setting starts from the orientation the camera recorded, and saved projects keep their stored setting; Rendering owns the project-wide creative look and preview target, which also initializes rendered export. Changing tabs changes available controls, not the image's processing. [Architecture](architecture.md) describes how these scopes are owned and disposed.

The editor uses a neutral dark workspace with amber identity and active-state accents. The shared application header presents the film-edge wordmark and project identity without making the wordmark an interactive control. [Branding assets](../assets/branding/README.md) explain the artwork and regeneration.

## Application menus and settings

On macOS, native AppKit menus provide the conventional Antitone, File, Edit, View, Window and Help groups. File → **Add Scans…** adds camera RAW source references without moving or modifying originals; Export belongs under File. Settings, About and local workflow help open focused in-editor panels. Settings controls optional CPU/GPU retention in whole MiB, including zero and explicit reset — see [Responsiveness and optional caches](#responsiveness-and-optional-caches); those limits are not a process-memory cap.

The [shared menu model](../src/platform/menu.rs) supplies actions, grouping and shortcuts to native and egui presentation; the [host](../src/platform/host.rs) owns native menu lifetime and forwards activations through winit's event-loop proxy, restoring a minimized editor so panels do not open invisibly. The editor owns availability and revalidates queued activations: a disabled command cannot bypass an active tool, decision or foreground operation. Text-targeting Edit actions and focus-changing commands wait for pending text input to render, so a Paste followed immediately by Save cannot lose the pasted value. The egui fallback uses the same commands and state when native menus are not installed; only the macOS host is implemented.

### Native quit negotiation

Dock and system Quit enter the same unsaved-change guard as menu Quit: the editor reports live close continuations, and busy work or active drafts decline the request without accepting or discarding anything. The [termination bridge](../src/platform/termination.rs) uses AppKit's deferred decision — winit owns its delegate, so the bridge adds only the missing optional decision method — and coalesces repeated termination requests while a decision is pending.

## Accepted state and drafts

Crop, picker and reference-area edits have temporary interaction state. Apply promotes a validated proposal; Cancel restores the working context. Tabs and outside clicks do not implicitly accept or discard those drafts. Local gradients instead preview a captured gesture and accept a validated release. Save/Export never consume half-applied geometry.

Tools deliberately use different views to reach what they measure: Dmin/Dmax expose uncropped oriented source so film margins are reachable; a bare-light reference exposes untransformed source; gamma/WB picks retain the working photograph's composition while their fitting uses declared measurement inputs. Applying a calibration or neutral pick captures the source lease and numerical inputs at admission, so later state cannot retarget the measurement. [Architecture](architecture.md) explains the execution and ownership model.

Fixed tool shelves sit below the stage tabs. Roll contains shared orientation, Analyze and Auto-crop; Photo contains turn/flip, Crop and Dust, with a persistent Layers inspector section. Active controls appear above ordinary inspector sections; clicking an active Crop/Auto-crop cancels its draft, and opening Crop/Auto-crop/Dust cancels gradient interaction without changing layer selection or accepted grades. Tools arbitrate canvas ownership rather than running competing gestures. [Analyze's roll review](roll-analysis.md#1-choose-what-to-sample) is its own review transaction over per-photo sampling drafts; browsing and exception edits never change accepted support or grading.

### Auto-crop is an explicit accepted edit

Photo's **Auto-crop** button proposes the current photo's crop and straightening into the manual Crop draft, then permits ordinary refinement. Roll's **Auto-crop roll** and the filmstrip context menu propose crops for a whole set, independently of browsing or selection.

[Shared preparation](spatial-metadata.md) can infer nominal picture bounds from credible edges and common dimensions. Missing film alone does not veto that frame; unresolved geometry or conflicting picture evidence abstains and keeps previous crops. Photos that merely fall outside the captured selection are not failures; abstentions carry a warning cue with their specific reason. [Geometry](geometry.md#automatic-framing-consumes-measurements-not-sampling-support) explains the evidence boundary and inward margin.

Proposals remain drafts: margin edits do not modify accepted crops or dirty the project, and Apply installs all proposed values atomically after revalidation — a stale target rejects the batch instead of silently shrinking its scope. Only applied crops persist; the margin is reused within this editor instance, not saved as a project setting.

### Provisional dust cleanup

Dust is direct canvas work, not a per-repair Apply/Cancel transaction. An unmodified click or stroke on free image content creates a source-pixel mask; release starts its donor search. Clicking an existing support or outline selects it; Shift adds, Option removes, Delete deletes that repair rather than removing the photo. Dragging a donor adjusts its stored offset; ordinary wheel input resizes the brush while Cmd/Ctrl-wheel zooms.

Mask admission is based on marked pixels and a bounded processing area, not a fixed side length. If a held stroke exceeds the budget, the draft and brush turn red and release discards the whole rejected stroke, preserving existing repairs rather than committing a truncated prefix.

Repairs evaluate asynchronously from immutable, full-resolution **ungraded AP1**: correction and camera conversion are upstream; adjustment layers are downstream. Each repair stores one donor offset shared by its components. Healing is deterministic from that offset, and reopening never searches. Source, input interpretation or correction changes invalidate evaluated values; downstream grading and navigation reuse them.

A released new or changed mask stays pending while a short, best-effort search finds one donor or rejects an impossible placement. Only the completed choice becomes an accepted repair and undo action. Its donor is then adjustable even if healing fails. Search retains its captured source across navigation; ordinary browsing and grading remain available. Save, Export, document replacement and Undo/Redo are unavailable until pending strokes resolve. Both operations follow [the architecture's](architecture.md) workflow and cancellation model.

Accepted repair instructions participate in ordinary [project saving and dirty protection](projects.md#source-owned-dust-repairs); TIFF/EXR export consumes the accepted set, including before the photo has been viewed.

## Look selection prepares before acceptance

Rendering owns one project-wide look. Its dropdown offers None, the built-in paper/print looks and **Load LUT…**; the load entry and file dialog declare the scene-referred contract, and [colour and output](colour-and-output.md#creative-looks-precede-output-rendering) owns the restrictions.

Built-in names come from the versioned catalog; imported and unknown historic names remain as saved, and [project storage](projects.md#embed-the-accepted-look-not-a-catalog-reference) preserves their accepted appearance. Successful imports also appear under **Loaded LUTs** for the rest of the editor session, including across project changes; repeated identical payloads share one entry.

Preparation and reselection use a document-owned workflow: resolution, import and device preflight run off-thread, and a failure is acknowledged without changing the accepted look. [Architecture](architecture.md) owns that lifecycle.

## Undo and redo

The Edit menu and Cmd–Z / Shift–Cmd–Z traverse one chronological history for the whole project. Roll, Photo and Rendering edits share it, including calibration, correction, accepted Analyze results and project-only photo removal. Navigation itself is not recorded; undoing a Photo edit activates its photo.

A continuous gesture is one action; numeric text commits, resets and commands are separate actions. Cancelled, rejected and unchanged edits add no entries; new accepted edits discard redo. The implementation retains up to 100 actions in memory as cloned accepted project descriptions — not RAW pixels or rendered images — and history is neither stored in the project file nor restored on relaunch.

## Numeric edits own their input

A displayed number is not necessarily a rounded stored value. Controls preserve precise values while the user reads or redraws them; only a deliberate edit may change processing. Pending text commits before a stage switch, Analyze snapshot or file action, and discrete commands wait for that admission so they remain separately undoable. Escape cancels text editing before it can cancel an image tool.

Colour wheels drag relative to the accepted value; holding Shift gives ten-times finer motion without a jump when Shift changes. Popovers, numeric controls and toolbar gestures own input ahead of the canvas, so one action cannot also pan, pick or apply a draft.

## Decisions are not operation failures

A decision asks permission before changing or replacing something; a failure explains why an attempted operation did not complete. Save/Discard, project and output replacement use the shared [decision dialog](../src/editor/presentation/dialogs.rs) family, so a required choice is not presented as a warning and none of these confirmations is an operation failure. Analyze folds its Photo-reset consequence into the sampling-area review instead of opening another dialog.

[Feedback presentation](../src/editor/presentation/feedback.rs) distinguishes information, warning and error with a named severity, glyph and restrained colour — never colour alone. Acknowledged messages state what did not happen and what remains intact, with full diagnostic details behind a subdued disclosure. [Operation controllers](../src/editor/feedback.rs) retain the typed failure separately from status text: acknowledgement or a matching success retires it, while merely starting another job or receiving stale results does not. Other import/analysis failures remain passive status rather than acknowledged messages.

## Working feedback follows what remains usable

Working feedback answers two questions: **what is happening, and what can I still do?** It is neutral activity, not a warning or a request for approval; its prominence comes from the operation's actual input restrictions.

| Work | Presentation and retained use |
| --- | --- |
| RAW import, project Open/New/Save, correction/context switches, undo/redo | A centred working modal after a brief delay; the transaction already blocks the workspace |
| Export | A delayed working modal with finished-photo counts and Cancel; workspace, inspection and application panels stay blocked until completion or cancellation acknowledgement |
| Selected RAW preparation | A spinner on the empty canvas; cached pending previews stay visible but never become the processing source |
| Reference preparation, picker measurement | Local tool/viewer feedback; no extra global dialog |
| Shared region preparation and running Analyze | A delayed working modal with counted work, then indeterminate validation and genuine Cancel; the intervening sampling-area review remains interactive |
| Thumbnails, camera previews, scopes | Existing in-place image/card/scope feedback |

Save/Save As remain available through application commands during shared region preparation; they capture ready results without waiting for preparation. A deferred consumer resumes after the write, or after acknowledgement of a failed write.

[Task activity](../src/editor/activity.rs) is the foreground integration point: begin a typed task on the controller's request ID, report counted or indeterminate progress, and finish the matching request on success, failure or send-failure. Request keys are scoped by operation, so a photo-load result cannot finish an export that happens to share a number; stale reports are ignored. Cancellation has two responsibilities — signal the real worker, then mark the task `cancelling`, which drops measured progress and Cancel until matching completion. [Working presentation](../src/editor/presentation/progress.rs) renders the task's modal or local surface.

## Export photographs

File → **Export…** writes Selected photos or All photos in filmstrip order; non-filmstrip source/reference records are excluded. Choose an existing destination folder, TIFF or EXR, and format-specific output settings. TIFF offers sRGB/Display P3 with 16-bit RGB and lossless Deflate; EXR has fixed scene-linear AP0 and half/full float precision. [Colour and output](colour-and-output.md#tiff-and-exr-serve-different-purposes) explains the different rendering semantics.

Filenames use the source stem with an optional suffix. Keep both is the non-destructive default, Skip leaves existing files alone, and Replace requires explicit batch confirmation. The last accepted folder is an application preference. Export choices and target overrides do not edit the project or dirty state.

The batch captures accepted crops and adjustments at full resolution, then processes one photo at a time without visiting it. Cancel stops subsequent work and may stop the current photo at existing safe checkpoints; an in-flight native/encoder call can finish before acknowledgement. The modal retains cancelling feedback, and Escape/outside clicks do not dismiss running work. Completion summarizes exported, skipped, failed and remaining photos, with diagnostics and Show in Finder for completed output. Individual failures leave other photos exportable; published files are never rolled back as a batch.

## Source inspection is a view

Developed-source inspection uses the same automatically lens-corrected development as the positive, bypassing inversion and photographic grading. Negative's **Viewing WB** defaults to Daylight, with As shot available for comparison; Digital already develops as-shot. The exposure control adjusts brightness relative to the [source-display baseline](colour-and-output.md#source-inspection-has-its-own-exposure-convention). Both controls apply to the canvas and source-preview thumbnails; calibrated filmstrip cards retain their positive grade. These viewing choices are transient: calibration, fitting, grades and exports still use the underlying developed pixels.

Source and positive caches have distinct identities: switching modes reacquires the appropriate source rather than grading an inverted positive as source light. Missing Negative calibration forces source-only presentation; completing a usable Dmin/Dmax pair returns to the positive. Digital is ready without anchors.

## Immutable adjustment-layer routing

Adjustment layers are recipe values, not rendered pixels: selected and cached previews, thumbnails and scopes share complete processing snapshots, and each image owns only its mutable output/preparation state. Calibration/correction and camera interpretation identify the input source; Roll WB and ordered layer values identify photographic grading. The composed processing identity includes both, excluding names, IDs, view selection and source-inspection WB/EV. [Processing](processing.md#adjustment-layers-change-controls-not-rendered-pixels) owns what that means for the recipe.

The Layers section creates global, linear and radial layers above the selection and displays processing order bottom to top. One set of photographic controls targets the selected layer, named in the Layers header even when the list is collapsed. Strength, gradient inversion and radial Feather appear below the list. Empty stacks and ineffective controls remain visible but greyed out; the initial global layer has the same selection, reordering and deletion behavior as any other layer.

The creation icons arm a one-shot gesture: a valid released stroke creates and selects a new adjustment with grading immediately available and no Apply step. Drags outside the selected shape's handles navigate instead of redrawing it; Escape cancels only the current gesture. Rows drag to reorder with an insertion line, double-click renames inline, and Delete/Backspace on the Photo stage removes the selected layer through a stale-safe confirmation. A rejected creation stays armed for retry; a rejected refinement returns to accepted geometry. Mask artwork and handle interaction hide when Layers is collapsed or while adjusting values, preserving the coverage preference. Active gradient gestures keep their guides until completion.

## A stable working view while grading

`WorkingView` owns normal-view demand, revision and accepted presentation for positive and source viewing. The positive view acquires a physical-view-sized signed linear AP1 source from an explicitly captured immutable developed-source lease; correction and calibrated inversion still process original camera pixels before footprint filtering. At a fixed view, exposure, WB, contrast and saturation reuse that same source. [Processing](processing.md#physical-view-processing-is-an-approximation) and [geometry](geometry.md#process-before-averaging) distinguish arithmetic agreement from sampling quality.

Grading is captured at frame boundaries rather than completed by a worker that could repeatedly lose a race with continuous controls: one submitted colour result stays immutable until completion, and subsequent input coalesces to the latest settings. Navigation replaces the source asynchronously through a photo-owned workflow, and replacements always come from originals — never recursive resampling of a preview. A coarse whole-composition neutral underlay fills newly exposed regions while regional support is missing. [Architecture](architecture.md) owns that workflow and admission model.

All canvas images are presented as `RenderedView` products. Repaired crop and precision previews use the same bounded original-coordinate acquisition as the working view; a cold repaired preview shows updating feedback until suitable support is ready. Crop editing maps the whole native-axis source beneath its affine draft, retaining content outside the frame.

The **Original-resolution reference** toggle requests original-sample GPU grading reconstructed at the physical view. Precision tools also present their completed original-sample images through the shared renderer, with native rectangular or affine framing as requested. These are purpose-bound products: leaving a tool or changing its inputs rejects stale results. Comparison never silently sharpens normal viewing; completed references enter optional retention after comparison ends.

## Photo-only fullscreen inspection

The fullscreen action beside the zoom controls, or F, makes the current photograph a full-display inspection view: the host makes the existing window borderless without entering a native fullscreen Space. Shared zoom/source controls float over the image and fade after inactivity; moving near the bottom or pressing Tab reveals them, and a hidden bar leaves no invisible canvas hitbox. Analyze review keeps its sampling controls and does not enter this mode.

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

A pending preview can show the current grade from a retained compatible camera proxy, but it is not a ready `Scan`: picks, crop editing, refinement, scopes and export wait for full preparation. An embedded preview is different again — already camera-rendered, with its **Camera** badge making that distinction visible. Its colour-managed luminance can seed [Digital starting brightness](processing.md#digital-starting-brightness), but RAW pixels still supply inversion, correction, measurements and exports. Completed current-recipe thumbnails supersede the camera image.

## Select and remove roll photos

Selection is temporary workspace state, separate from each source's accepted Photo settings. The active photo always belongs to the selected set and has a lighter neutral card background plus the selection outline.

- Plain click makes that photo active, retaining or replacing the selection; arrow navigation uses the same rule.
- Cmd-click toggles membership without changing the active photo; Shift-click selects the contiguous range from the last plain-click anchor; Shift–Cmd–A reduces the selection to the active photo.

**Remove selected photos…** (a selected card's context menu, or Delete/Backspace outside text editing) requires confirmation and participates in undo. It deletes each selected photo's source record and analysis support; RAW files stay on disk and project calibration and output remain. An accepted flat-field reference survives as a reference-only record until correction is cleared. Undo restores the removed records. The next active position is the old index in the shortened roll rather than the next surviving identity.

## Responsiveness and optional caches

Optional reuse of immutable full-resolution developments, uncorrected proxies and decoded camera references shares a byte-bounded CPU store; [retention settings](#retention-settings-and-pressure) control its limit in whole MiB, including zero. [Async acquisition](architecture.md#source-acquisition-and-retention) separates consumer leases from cache ownership. A compatible source hit avoids development, while correction, grade and crop remain independent consumer state. File identity is captured at Import/Open for persisted-cache validity, [not photographic identity](projects.md#compatibility-is-about-the-processing-basis).

### Scheduling and readiness


Two [native owners](../src/source/loader.rs) at most serve selected/reference/Analyze and thumbnail development: each keeps worker-local mutable RAW handles and runs one synchronous unpack/development call at a time. Foreground work precedes visible misses, then eager work for unvisited sources; a pending foreground demand can start on a free owner, while background work starts only while no foreground demand exists. Cancelling a GPU ticket releases visible admission without discarding already-owned CPU/disk work. CPU/disk preloads and GPU-ready cards have separate completion states, allowing idle work to render an offscreen card later. [Native scheduling](native-development.md#editor-scheduling) explains the thread policy and its latency tradeoff. The deferred [source-loading plan](../SOURCE_LOADING_PLAN.md) covers async thumbnail and embedded-preview coordination.



### Positive thumbnail reuse

Positive navigation cards interpret a small camera-RGB proxy, then apply ordered layer grading, look and output target. They favor responsive browsing and recognizable photographs over original-sample fidelity. Use the main viewer or exports to inspect Dust repairs rather than inferring their accuracy from a small navigation card.

Camera preview display and Digital preparation share bounded JPEG/RGB8 extraction and decoding, including in-flight work. ICC profiles or explicit colour labels, including the matching DNG preview IFD, establish interpretation. The document's recorded starting EV survives pixel eviction; a Digital positive waits for that value before consuming shared proxy pixels.

Compatible neutral products survive grade, look and target changes. Across project replacement, Digital positives also require a known matching normalization, even when captured pixels and photographic controls match. While an update prepares, the previous card stays visible with an updating indicator if source, correction, framing and target match. Replacements retire older outputs for that source; shared look allocations are charged once across cards.

### Retention settings and pressure

Settings controls CPU/GPU optional retention in whole MiB, including zero, independently of project/grade state; [current defaults and units](../src/editor/cache_limits.rs) describe the choices. Pressure warnings drop unprotected optional ownership and suspend offscreen work without rewriting chosen limits; selected/visible essentials may exceed the optional limits. Visible missing cards remain protected even at zero optional retention, and a retained CPU source proxy alone does not suppress camera-preview loading until the processed image is available. GPU infrastructure is shared, but mutable image storage, revision and processing signatures belong to each image/job; [architecture](architecture.md) describes those worker boundaries.

## Build UI from shared presentation

Use the existing component families rather than copying their painting or interaction logic:

- [Presentation](../src/editor/presentation.rs): header/branding, fallback menus and panels, labelled rows, stage tool shelves.
- [Viewer framing](../src/editor/presentation/viewer.rs) / [Hierarchy](../src/editor/presentation/hierarchy.rs): docked controls, sections, disclosure/reset, stage tabs, selectable rows.
- [Decisions](../src/editor/presentation/dialogs.rs), [Feedback](../src/editor/presentation/feedback.rs), [Tasks](../src/editor/activity.rs), [Working](../src/editor/presentation/progress.rs): the shared modal, severity, task and progress surfaces.
- [Cards](../src/editor/presentation/cards.rs), [Icons](../src/editor/icons.rs), [Controls](../src/editor/controls.rs): filmstrip geometry, semantic actions, photographic value widgets.

Presentation receives display-ready state and returns responses; controllers still own availability, IDs, accepted/draft state and dispatch. Inspector dropdowns fill the section width, with truncation rather than value-dependent sizing.

### Inspector density and value grammar

The inspector uses its own spacing scale, tighter than the rest of the application: [Presentation](../src/editor/presentation.rs) owns the inspector row height, item gap, section gap, slider height and numeric width, and `inspector_spacing` applies them to the column. Sections separate with a separator and a short gap; a section header carries its disclosure, its changed cue, a reset, and optionally one section tool. Anything added to the inspector should take its height and gaps from these constants rather than choosing its own. Consecutive value rows use a tighter gap than sections: their tracks read as one block only when the neighbouring rows are closer than the separators around them.

A parameter is one row: a fixed-width name column, the track, then an editable value with its unit. The shared name column makes every track start at the same x, and names are right-aligned inside it so each sits against the control it labels; a unit label shortens the track's right end, and a name longer than the column truncates rather than buying track. Reset is a name gesture rather than a control — double-click or Option-click the name — backed by a **Reset to default** item in the value's context menu; a section reset still covers its whole group. Keep the row to those three parts: a per-row icon competes with the value for the narrow column.

Dragging a track is absolute, except when the press begins with Shift: that press holds the accepted value and the remaining travel applies at one-tenth rate, matching the numeric field's fine adjustment. Keyboard steps follow the same fraction.

White balance and gamma balance share one body: the two axis rows, and a label-free hue wheel at the right end of the block, with the eyedropper and section reset in the header. The axes use the same one-line row as every other parameter; the wheel gives up a little diameter so those tracks stay usable. The wheel is a correction surface rather than a measurement device, so it sizes with the panel width and drags relatively from the press origin — Shift gives ten-times finer motion and a double-click resets the balance. Precision and stepping stay with the axis rows and their numeric fields.

Scopes sit at the top of the column with their own disclosure, plot menu and drag handle. Their height is a persisted ratio of the column width and the default is deliberately modest, so the stage controls stay reachable; the handle restores that default on double-click, and the vectorscope's plot is square, so its circle follows the shorter side.

The inspector reserves a band for the scroll bar because dense rows put values and header actions near the right edge, where a floating bar would take their pointer input.

Feature controllers use ordinary `&mut Editor`, raw `egui::Ui` and a separate borrowed [UiContext](../src/application.rs); `spawn` supplies combined state/execution context to its factory only when the host polls outside painting. [Architecture](architecture.md) owns that execution model.

### Contextual status indicators

Use the named [contextual status pattern](../src/editor/presentation/feedback.rs) for a local condition, not a feature-specific badge or global notification. `ContextualStatus` carries display-ready reason and level; cards and tool warning identities reuse the same primitive, and controllers own the condition and its retirement.

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

Reading text should be quiet; short instrument markings give the workspace its camera-body character. The editor embeds [IBM Plex Sans](../assets/fonts/README.md): Text for ordinary controls, explanations, filenames and numeric entry; Medium for stage tabs, active tool names and compact readouts. [Presentation](../src/editor/presentation.rs) installs the fonts and owns `instrument_label`, `instrument_readout` and `TextAction::readout`; use those shared roles rather than restyling per feature. The Arimo wordmark remains separate artwork; stage tabs share equal breathing room around their labels rather than equal widths.

### Colour roles

Colour expresses interaction state, not feature ownership. Presentation installs the editor theme once; ordinary egui widgets and shared components inherit its selection, active-state and link colours. Custom interaction painting reads the applicable role from `ui.visuals()` or `ui.style().interact(&response)` rather than copying brand RGB values. Contact-sheet fills and image wells stay deliberately neutral; photographic wheels and scope plots keep colours that represent image data.

## Working on this area

Start with [editor orchestration](../src/editor.rs), then the feature owner: [viewer](../src/editor/viewer_ui.rs), [crop](../src/editor/crop_editor.rs), [auto-crop](../src/editor/auto_crop.rs), [analysis](../src/editor/analysis.rs), [correction](../src/editor/flat.rs), [thumbnails](../src/editor/thumbnails.rs), [pending previews](../src/editor/selected_preview.rs) or [camera placeholders](../src/editor/thumbnails/embedded.rs).

Use both component and composed-event checks from [development](development.md#shared-editor-ui-checks).