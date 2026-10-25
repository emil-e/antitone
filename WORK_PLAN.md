# Make Negatives Great Again

## Agreed scope

Implement issue #6 together with display-only daylight/as-shot white balance for negative source inspection. Use daylight by default; share source-preview WB and manual exposure between canvas and source-preview thumbnails. Keep calibrated positive thumbnails unchanged. Unity-WB development, calibration measurements, accepted grades, recipes and export stay independent of these transient viewing settings.

## Acceptance

- Negative source previews use available camera daylight/as-shot metadata without redeveloping RAWs.
- Canvas and source-preview thumbnails share manual inspection EV and viewing WB; source-specific metadata is respected.
- Switching inspection settings does not alter project state, grade identity, recipes or processing measurements.
- Missing/invalid WB is handled safely, including direct DNG sources.
- Component and composed UI/render tests verify behavior; relevant contributor docs describe the result.

## Implementation choices / established results

- LibRaw uses camera-channel viewing-gain ratios conjugated into the cached AP1 basis. Direct DNG uses the SDK's profile-adapted daylight/as-shot matrices, without changing its decode or interpretation identity.
- Header preparation retains per-source viewing transforms in the transient Document keyed by stable SourceId; selected ready sources retain native metadata. No metadata IO or redevelopment is needed while drawing.
- Source-preview parameters share viewing WB and inspection EV across canvas and source cards. Positive cards and Digital's already-as-shot source balance remain separate.
- Physical-view inspection now applies a full display matrix instead of a scalar; fullscreen restores viewing WB alongside inspection EV.
- The viewing-WB control uses two labelled selections in the existing popup. This avoids a nested dropdown closing the inspection controls when its option is selected.

## Verification

All final checks passed; logs are disposable under `.cache/`:

- Debug targeted editor/Metal/UI/source/document/history regressions: 204 passed, 3 existing ignored sample-dependent cases (`regressions.log`).
- Release source-inspection/numerical/composed checks: 23 passed (`release-tests.log`).
- SDK unit tests, shared row component and synthetic guide capture workflow: 24 passed (`guide-native-tests.log`).
- Hardware-independent core: 159 passed (`core-tests.log`).
- Production editor release build, without reference features (`production-build.log`).
- UI/editor/native test-target Clippy with `-D warnings` (`clippy.log`).
- Authored-source formatting, documentation links and `git diff --check` passed.
- Inspected the composed inspection-popup capture and synthetic Analyze-review capture: controls fit; settled canvas/source-card colours agree. Captures remain under `.cache/` and are not committed photographic documentation assets.
- Independent fresh-context review found no actionable findings. It examined whole-case/assertion quality and fixture/control discovery. Explicit stage/history viewing-metadata tests and independent dual-illuminant/ForwardMatrix daylight references remain coverage limits.

## Integration scope

User requested integration of the completed viewing changes first. Digital input brightness normalization is follow-up only and is excluded from this PR. Integration authorizes checkpoint commits, rebase, photographic guide regeneration, PR publication and CI monitoring; merging remains the user's action.

## Remaining before integration

1. Rebase onto current fetched main and resolve/recheck adjacent changes: main advanced from 820cfb3 to d382d8d (#59, retained-source removal). Relevant changes and current AGENTS.md were inspected; this task's uncommitted work remains on its original base.
2. Inspect real-negative appearance and refresh photographic guide captures. The documented full-roll regeneration is about seven minutes; get approval for that workload or an explicit deferral before integration. Synthetic guide images must not replace the maintained photographic assets.

## Current state / next action

Implementation is complete and locally verified in `.worktrees/make-negatives-great-again`, branch `feat/make-negatives-great-again`. Integration is now requested. Next: checkpoint and rebase onto fetched main, regenerate/inspect photographic guides, recheck integrated result and publish PR. Shared native dependencies reused through documented LIBRAW_SOURCE/OCIO_PREFIX overrides and native-cache link.
