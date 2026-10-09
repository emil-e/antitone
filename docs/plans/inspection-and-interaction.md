# Source inspection and interaction feedback

Scope basis: remaining approved feedback pass; not started. [Roadmap](../../PLAN.md) owns order/status. Completed Dmin/toe and output work are not to be reopened by default.

## Goal and boundaries

Make developed-source inspection and Mac image navigation usable without changing calibration, export, response, RAW development or ACES. Preserve full-resolution picker coordinates, atomic rejection/cancellation and working-photo/view restoration.

## Decisions and open questions

- Normal two-finger scrolling should pan; pinch zoom remains pointer-anchored. Retain an explicit modified-wheel zoom path for mouse users without applying gestures twice.
- Outside picker mode, a fitted-image click should zoom to native pixels around that point; zoomed primary drag pans. Distinguish click/drag, provide a clear zoom-out/Fit interaction and matching zoom/grab cursors.
- Picker click patches, rectangle creation/move/corner resize retain priority over ordinary navigation and appropriate cursors.
- `z` should Fit. The reported stale fitted resolution needs reproduction/diagnosis; existing refinement is implemented, so do not assume downsampling is absent.
- Preview sometimes fails to update with **“Settled preview failed: GPU processing: invalid area-average dimensions or GPU budget”**. Reproduce the failing view/window state and diagnose dimensions, resource budget and refinement scheduling; do not mask the error by silently weakening budgets or skipping refinement.
- The user explicitly wants consultation for **each existing UI insertion that changes viewer size** before choosing a replacement. Gather the cases/screenshots first. Tab prominence and Output placement feedback also need a current-layout discussion; current pinned Output was previously approved.
- Dark-source normalization/inspection needs its own check. If correct but inconvenient, a clearly separate source-view brightness control may be appropriate; it must never alter measurement/development/export. Exact UX/default is not decided.
- Red rendering appears dark in user feedback; exact source/settings are not recorded. Investigate only with a reproducible grade before claiming an inversion-basis defect or changing processing policy.
- Toe lift produces rather cold shadows, at least with **Vision3 500T**. The user suspects that the tungsten stock's greater blue sensitivity could leave only blue when other channels fall below a threshold; this is an unverified hypothesis, not an established cause. Capture the RAW, calibration, response and Roll/Photo settings, then trace near-base signed working channels through rendering before proposing a remedy. Do not silently warm shadows, add a pedestal/clamp or reinterpret the accepted response.
- Give the remaining UI a focused UX-polish pass: review unnecessary descriptions/redundant scope wording, hierarchy and action placement across calibration, editing and navigation. Controls and copy already cleaned up are not a new redesign requirement; keep actionable errors and concise state indicators.

## Steps and acceptance

1. Reproduce remaining source/navigation/refinement/layout and colour symptoms against current code; separate already-fixed issues from outstanding ones, discuss viewer-intrusion cases and review remaining UX clutter. Distinguish the Vision3 cold-shadow observation from its proposed cause; any processing-policy change still requires separate agreement.
2. Specify presentation-only source inspection state and implement only the agreed behavior. Verify bypass, calibration/fitting/export independence and invalid/cancelled picks.
3. Implement gesture/click/cursor/Fit behavior with event/geometry tests and actual Mac trackpad exercise. Verify anchors, bounds, physical-pixel 100%, click-vs-drag and picker isolation.
4. Diagnose reproduced stale-resolution and area-average-dimensions/GPU-budget failures at their source. Verify fit/native/resize/rapid-switch behavior, valid dimensions/resource bounds and stale-result rejection, without perpetual idle redraw or weakened budget checks.

Likely files: `src/{editor,viewport,viewer,refinement,resample,editor_tests,editor_smoke}.rs`; `src/platform/host.rs` only for a demonstrated event translation issue. Numeric/processing policy changes require separate agreement and regression gates.

## Progress / handoff

No implementation or new verification in this package. Existing code has source bypass, fit/100%, wheel/pinch zoom, several pan gestures and tiled regional refinement. This work refines those paths rather than implementing them from scratch.

Next action when selected: inspect current viewer event handling and reproduce the remaining trackpad/source-inspection cases; ask the user which viewer-intrusion cases to address before changing their layout.

Evidence/context: remaining symptoms were consolidated from `docs/feedback.md`; it is now an inbox, not the persistent requirement record. See [historical approved pass](../research/history/feedback-plan.md), [architecture](../architecture.md) and [control contract](../contracts/editor-controls.md).
