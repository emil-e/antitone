# Verification tooling: native boundaries and contributor handoff

Status: broader native input coverage and contributor handoff remain unfinished. Current [framework UI regressions](../development.md#framework-ui-regressions) and [native-host checks](../development.md#explicit-native-host-checks) are implemented, separately scoped boundaries. [Architecture](../architecture.md#ui-test-boundary) explains ownership; the [authored fixture guide](../../tests/fixtures/raw/README.md) and [diagnostics](../diagnostics.md) own current commands and limits. [Roadmap](../../PLAN.md) owns priority.

## Goal and scope

Keep a small verification system shared by humans and agents, with durable regression coverage, independent references and explicit optional real-use acceptance. Contributors should understand prerequisites, failures and limits without private photographs or execution histories.

Remaining scope is concrete native-host input checks, unfinished fixture/thumbnail boundaries and contributor-command organization. Preserve application behavior, private Editor ownership, shared UI components, project/preferences formats, processing/development identity and numerical tolerances. Keep pinned LibRaw/OCIO, native-camera calibration, signed/HDR AP1, AP0 EXR and compact RGB16 sources.

Non-goals: redesigning the editor or processing, adopting eframe, a bespoke widget/action framework, an automation server, port implementation, new camera support or establishing physical photographic/input/performance/monitor acceptance. Do not expand [Analyze photographic acceptance](roll-assistance.md).

## Unfinished native coverage

- Identify a concrete missing pointer/keyboard forwarding contract in the production host and add the smallest explicitly launched check for it. Framework egui events do not establish host forwarding.
- Keep common input/fixture expectations separate from platform adapters. macOS/Metal is the current verification platform, not the product boundary.
- Preserve main-thread window execution, opt-in `--run`, actual surface-paint/presented-frame gates, explicit completion and failure on interruption/timeout. Missing hardware or permissions never become success.
- Broader per-tool Editor-in-window appearance, native dialogs, physical trackpad/keyboard/VoiceOver, preferences relaunch, monitor matching, macOS 14 and broader devices/platforms remain separate acceptance.
- Do not add general OS accessibility automation or visual baselines without a concrete contract and separately selected scope.

## Unfinished fixture and worker boundaries

- Broader corrected-thumbnail coordination and proprietary-camera preview behavior remain unproved. Named authored tests establish map-keyed thumbnail readiness and document retention, not universal coordination or photographic correctness.
- Adobe DNG Validator conformance remains unestablished. Pinned LibRaw decoding is not general DNG conformance.
- Initial-path opening in `Editor::new` still recognizes only ARW. Picker filters do not establish CLI startup support for other RAW suffixes; keep this separately scoped entry-point gap explicit.
- Extend fixtures only for an uncovered gate. Keep authored inputs read-only, temporary copies/outputs isolated, production sample minimums and numerical thresholds unchanged. No downloads or private inputs for ordinary regressions.

## Contributor handoff

- Keep core/RAW/UI/reference/native/optional-real-use commands in their existing guides; remove duplicated explanations from this plan when their owning guide is sufficient.
- Inspect remaining named private-photo diagnostic recipes and retain only present consumers/questions. Supplied-source real-use acceptance must not become a default regression prerequisite.
- Retained probes need concise purpose/input/output/failure/limit headers. Reports, successful captures and implementation self-comparison are not independent physical acceptance.
- Preserve upstream OCIO → CPU/Metal, Apple transfer/ICC, broad native gamut and ImageIO storage/profile boundaries. No cache-selected coverage or stale fallback after reference generation failure.
- Keep test-framework/support code excluded from ordinary application builds. Ordinary all-feature tests must not unexpectedly open windows or write personal state.
- Keep generated captures under ignored storage, named as offscreen/native evidence as appropriate. Do not archive tools/plans or auto-approve appearance with snapshots.

## Acceptance and exact next action

Run focused checks/lints/parser/link checks for each selected boundary and inspect the diff. Framework interactions must use actual widgets with bounded observable waits; native checks require paint/presentation and completion, including negative cases. Independent numerical thresholds and source/development identities remain unchanged. Update guides and remove this plan when its remaining work has proper homes; do not retain a completed migration journal.

Next: inventory production pointer/keyboard forwarding and existing host tests, select one concrete uncovered event boundary, and confirm the narrow native-check scope before implementation. Keep broader per-tool Editor/native and physical acceptance explicit. Separately inspect remaining diagnostic recipes for present utility; no blanket UI rewrite or golden-image adoption.
