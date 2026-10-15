# Local photographic adjustments

The scalar/persistence/export and GPU snapshot foundations are implemented. [Processing](../processing.md#local-grades-change-controls-not-rendered-pixels), [geometry](../geometry.md#source-anchored-local-masks), [projects](../projects.md#source-owned-local-adjustments) and [editor routing](../editor.md#immutable-local-grade-routing) explain current behavior. Normal editor Open/Save/source preparation/export still rejects nonempty local collections, including inactive records. Local actions/tools and admission are paused behind the shared [screen-sized working-image package](view-working-set.md); keep exercising the existing local snapshots through that work. Photographic/resource acceptance remains unfinished. [PLAN.md](../../PLAN.md) owns priority.

## Scope and non-goals

Build source-owned linear and radial gradients using the existing photographic WB, exposure, contrast/pivot/Colour preservation and saturation/Density controls. Global Roll/Photo processing remains the foundation; ordered locals follow before rendering/AP0 export. Composition is always whole-photo.

Keep model-4 native-camera inversion, signed/HDR AP1, existing source precision, exact settled/export process-before-average and numerical gates. The shared physical-view Global prototype must pass its remaining detail, interaction and ownership acceptance before local admission. No brushes, raster/AI/compound masks, local inversion/calibration/neutral fitting, cross-photo copying, automatic suggestions, recovery or general undo. Rename/enable/delete/reorder are in scope; duplication is optional follow-up.

## Responsiveness and supported-domain gates

Production uses exact per-code response reuse, endpoint bypasses and indexed local WB/constants. Expensive nonlinear stages still limit fresh-work latency; the shared equivalent ACES resource layout removed the measured rendering bottleneck on Metal. Continuous corrected samples retain direct high-precision inversion. Conservative range/normal-precision gates can reject modest photographic stacks; support depends on calibration, response, global grade and ordered local controls, not a count limit. Controlled supported-stack tests fix gamma at 0.65; the current default 0.60 is not equivalent admission evidence.

Scopes follow visible coverage and hold a coherent previous trace while updates are pending. The Global physical-view prototype shares its accepted target-linear viewer result with scopes rather than processing a whole retained proxy again. Its filtered neutral AP1 and footprint-centre masks remain preview approximations, not exact original-sample reconstruction. Never average camera samples before nonlinear processing to claim exact equivalence.

Gate before feature exposure: measure actual edit/pan/zoom-to-presentation feedback, redundant/obsolete work, source upload/reconstruction, CPU/device/image memory and rapid response/source/view-key churn. Arc/declared-buffer lifecycle tests do not establish aggregate or native in-flight memory acceptance. Re-measure integrated parameters/platforms; diagnostic smaller rasters are not a preview-detail policy. [Runnable probes and limits](../diagnostics.md#local-processing-and-reuse).

## One explicit editing target

Within Photo, keep Composition above an Adjustments list:

```text
Composition                         [always whole-photo]
Adjustments
  Global
  Linear 1                 [enabled]
  Radial 1                 [enabled]
  + Add adjustment

Editing: Radial 1
  Edit mask · Invert · Strength
  White balance
  Exposure
  Contrast
  Saturation
```

Global stays first and cannot be disabled/deleted/reordered. Local rows show processing order; start with explicit accessible Move up/down actions. Add Rename/Delete; deletion names the adjustment and confirms while undo is unavailable. Overlay visibility is independent of enabled state. Disabled records remain editable with a visible indication that they currently have no effect.

Reuse shared presentation and photographic controls, defaults/ranges and section-scoped resets. New locals start at identity; resetting values does not alter/delete their mask. Disable local neutral picking rather than reinterpret global fitting. Keep `Editing: …` visible.

Key pending numeric text by source, adjustment (or Global) and parameter. Resolve it using existing commit/cancel rules before target/list changes; input cannot leak into another adjustment. Use shared symbolic section expansion, not preferences per adjustment. Verify normal/narrow layout, accessible actions and popup/keyboard precedence.

## Geometry drafts and transactions

- Linear creation drags between unaffected/affected boundaries; body/endpoints translate, rotate or change width.
- Radial creation drags an ellipse; centre, axes, rotation and feather edit it.
- Use existing fixed viewer-bar identity/Apply/Cancel framing and shared UI roles, not another floating panel.
- Inverse-map gestures through accepted composition/view mappings into original source coordinates. Permit off-image/outside-crop geometry; navigation does not reanchor masks. Preserve entry composition/viewport; pan/zoom remains available, with inspector/popover input taking precedence.
- Creation/mask edits are cancellable drafts. Selected-image preview may show the draft; Save/Export and authoritative worker snapshots use accepted state only. Suspend scope/thumbnail draft propagation.
- While drafting, allow mask controls/navigation but lock unrelated processing, target/list/source changes and competing tools. Tab changes do not accept/cancel. Apply validates before installation; Cancel restores accepted geometry and entry view. Cancelled creation creates no accepted ID or dirty state.
- Numeric grading after mask acceptance is live. Geometry Cancel does not undo earlier accepted numbers; communicate that boundary so Apply/Cancel is not mistaken for an adjustment-wide session.

Identity masks must remain drawable through overlays. Pending selected previews can display accepted locals, but mask tools require authoritative preparation.

Smoothstep and weighted-control semantics are implemented; photographic feel is not accepted. Exercise noisy shadows, coloured light and broad/fractional exposure/WB/contrast/saturation gradients before choosing default feather, radial hard-edge availability/minimum radius and default strength. Do not add selectable falloff families or a wider strength range without need.

## Admission and integration

All candidates must validate the complete resulting stack before mutation: create/edit/delete/reorder/re-enable, Global/Roll/calibration/correction changes and Open. Preserve rejected accepted state, revisions and display. Validate offscreen source grades without retaining them. Analyze's candidate validation must keep the existing nonempty-grade rejection until complete local candidate preflight/admission is ready; full Photo reset and local-preservation/reset UX need an explicit rule before exposure.

Capture immutable processing snapshots in refinement/export; names/IDs/allocator/selection/overlays do not alter processing identity. Source/correction revisions remain independent. Accepted local edits invalidate rendered output, not RAW development or compatible proxy storage. Discard stale settled/scope/thumbnail results. Inspection and calibration/global fitting bypass locals. Preserve contract-only TIFF/EXR metadata; it is not a saved recipe.

Use analytic per-pixel stages, not one image/dispatch per adjustment. Validate full-stack support even when the acquired linear input is reused. Share immutable resources, never mutable image preparation/output. Export/reference local stages evaluate before full-source target/AP1 averaging with original origins and valid-edge handling. The working preview instead grades filtered linear AP1 and evaluates masks at mapped original-coordinate centres; photographic export correspondence, not sampling-order equality, decides whether that preview is acceptable.

## Build order and acceptance

1. Accept the [implemented physical-view Global prototype](view-working-set.md) while testing representative local snapshots. Demonstrate stable fixed-view detail, deferred navigation detail, shared viewer/scope processing and complete latency/resource ownership before resuming local admission/tools.
2. Resolve numerical/resource admission, then Global/local selection and atomic list/numeric actions. Check disabled input, target-scoped reset, stale results and strict persistence.
3. Add cancellable linear interaction; physically exercise exposure feathers, off-image handles, Apply/Cancel and normal/narrow layouts before radial complexity.
4. Add radial handles/feather; check overlap/order, falloff and all controls on actual photographs. Reopen Save/Open and both-target TIFF/AP0 EXR, with correction and orientation/refinement seams.
5. Keep current topic guides/architecture/checks aligned and deferred acceptance explicit; remove this plan only after remaining scope has a proper home. No push or main integration is implied.

Acceptance includes exact inactive/identity/zero/full endpoints; independent fractional mask/control fixtures; signed/HDR AP1/floor flags; actual proxy centres and tile seams; atomic unsupported reorders/re-enables; immutable snapshots; missing/rejected/stale-state preservation; full-source export before averaging; real pointer/trackpad, skin/noisy shadows and Fit/native agreement. Numerical/headless checks do not establish photographic, native-shell, monitor or broader-device acceptance.

## Exact next action

Finish [physical-view prototype acceptance](view-working-set.md#next-checkpoint), not local UI yet. Preserve local foundations and use representative local snapshots to prevent a Global-only working-image design. Verify complete-chain 1080p/4K latency, sampling quality and concurrent ownership; keep the existing precision gates, stale-result rejection and hosted submission gate.

Normal editor admission, local actions, gradient tools, aggregate/in-flight memory and broader corrected/local RAW/export, photographic, physical and device acceptance remain unverified. Use [focused checks](../development.md#local-grading-and-shared-consumers) and [diagnostics](../diagnostics.md#local-processing-and-reuse), not a reflexive full suite.
