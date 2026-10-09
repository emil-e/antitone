# Editor control contract

Current UI/control behavior. Remaining work is scheduled in [the roadmap](../../PLAN.md), not in the verification snapshot below.

The editor uses `src/editor_controls.rs` for a small shared hierarchy: **section → parameter row → specialized input**. `Editor::controls` owns roll/photo grouping and reset scope; the helpers own spacing, entry, range, focus and presentation. New grades use adjustable film response/model 3; original model-2 grades are retained without implicit opt-in. Project schema 3 and the development/output contracts are unchanged.

## Layout and actions

- Roll: Calibration (RGB-only Dmin/Dmax) → Gamma & balance → Toe & shoulder → Exposure.
- Photo: White balance → Exposure. Geometry is not shown until implemented.
- Output remains visible below the scrollable edits, in the same position on both tabs. No empty Roll-post stage.
- Each scalar row has its label/value/unit/reset above a full-width slider. Calibration rows share availability, RGB display and a pick action.
- Gamma balance and WB share a color component: named/resettable container, wheel, warmth/tint rows, then neutral pick. There is no separate “Neutral” reset button or hidden fitted baseline.
- Gamma & balance reset changes gamma/balance only; Toe & shoulder reset restores shared toe **2.5** / shoulder **0** without changing other edits; exposure resets affect their own scope only; color reset restores the actual zero-coordinate identity. Calibration is never erased by an edit-section reset. Gamma resets to **0.65**, not 1.
- Existing model-2 grades show “Original response · model 2” and an explicit **Use adjustable response** action. Drawing controls never opts in; the action preserves calibration/gamma/balance/exposure and changes only response. A reset does not convert an original grade.
- Shoulder **0 = off**. Both linked numeric/slider parameters share actual processing/fitting state; inverse poles and unsupported GPU range/precision are rejected atomically.
- Implementation details live in tooltips. Unsaved state, approximation/refinement and actionable errors remain visible. Successful refinement no longer overwrites an earlier pick/edit error in the status line.

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

## Resolve-style wheel orientation and mapping

Both wheels use the user's requested **blue at 3 o'clock, yellow at 9 o'clock**, with RGB/CMY proceeding clockwise. The displayed six sectors are equally spaced: R at 11, M at 1, B at 3, C at 5, G at 7, Y at 9. Artwork, labels, pointer input and marker positioning share the same orientation transform. This is a UI direction convention, not a claim about exact Resolve processing or a finalized vectorscope measurement basis.

In a right/up plane, the existing opponent coordinates are projected with:

```text
P(rgb) = ((R-B)/sqrt(2), (R-2G+B)/sqrt(6))
O = [ -sqrt(3)/2   1/2       ]
    [  1/2        sqrt(3)/2  ]
```

- **Gamma balance:** display matrix `O`. Coordinates still control relative log gamma in AP0, not RGB gains. Pixel response depends on the negative/calibration.
- **White balance:** display matrix `O * J`, where `J` is the exact differential AP1 neutral-white response to the existing Bradford log-cone-gain coordinates. With `C = Bradford * AP1_to_XYZ`, its columns are `P(C^-1 * diag(C*white) * opponent(1,0))` and the corresponding tint column. This is a local display adapter, not a replacement WB operator.

Each matrix is normalized by the maximum mapped corner length of the legal warmth/tint square. Input uses its inverse; coordinates outside the legal square are clipped to the existing ±1 limits. Wheel, sliders, numeric entry and fitted neutral corrections all share the same `Balance` state. There is no new saturation/strength state, physical Kelvin interpretation, or processing/profile change.

## Verification

At the completed response-integration checkpoint, **88 all-feature tests**, strict all-target Clippy and formatting passed. [Recorded logs](../research/evidence/README.md) are historical evidence, not a claim about a fresh run. Headless egui event/layout tests cover normal/narrow widths, stable row IDs, unrounded/manual out-of-track values, normal/fine slider keys, mouse and Tab numeric entry, Enter/Escape, actual gamma default, pending-text disable transitions, wheel input/disabled behavior, adapters/full-square round-trips, local WB response, blue/yellow orientation, and independent section/color resets retaining calibration and other edit scopes. Response-specific tests cover precise/fine shoulder steps, true defaults, explicit original-grade opt-in, Roll/Photo section order and scrolling to Toe & shoulder/Exposure at 315×800 and 270×600 while Output remains pinned.

The real-image editor smoke has an optional layout pass:

```sh
ANTITONE_SMOKE_CONTROLS=1 CC=/usr/bin/clang CXX=/usr/bin/clang++ \
  ./scripts/cargo.sh run --offline --release --features editor --bin antitone -- --smoke \
  samples/DSC04397.ARW 4000 5644 128 32 \
  samples/DSC04378.ARW 4000 2600 128 128 samples/DSC04416.ARW
```

It exercises both tabs at 1100×800 and 800×600 logical window sizes, with 315/270-point control panels. Current window-only screenshots/logs are ignored under `.cache/probes/film-model-images/response-controls-{roll,photo}-{normal,narrow}.png` and the retained [response-controls smoke log](../research/evidence/film-model-images/response-controls-smoke.txt). The response-enabled layout run completes in **31.49 s / 1,576 submitted frames** on M5 Pro, including the four timed holds. Screenshots confirm blue-right/yellow-left orientation, White balance before Photo Exposure and pinned Output; the added Roll rows require scrolling below gamma/balance at these window heights, covered by the mouse-wheel regression. These are diagnostic settings, not the user's photographic grade or M1/physical-monitor acceptance. Trackpad/click-zoom work, thumbnails/scopes and Save/Open/recovery/undo remain separate approved steps.
