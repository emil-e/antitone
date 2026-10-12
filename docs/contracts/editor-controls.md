# Editor control contract

Current UI/control behavior. Remaining work is scheduled in [the roadmap](../../PLAN.md), not in the verification snapshot below.

The editor uses `src/editor_controls.rs` for a small shared hierarchy: **section → parameter row → specialized input**. `Editor::controls` owns roll/photo grouping and reset scope; the helpers own spacing, entry, range, focus and presentation. New grades use adjustable film response in native-camera model 4; old AP0 projects reject. Project schema/development identity are versioned accordingly; the output contract is unchanged.

## Layout and actions

- File (New, Open, Add RAWs, Save, Save As) and Export (TIFF, EXR half/full) menus replace action buttons within the existing header row; document name/Modified status stays single-line. No added toolbar row. Cmd-N/O/S and Cmd-Shift-S trigger file actions after accepted numeric/control edits; New/Open/Close share Save/Discard/Cancel protection.
- Roll/Photo tabs precede the controls. Only the selected scope's top panel is shown, pinned outside the edit scroll area: **Scan orientation** on Roll, **Composition** on Photo. They never appear together.
- **Composition** combines photo orientation and crop/straighten. It offers Turn left/right, Flip horizontal/vertical, Crop and crop-only reset. Crop editing replaces the action rows with Aspect/preset/swap, Angle numeric/slider/reset, optional thirds, dimensions and Reset / Cancel / Apply. Switching to Roll hides these controls without changing/cancelling the draft or viewport; return to Photo to resume, or use Enter/Escape.
- Roll: **Scan orientation** (shared Turn left/right and Flip horizontal/vertical) → Flat-field correction → Calibration (RGB-only Dmin/Dmax) → Gamma & balance → Toe & shoulder → Exposure.
- Photo: **Composition** → White balance → Exposure. Orientation/framing is not repeated in a second Geometry section. Crop draft supports image-under-frame dragging, edge/corner resize and outside-frame straightening. Enter applies; Escape restores the initial framing/viewport. Rotated frames stay within actual source coverage. The marquee uses a thin contrasting border, corner brackets and short edge ticks, not filled object-resize handles. Thirds guides follow the crop and affect neither measurement nor export; denser alignment overlays remain deferred.
- Flip controls are momentary actions, never checked-state displays. Each flips the corresponding current displayed axis at its stage; applying it twice restores that stage. Quarter-turns still permute internal flip flags to turn the current composition correctly. Rotation/reflection order remains meaningful, but canonical flags are not exposed as a history of actions.
- Orientation is available with a loaded source even before calibration; actions/resets are disabled during picking/measuring and crop drafts. Accepted orientation edits Fit the new composition. Turn/flip controls retain the crop's selected source content. Roll turns/flips act on the shared scanning baseline before unchanged per-photo exceptions; photo actions transform what is currently displayed. Scan orientation reset and Composition reset are independent and preserve colour/calibration settings; Composition reset clears photo orientation/framing, while Reset crop clears crop/straightening and leaves exact orientation intact. Draft controls cannot change grading/output state or save/export half-applied framing.
- Output remains visible below the scrollable edits, in the same position on both tabs. No empty Roll-post stage.
- Each scalar row has its label/value/unit/reset above a full-width slider. Calibration rows share availability, RGB display and a pick action.
- Gamma balance and WB share a color component: named/resettable container, wheel, warmth/tint rows, then neutral pick. There is no separate “Neutral” reset button or hidden fitted baseline.
- Gamma & balance reset changes gamma/balance only; Toe & shoulder reset restores shared toe **2.5** / shoulder **0** without changing other edits; exposure resets affect their own scope only; color reset restores the actual zero-coordinate identity. Calibration is never erased by an edit-section reset. Gamma resets to **0.65**, not 1.
- A model-4 classic-curve diagnostic state shows “Classic curve · camera model 4” and an explicit **Use adjustable response** action. Drawing controls never opts in; the action preserves calibration/gamma/balance/exposure and changes only response. A reset does not convert an original grade.
- Shoulder **0 = off**. Both linked numeric/slider parameters share actual processing/fitting state; inverse poles and unsupported GPU range/precision are rejected atomically.
- Implementation details live in tooltips. Unsaved state, approximation/refinement and actionable errors remain visible. Successful refinement no longer overwrites an earlier pick/edit error in the status line.

## Flat-field workflow

Choose reference RAW opens an uncorrected reference with a reference-specific automatic rectangular proposal, or manual selection if detection is ambiguous. Inspect/move/resize/edit an integer rectangle in untransformed original-source coordinates using the shared canvas mapping; photo crop/straightening and roll/photo orientation are temporarily bypassed for this reference-support edit only. Apply correction accepts it and returns to the working photo; Cancel restores accepted composition and the exact original view/state. Estimation excludes borders before smoothing and rejects clipped/unusable support. The reference must be film-free and from the same scanning setup; metadata compatibility alone does not prove matching lighting/aperture/focus.

The Roll section shows filename, Enabled, Edit area and Clear before Calibration. New reference selection never adds a filmstrip photo or hides an existing one. Failed/stale/cancelled operations retain accepted map, grade and working context. Enabled/Clear preserve viewport and inspection mode. Dmin/Dmax are never resampled; the setup/calibration warning recommends re-picking after correction changes. Enabled/map/support are saved; reference-edit context and inspection EV remain transient. No extra panel or toolbar.

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
- Fit uses the visible viewer canvas, excluding the controls panel. Picker header content wraps within that width; sampling shows the filename with the full path on hover, keeping Cancel accessible. Long paths/instructions cannot enlarge the fitted canvas.
- Click a fitted image for physical-pixel 100% around that point; click an explicitly zoomed image to Fit. Zoomed primary drag pans; middle/Space-primary drag also pans during picking. `z` Fits unless text input has focus.
- Picker primary click/drag retains patch/rectangle priority; zoom, grabbing, crosshair, move and corner-resize cursors follow the active interaction. Picks remain in original developed-source coordinates through shared composition mapping; resize cursors follow canvas direction after orientation.
- Starting a picker retains zoom/pan and exact orientation. Gamma/WB keep the displayed grade/balance; Dmin/Dmax enter developed-source inspection, with working-photo viewport restoration after temporary reference selection. Dmin/Dmax temporarily reveal uncropped oriented source, preserving physical magnification and the original source-coordinate canvas-centre anchor. Fit is also frozen through picker-toolbar canvas-size changes; unchanged exit restores Fit, and Gamma/WB retain deliberate in-picker navigation. [Coordinate/view requirements](geometry.md#panels-and-picker-views).
- Crop entry Fits the full active source with a handle margin, not the accepted crop. Resizing and release retain source magnification/placement; angle changes retain magnification and the screen pivot. Explicit 100%, +/− and **Fit source** remain available. Only Apply Fits the finished composition; Cancel restores the pre-edit viewport. The provisional editing view does not request settled reconstruction, including when explicit zoom places the frame offscreen.
- While developed-source view is active, **Inspection** in the existing viewer control row opens a zero-default exposure adjustment with the standard exposure range/reset. It changes only source-preview brightness before unchanged ACES; it is not saved and does not affect RAW development, calibration, fitting, edits or exports. Positive preview ignores it. No added viewer row/panel.

## Filmstrip thumbnails

The existing bottom strip shows approximate thumbnails of each accepted composition and filenames. Source/processed caches stay geometry-independent; crop/orientation/aspect are applied on paint. Selection is unavailable during a crop draft or flat-field support edit; a crop draft also suspends idle thumbnail scheduling. Drag its top edge to change height (default 140 points, 52…240 points and window-height limited); images follow available height with preserved aspect. Resizing, scrolling and caches are transient and do not dirty/save the project. Horizontally offscreen cells are virtualized; cache capacity does not dictate cell width. Visible entries are protected from eviction, recent entries support scrolling back, and normal-pressure idle prefetch for nearby and then unshown photos is lower priority and uses only spare retention budget. Optional retention scales with machine capacity and memory pressure; [cache contract](output.md#responsiveness-and-optional-caches).

Without calibration thumbnails show developed source at zero inspection gain; otherwise they show actual shared roll and each photo's grade through the existing GPU/ACES/managed presentation path. The viewer's source bypass/inspection EV does not change thumbnails. Enabled flat-field gains apply in original source coordinates; changing/clearing the map invalidates GPU image identity but reuses uncorrected RGB16 proxy cache entries. Normal selection and Dmin/Dmax reference-source selection retain their existing behavior. Loading placeholders and error markers/tooltips retain filenames and selection; thumbnails never supply measurements or export pixels. Verified selected development seeds a thumbnail instead of being developed again. Grade/view edits reuse source work, obsolete scroll jobs cancel between phases, and a disposable persistent RGB16 cache avoids re-development on later visits/restarts. Critical memory pressure suspends new thumbnail work without changing the grade or hiding existing visible images.

## Image scopes

A collapsible slot above Roll/Photo controls switches waveform/vectorscope without changing viewer dimensions. Height follows sidebar width and a draggable bottom divider adjusts the scope/controls tradeoff; double-click resets sizing. Vectorscope is the initial plot, with labelled 1×/2×/4×/8× magnification and reset; intensity compensates for smaller chroma bins when magnified. Waveform has RGB/luma modes. RGB/CMY direction ticks/letters remain on the vector rim when zoomed, distinct from magnified 100% reference boxes. A dashed Skin hue guide is explicitly approximate, not a correctness test. Showing scopes reduces grading scroll space while Output remains pinned. Both plots use the positive grade in the selected output's native encoded basis, labelled Whole frame / Approximate; source/reference/busy views and crop drafts suspend them. Accepted crop/orientation follows whole final-frame order, not viewer or source-bounding-box coverage. No project dirty/schema/export change. [Measurement, coverage and lifecycle contract](scopes.md).

## Resolve-style wheel orientation and mapping

Both wheels use the user's requested **blue at 3 o'clock, yellow at 9 o'clock**, with RGB/CMY proceeding clockwise. The displayed six sectors are equally spaced: R at 11, M at 1, B at 3, C at 5, G at 7, Y at 9. Artwork, labels, pointer input and marker positioning share the same orientation transform. This is a UI direction convention, not a claim about exact Resolve processing or the vectorscope's target-native luma/chroma measurement basis.

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

At the completed response-integration checkpoint, **88 all-feature tests**, strict all-target Clippy and formatting passed. [Recorded logs](../research/evidence/README.md) are historical evidence, not a claim about a fresh run. Headless egui event/layout tests cover normal/narrow widths, stable row IDs, unrounded/manual out-of-track values, normal/fine slider keys, mouse and Tab numeric entry, Enter/Escape, actual gamma default, pending-text disable transitions, wheel input/disabled behavior, adapters/full-square round-trips, local WB response, blue/yellow orientation, and independent section/color resets retaining calibration and other edit scopes. Response-specific tests cover precise/fine shoulder steps, true defaults, explicit original-grade opt-in, Roll/Photo section order and scrolling to Toe & shoulder/Exposure at 315×800 and 270×600 while Output remains pinned.

The real-image editor smoke has an optional layout pass:

```sh
ANTITONE_SMOKE_CONTROLS=1 cargo app --offline -- --smoke \
  samples/DSC04397.ARW 4000 5644 128 32 \
  samples/DSC04378.ARW 4000 2600 128 128 samples/DSC04416.ARW
```

It exercises both tabs at 1100×800 and 800×600 logical window sizes, with 315/270-point control panels. Current window-only screenshots/logs are ignored under `.cache/probes/film-model-images/response-controls-{roll,photo}-{normal,narrow}.png` and the retained [response-controls smoke log](../research/evidence/film-model-images/response-controls-smoke.txt). The response-enabled layout run completes in **31.49 s / 1,576 submitted frames** on M5 Pro, including the four timed holds. Screenshots confirm blue-right/yellow-left orientation, White balance before Photo Exposure and pinned Output; the added Roll rows require scrolling below gamma/balance at these window heights, covered by the mouse-wheel regression. These are diagnostic settings, not the user's photographic grade or broader-device/physical-monitor acceptance. Navigation/source inspection is implemented with focused event/GPU coverage and an unlocked 722-frame real-source smoke; physical trackpad feel remains unverified. Bounded thumbnails now have cache/identity/cancellation/parameter and event resize/selection coverage plus a four-source onscreen smoke. Responsive proxy scopes are implemented; [implementation/checks](../plans/completed/vectorscope-and-waveform.md). Full-source scope analysis and recovery/relink/undo remain separate steps. Basic Save/Open is implemented; [closure/checks](../plans/completed/save-open.md).
