# Editor control contract

Current UI/control behavior. Remaining work is scheduled in [the roadmap](../../PLAN.md).

The editor uses `src/editor_controls.rs` for a small shared hierarchy: **section → parameter row → specialized input**. `Editor::controls` owns roll/photo grouping and reset scope; the helpers own spacing, entry, range, focus and presentation. New grades use adjustable film response in native-camera model 4; old AP0 projects reject. [Project state](project-state.md) and [output](output.md) define compatibility.

## Layout and actions

- File (New, Open, Add RAWs, Save, Save As) and Export (TIFF, EXR half/full) menus occupy the header row; document name/Modified status stays single-line. No added toolbar row. Cmd-N/O/S and Cmd-Shift-S trigger file actions after accepted numeric/control edits; New/Open/Close share Save/Discard/Cancel protection.
- Equal-hit-region **Roll → Photo → Rendering** stage tabs precede one tab-specific inspector scroll area. Roll is shared setup, Photo is per-image work, Rendering is the common preview/TIFF target; these are not independent processing pipelines. Successful accepted New selects Roll and opens missing Calibration; failed/cancelled/stale New and ordinary imports preserve tab/expansion.
- **Composition** on Photo combines turn/flip actions and crop/straighten. During cropping ordinary orientation actions stay visible/disabled, while a lightly tinted active-mode body exposes aligned Aspect/Angle/Guides rows and scoped draft reset. Roll/Rendering may hide settings without changing the draft, view or common viewer-bar Apply/Cancel. No orientation-in-draft behavior.
- Roll: **Scan orientation** → Flat-field correction → Calibration (RGB-only Dmin/Dmax) → Gamma & balance → Toe & shoulder → Exposure.
- Photo: **Composition** → White balance → Exposure. Orientation/framing is not repeated in a second Geometry section. Crop draft supports image-under-frame dragging, edge/corner resize and outside-frame straightening. Enter applies; Escape restores the initial framing/viewport. Rotated frames stay within actual source coverage. The marquee uses a thin contrasting border, corner brackets and short edge ticks, not filled object-resize handles. Thirds guides follow the crop and affect neither measurement nor export; denser alignment overlays remain deferred.
- Flip controls are momentary actions, never checked-state displays. Each flips the corresponding current displayed axis at its stage; applying it twice restores that stage. Quarter-turns still permute internal flip flags to turn the current composition correctly. Rotation/reflection order remains meaningful, but canonical flags are not exposed as a history of actions.
- Orientation is available with a loaded source even before calibration; actions/resets are disabled during picking/measuring and crop drafts. Accepted orientation edits Fit the new composition. Turn/flip controls retain the crop's selected source content. Roll turns/flips act on the shared scanning baseline before unchanged per-photo exceptions; photo actions transform what is currently displayed. Scan orientation reset and Composition reset are independent and preserve colour/calibration settings; Composition reset clears photo orientation/framing, while Reset crop clears crop/straightening and leaves exact orientation intact. Draft controls cannot change grading/output state or save/export half-applied framing.
- Rendering owns a scrollable **Target** section: Display P3/sRGB and scoped reset, labelled **Preview + TIFF · 100 nit**. It controls preview/thumbnails/scopes/TIFF, not the monitor profile or scene-linear AP0 EXR. Target changes/reset are disabled during crop and flat-field drafts. In-flight exports keep their original target/settings snapshot.
- Each scalar row has its label/value/unit/reset above a full-width slider. Calibration rows share availability, RGB display and a pick action.
- Gamma balance and WB use flattened section bodies: wheel, warmth/tint rows and neutral pick, with the section's scoped reset. There is no separate “Neutral” reset button or hidden fitted baseline.
- Gamma & balance reset changes gamma/balance only; Toe & shoulder reset restores shared toe **2.5** / shoulder **0** without changing other edits; exposure resets affect their own scope only; color reset restores the actual zero-coordinate identity. Calibration is never erased by an edit-section reset. Gamma resets to **0.65**, not 1.
- A model-4 classic-curve diagnostic state shows “Classic curve · camera model 4” and an explicit **Use adjustable response** action. Drawing controls never opts in; the action preserves calibration/gamma/balance/exposure and changes only response. A reset does not convert an original grade.
- Shoulder **0 = off**. Both linked numeric/slider parameters share actual processing/fitting state; inverse poles and unsupported GPU range/precision are rejected atomically.
- Implementation details live in tooltips. Unsaved state, approximation/refinement and actionable errors remain visible. Successful refinement must preserve a pending pick/edit error in the status line.

## Shared visual language

Recognizable actions use project-authored outline icons with semantic names/tooltips and distinct interaction/disabled states; tabs, parameter labels/values and Apply/Cancel retain text. Sections have independent disclosure/reset targets and accessible expanded state. Adjusted collapsed sections stay collapsed with a marker/summary; painting controls does not round values or alter reset scope. Roll Calibration/Gamma & balance and Photo Composition/WB/Exposure start open; other setup sections start closed.

Symbolic section expansion, scope visibility/height ratio and preferred filmstrip height persist in version-1 application preferences, not project state. On macOS the file is `~/Library/Application Support/Antitone/ui-preferences.json`; other platforms use XDG config or `~/.config/antitone`. Load once; save changed preferences atomically after event-loop exit. Malformed/unreadable/unsupported files remain untouched with nonfatal diagnostics. Window constraints do not overwrite preferred sizing. Tabs, source EV, view, plots/magnification, drafts and resources remain transient.

## Flat-field workflow

Choose reference RAW opens an uncorrected reference with a reference-specific automatic rectangular proposal, or manual selection if detection is ambiguous. Inspect/move/resize an integer rectangle on canvas in untransformed original-source coordinates using the shared mapping; photo crop/straightening and roll/photo orientation are temporarily bypassed for this reference-support edit only. Apply correction accepts it and returns to the working photo; Cancel restores accepted composition and the exact original view/state. Estimation excludes borders before smoothing and rejects clipped/unusable support. The reference must be film-free and from the same scanning setup; metadata compatibility alone does not prove matching lighting/aperture/focus.

The Roll section shows filename, Enabled, Edit area and Clear before Calibration. New reference selection never adds a filmstrip photo or hides an existing one. Failed/stale/cancelled operations retain accepted map, grade and working context. Enabled/Clear preserve viewport and inspection mode. Dmin/Dmax are never resampled; the setup/calibration warning recommends re-picking after correction changes. Enabled/map/support are saved; reference-edit context and inspection EV remain transient. Active area identity/progress lives in the existing Roll section; Apply/Cancel use the shared viewer-bar group, not an extra panel.

## Numeric and keyboard behavior

| Parameter | Normal slider | Numeric/drag range | Default | Slider step / Shift step |
| --- | --- | --- | --- | --- |
| Gamma | 0.1…3 | 0.1…3 | 0.65 | 0.01 / 0.001 |
| Toe | 1…4 | 1…4 | 2.5 | 0.01 / 0.001 |
| Shoulder | 0…0.02 | 0…0.02 | 0 (off) | 0.0001 / 0.00001 |
| Exposure | −3…3 EV | −16…16 EV | 0 EV | 0.05 / 0.005 EV |
| Warmth / tint | −1…1 | −1…1 | 0 | 0.01 / 0.001 |

Numeric drag speed is 0.01 (shoulder: 0.0001), with egui's Shift-drag fine adjustment. Click or Tab to enter text; Enter/focus loss commits and Escape cancels. Model-range limits constrain numeric edits. Existing legal values are not rounded or clamped merely by drawing a row, including exposure outside the normal slider span. Moving the exposure track intentionally returns to its normal span. Display precision is separate from the stored f64 value.

Stable IDs separate roll, photo, sections and individual parameters. Disabled rows render a read-only value snapshot: egui 0.36.2's `DragValue` can otherwise commit pending text on focus loss **even while disabled**. Busy/overwrite transitions therefore cannot unexpectedly apply that text. Existing complete GPU-parameter validation still rejects unsupported combined proposals and restores the previous edits; there is no source-view or CPU fallback.

## Viewer navigation and source inspection

- Unmodified scroll pans both axes; Ctrl/Cmd-scroll or pinch zooms around the pointer. Multitouch uses its own center/translation and does not also apply synthetic wheel/zoom events.
- One fixed 40-point viewer-width bar keeps canvas dimensions stable across tool transitions. Fit/physical-pixel 100%/zoom, source view, source-only inspection and help lead; target/Approximate/Refining/Cached preview state trails outside tools. No permanent ready row or tool filename/coordinate panel.
- Click a fitted image for physical-pixel 100% around that point; click an explicitly zoomed image to Fit. Zoomed primary drag pans; middle/Space-primary drag also pans during picking. `z` Fits unless text input has focus.
- Picker primary click/drag retains patch/rectangle priority; zoom, grabbing, crosshair, move and corner-resize cursors follow the active interaction. Picks remain in original developed-source coordinates through shared composition mapping; resize cursors follow canvas direction after orientation.
- Starting a picker retains zoom/pan and exact orientation. Gamma/WB keep the displayed grade/balance; Dmin/Dmax enter developed-source inspection, with working-photo viewport restoration after temporary roll-source selection. Calibration sampling uses only ordered filmstrip sources; no external/unregistered chooser. Dmin/Dmax temporarily reveal uncropped oriented source, preserving physical magnification and the source-coordinate canvas-centre anchor. Unchanged exit restores Fit; Gamma/WB retain deliberate in-picker navigation. [Coordinate/view requirements](geometry.md#panels-and-picker-views).
- Crop entry Fits the full active source with a handle margin, not the accepted crop. Resizing and release retain source magnification/placement; angle changes retain magnification and the screen pivot. Explicit 100%, +/− and **Fit source** remain available. Only Apply Fits the finished composition; Cancel restores the pre-edit viewport. The provisional editing view does not request settled reconstruction, including when explicit zoom places the frame offscreen.
- Every image tool uses one trailing tinted viewer-bar group: recognizable icon/name, Cancel then primary Apply. The name→Cancel gap is 12 points; buttons remain closely grouped. Busy work replaces the icon with a spinner and disables unsupported actions; Apply remains present/disabled without a proposal. Target/quality and the unavailable source toggle yield to the group. At narrow widths auxiliary zoom ± and inline EV yield first; Fit/100%, inspection and help remain. Only selection/frame artwork stays on canvas; no floating box, generic Details or placement chevron.
- Enter/Escape and existing Apply/Cancel transaction boundaries remain; numeric entry/popovers own keys first. Bar/inspector/popover gestures cannot also manipulate the canvas. Tab changes/outside clicks never implicitly accept/discard a draft.
- Source-only **Inspection** opens a zero-default exposure adjustment with the standard range/reset. Hidden inspection retains EV. It changes only source-preview brightness before unchanged ACES; it is not saved and does not affect development, calibration, fitting, edits or exports. Positive preview ignores it.

Normal registered-photo switches can show **Cached preview · developing full-resolution RAW…** from a compatible retained 256 RGB16 proxy. It follows the selected photo's current grade, target and accepted composition, with separate source-inspection brightness. It is display-only: picks, crop edits, refinement, scopes and exports stay unavailable until full preparation completes. Memory misses/stale keys keep the loading placeholder; failure/replacement removes cached display. Temporary picker/reference loading does not adopt it. Preview admission does not change saved state.

## Filmstrip thumbnails

The existing bottom strip shows approximate thumbnails of each accepted composition and filenames. Source/processed caches stay geometry-independent; crop/orientation/aspect are applied on paint. Selection is unavailable during a crop draft or flat-field support edit; a crop draft also suspends new thumbnail scheduling. Drag its top edge to change height (default 140 points, 52…240 points and window-height limited); images follow available height with preserved aspect. Resizing, scrolling and caches are transient and do not dirty/save the project. Horizontally offscreen cells are virtualized; cache capacity does not dictate cell width. Visible entries are protected from eviction and missing visible cells do not wait for settled refinement or grading idle after foreground preparation. Worker completion requests immediate repaint. Recent entries support scrolling back, and normal-pressure idle prefetch for nearby and then unshown photos is lower priority and uses only spare retention budget. Optional retention scales with machine capacity and memory pressure; [cache contract](output.md#responsiveness-and-optional-caches).

Without calibration thumbnails show developed source at zero inspection gain; otherwise they show actual shared roll and each photo's grade through the existing GPU/ACES/managed presentation path. The viewer's source bypass/inspection EV does not change thumbnails. Enabled flat-field gains apply in original source coordinates; changing/clearing the map invalidates GPU image identity but reuses uncorrected RGB16 proxy cache entries. Normal selection and ordered-roll Dmin/Dmax sampling preserve working-photo return state. Loading placeholders and error markers/tooltips retain filenames and selection; thumbnails never supply measurements or export pixels. Verified selected development seeds a thumbnail instead of being developed again. Grade/view edits reuse source work, obsolete scroll jobs cancel between phases, and a disposable persistent RGB16 cache avoids re-development on later visits/restarts. Critical memory pressure suspends new thumbnail work without changing the grade or hiding existing visible images.

## Image scopes

A separate shaded inspection region above Roll/Photo/Rendering switches waveform/vectorscope without changing viewer dimensions. Height follows sidebar width and a draggable bottom divider adjusts the scope/controls tradeoff; double-click resets sizing. Vectorscope is the initial plot, with labelled 1×/2×/4×/8× magnification and reset; intensity compensates for smaller chroma bins when magnified. Waveform has RGB/luma modes. RGB/CMY direction ticks/letters remain on the vector rim when zoomed, distinct from magnified 100% reference boxes. A dashed Skin hue guide is explicitly approximate, not a correctness test. Showing scopes reduces inspector scroll space while retaining stage tabs; visibility and size are application preferences, and Target stays in Rendering. Both plots use the positive grade in the selected output's native encoded basis, labelled Whole frame / Approximate; source/reference/busy views and crop drafts suspend them. Accepted crop/orientation follows whole final-frame order, not viewer or source-bounding-box coverage. No project dirty/schema/export change. [Measurement, coverage and lifecycle contract](scopes.md).

## Resolve-style wheel orientation and mapping

Both wheels use **blue at 3 o'clock, yellow at 9 o'clock**, with RGB/CMY proceeding clockwise. The displayed six sectors are equally spaced: R at 11, M at 1, B at 3, C at 5, G at 7, Y at 9. Artwork, labels, pointer input and marker positioning share the same orientation transform. This is a UI direction convention, not a claim about exact Resolve processing or the vectorscope's target-native luma/chroma measurement basis.

In a right/up plane, the existing opponent coordinates are projected with:

```text
P(rgb) = ((R-B)/sqrt(2), (R-2G+B)/sqrt(6))
O = [ -sqrt(3)/2   1/2       ]
    [  1/2        sqrt(3)/2  ]
```

- **Gamma balance:** display matrix `O`. Coordinates control relative log gamma in native-camera RGB, not RGB gains. Pixel response depends on the negative/calibration.
- **White balance:** display matrix `O * J`, where `J` is the exact differential AP1 neutral-white response to the existing Bradford log-cone-gain coordinates. With `C = Bradford * AP1_to_XYZ`, its columns are `P(C^-1 * diag(C*white) * opponent(1,0))` and the corresponding tint column. This is a local display adapter, not a replacement WB operator.

Each matrix is normalized by the maximum mapped corner length of the legal warmth/tint square. Input uses its inverse; coordinates outside the legal square are clipped to the existing ±1 limits. Wheel, sliders, numeric entry and fitted neutral corrections all share the same `Balance` state. There is no new saturation/strength state, physical Kelvin interpretation, or processing/profile change.

## Verification

Headless event/layout checks cover numeric precision and range, keyboard/mouse entry, Enter/Escape, pending-edit disable transitions, wheel mapping and disabled behavior, true scoped defaults/resets, section order and control reachability at normal/narrow widths. Wheel adapters are checked independently for full-square round-trips, local WB response and blue/yellow orientation.

[Specialist verification](../build-and-probes.md#editor-and-layout-smoke) provides real-image layout commands with diagnostic settings. Headless events and scripted visible frames do not establish physical trackpad feel, photographic correctness or device/monitor acceptance.
