# Verification tooling and UI test structure

Status: unfinished fixture-regression migration, reference and UI restructuring. Use the [authored DNGs](../../tests/fixtures/raw/README.md) and [supported diagnostics](../diagnostics.md) as the starting coverage. General composed-UI/native presentation remains unproved. No UI-test feature or framework integration exists. [Roadmap](../../PLAN.md) owns priority; this plan does not change active photographic work or shared UI ownership.

## Goal

Replace accumulated probes, private-photo recipes and scripted application modes with a small, understandable verification system shared by humans and agents. Keep durable regression and independent reference coverage; delete one-off machinery instead of preserving it for hypothetical future investigations.

A contributor should be able to identify what a check exercises, its prerequisites, what failure means, and what remains unverified without reading a session transcript or discovering an author's photographs.

## Scope and non-goals

Scope: test data/support, standalone diagnostic targets and scripts, native references, editor automation, capture helpers and their contributor documentation. Include the current Analyze diagnostics and smoke route, not just the older editor/flat-field routes. Inspect the current code and callers before each migration slice.

Preserve application behavior, private editor ownership, shared UI components, project/preferences formats, processing/development identity and existing numerical tolerances. Retain pinned LibRaw/OCIO, native-camera calibration, signed/HDR AP1, AP0 EXR and the compact RGB16 source. See [RAW rationale](../decisions/established-raw-development.md) and [rendering rationale](../decisions/sdr-output-and-presentation.md).

Non-goals: redesigning the editor or processing, adopting eframe as the application host, building a bespoke widget/action framework, an automation server or new CI service, adding camera support, distributing photographic datasets, and establishing photographic, physical-input, performance or monitor acceptance. Use the implemented [shared UI components](../architecture.md#editor-workspace); do not expand [Analyze acceptance](roll-assistance.md).

## Target separation

| Layer | Purpose | Inputs and execution | What it does not establish |
| --- | --- | --- | --- |
| Core regressions | Algebra, state, persistence and source-coordinate contracts | Small in-memory fixtures; ordinary Cargo tests | RAW decoding, native presentation or photographic quality |
| RAW integration | Real LibRaw/Scan/development/export plumbing | Checked-in authored DNGs; native-feature tests | Proprietary-camera support or calibrated scene colour |
| Independent references | OCIO/Metal agreement, gamut, ICC, transfer and external decoding | Deterministic synthetic inputs; focused native/GPU checks | Photographic preference or physical-monitor matching |
| UI interactions | Actual widget actions, focus, transactions and composed layout | Framework-driven egui events; deterministic state/DNG fixtures | AppKit routing, native dialogs or physical gestures |
| Native-host checks | Surface/window lifecycle, presentation and host input forwarding | Separate explicitly launched test runner; unlocked desktop where required | Whole-device or physical-input acceptance |
| Visual evidence | Named rendered states and failure screenshots for inspection | Opt-in capture or reviewed snapshot checks | Automatic approval of appearance or usability |
| Optional real-use acceptance | Camera behavior, photographs, actual resource costs and human interaction | Explicitly supplied read-only sources/devices | A reproducible default regression suite |

Keep common behavior and fixture data separate from platform adapters. Platform-specific references such as ImageIO, ColorSync and Metal validate the current macOS paths; they must not make those APIs part of the cross-platform export or interaction contracts. Porting the application and establishing other platforms' native checks are outside this verification slice.

Build/setup scripts remain the normal shared infrastructure. No agent-specific Cargo wrapper or second bootstrap workflow.

## Checked-in RAW fixtures

### Data and provenance

- Store a small authored set under `tests/fixtures/raw/`, outside ignored private `samples/`. Check in the DNG binaries themselves: ordinary checks must not download or generate their required RAW inputs.
- Start with uncompressed 16-bit, 2×2 Bayer DNG. Validate against the [DNG specification](https://helpx.adobe.com/camera-raw/digital-negative.html) and the pinned LibRaw implementation, not just a TIFF viewer.
- Supply consistent CFA, black/white, WB, camera matrix, active area and synthetic camera identity. Do not masquerade as a real camera or add production special cases to accept fixtures.
- Keep a small maintained generation recipe in `scripts/` and a fixture README explaining named patches, metadata, dimensions and intended assertions. Regeneration must be reproducible, with no wall-clock/random/path-dependent metadata. The script is maintenance tooling, not a prerequisite for running tests.
- Checked-in fixtures are read-only. File-edit/revision/error tests operate on temporary copies. Projects, caches, exports, logs and captures use isolated temporary or ignored output directories; do not load or overwrite a developer's preferences or grades.

### Remaining fixture coverage

Reuse the [current fixture set and its named assertions](../../tests/fixtures/raw/README.md) before adding variants. Add data only for an uncovered gate, such as corrupt metadata or a required native-pixel viewport. Keep direct JPEG/ICC unit fixtures for decoder-only checks; synthetic RAW success does not replace proprietary-camera acceptance.

Start with dimensions in the hundreds, not full camera resolution. Determine the actual minimum from patch support, crop/refinement, Analyze adequacy and viewport tests. Do not weaken sample-count or numerical gates to accommodate tiny fixtures. Native-pixel tests may need a smaller viewport or one deliberately larger fixture. Generate large workloads only for explicitly selected resource/performance exercises.

Use sensor/metadata expectations and independent numerical references, not outputs blindly accepted from the implementation under test. Preserve complete RGB16/thread-policy agreement where the contract requires it. Synthetic roll/flat-field results test declared behavior, not the physical quality of those estimators.

## Probe and diagnostic disposition

Preserve the listed coverage, not every existing executable. Remove a gate-bearing program only after its supported domain, thresholds and independence have an exercised replacement. No new baseline snapshots, archives or dormant comparison packages.

| Current surface and inputs | Current assertions / reports | Replacement layer and unfinished treatment |
| --- | --- | --- |
| `native/ocio_probe.cpp`, `src/bin/{gpu_probe,rendering_probe}.rs`, their scripts; generated `.cache/probes` inputs | AP1/AP0 CPU equivalence and production CPU/Metal agreement: `abs(error) <= 2e-5 + 2e-4 * abs(reference)`; OCIO finite RGB/LUTs and neutral spread `<= 2e-5`. Metal also validates explicitly requested camera model-4 RGB16 inversion/matrix/floor flags. Printed maxima/resource counts are reports, not extra gates. | **Independent references:** retain the upstream OCIO → CPU/GLSL/LUT → Metal chain and camera artifact freshness checks. Consolidate only actual duplication. |
| `native/profile_reference.mm`; rendering-probe ICC/encoded/linear artifacts | Independent Core Image/ColorSync conversion: finite RGB and absolute error `<= 2e-4`, both targets, toes and gamut corners. No window. | **Independent references:** retain Apple conversion and generate prerequisites explicitly. |
| `native/presentation_reference.mm`, `src/bin/presentation_probe.rs`; transfer artifacts and macOS desktop | Core Image signed/HDR transfer error `< 3e-5`; GPU f32/half readbacks at `2e-5`/`0.001`, UI checks, managed-layer state, resizes and noninteractive submitted-frame gate. | **Independent references + native host:** separate offscreen checks from explicit main-thread window execution; preserve both boundaries. |
| `native/gamut_probe.cpp`, its script and Rust `--gamut-check`; synthetic AP1 | Native 10³ grid per target: finite RGB, 990 coloured cases must have spread `>= 1e-6`. Rust checks just yellow spread `>= 0.01` and neutral spread `<= 2e-5` in common linear sRGB. | **Independent references:** retain broad native coverage; the yellow-only check cannot replace it. Consolidate only after equivalent exercised coverage. |
| `native/raw_development_probe.cpp`; supplied RAW/WB reference, fixed rectangle | Fails on changed regional/whole sensor diagnostics and unrestored OpenMP task policy. Prints complete RGB/metadata/diagnostic digests for cross-budget comparison and phase timings; the executable alone does not assert cross-run equality. | **RAW integration + optional native diagnostic:** fixture-derived regions, complete RGB16/thread agreement and explicit diagnostic command. Retain the current supplied-source route meanwhile. |
| `tests/raw_dng.rs`; authored DNG and temporary copies | Scan cache/coordinates/failed-pick immutability, sensor-white/black rejection, measured inversion anchors `2e-12`, channel flooring rate `<= pixels/100`, and nonempty eligible LibRaw matrix comparisons `2/65535`. | **RAW integration:** retain gates while extending concrete rejection/compatibility cases. |
| `scripts/camera-reference.rs`; committed patch DNG plus two declared code-floor cases | Writes five RGB16/matrix/parameter/CPU-working/flag artifacts. `scripts/probe-ocio.sh --camera` regenerates and explicitly includes them; missing/stale requested artifacts fail. | **Independent-reference support:** preserve input provenance and the distinction between scalar implementation references and independent upstream OCIO output. |
| `src/bin/raw_probe.rs`; supplied RAWs | Full mode: channel flooring count `<= pixels/100`, exact sampled repeated camera development, eligible in-sRGB LibRaw matrix error `<= 2/65535` (zero comparisons does not fail). Metadata/statistics/synthetic anchors and optional unmanaged PPMs are reports. | **Optional supplied-source diagnostic:** retain for actual camera inspection; authored regression checks do not establish broader-camera acceptance. |
| `tests/{export,raw_dng}.rs`, `native/tiff_reference.mm`; synthetic/authored inputs and temporary exports | Rust checks exact TIFF/AP0 EXR pixels/metadata and destination safety. Named opt-in ImageIO test independently checks one-image TIFF, declared dimensions, RGB16 decoding/storage and exact ICC bytes, including rejection cases. | **RAW integration + independent references:** retain independent decoder checks; large photographic exports and external-application acceptance remain separate. |
| `src/editor_analysis_diagnostics.rs`, `native/image_contacts.m`; explicit private case manifest and fresh output | Actual Editor import/correction/Analyze invariants and unchanged failure state; JSON reports and 384px contacts. A diagnostic run can complete with failed analysis/render cases. ImageIO source previews and six-column contacts are inspection tools, not acceptance gates. | **Optional real-use acceptance:** retain narrowly scoped current-Analyze inspection. Keep synthetic estimator/transaction coverage in core/editor tests; require a concrete question before adding diagnostic modes. |
| `src/editor*smoke.rs`, `ANTITONE_SMOKE_*`, capture scripts and window locator; supplied RAWs/desktop | GPU/worker/picker/project/correction/Analyze scenarios and visible-frame/layout-marker checks. Ordinary/project smoke lacks an explicit completed-success result; flat-field/Analyze checks reject early close. Capture success does not approve appearance. | **UI interactions + native host + visual evidence:** migrate scenarios before deleting routes. Use actual widget events, explicit completion and capture adapters rather than phase/log-polling machines. |

Current non-private estimator coverage includes the neutral-ramp/manual picker and HDR-outlier exposure tests in `tests/{camera_inversion,roll_analysis}.rs`, weighted sampling in `tests/processing.rs`, production tonal-fit tests in `src/roll_analysis_gamma.rs`, and editor Analyze transaction tests. Keep those actual-production checks; do not substitute tests of duplicate diagnostic optimizers or unused weighting variants. Production JPEG extraction/immutability/orientation/ICC/replacement tests and authored-preview RAW integration remain.

Each retained probe starts with a concise comment explaining its purpose, inputs/prerequisites, generated outputs or side effects, failure conditions and verification limits. Distinguish a diagnostic report from an assertion-bearing gate; a successful render/contact-sheet generation is not photographic acceptance. Put supported commands in contributor guides, not long instructions inside source comments.

## Structured UI automation

### Framework proof and ownership

[egui_kittest 0.36.2](https://docs.rs/egui_kittest/0.36.2/egui_kittest/) is the first candidate: it matches current egui/wgpu and offers AccessKit queries/actions, controlled frames and offscreen rendering. This is a proposed test dependency, not an application-framework decision. Do not adopt eframe or expose private editor fields merely to fit the harness.

Prove one isolated shared component and one composed Editor scenario, including a real custom GPU paint callback. Check shared device/queue resources, texture/pipeline formats, precise rendering, backend completion/repaint behavior and accessibility names. If integration fails, identify the actual constraint before choosing another library or writing an adapter.

Reuse existing input/AccessKit tests, `Editor` fixtures, component tests and production worker/rendering paths. Fixture setup may initialize state or open a document; the interaction being asserted must use real widget input rather than direct field assignment. Keep private-editor scenarios near the editor's test modules. Add only narrow support for fixture setup, bounded waits, rendering and result artifacts; no parallel state machine, action DSL or widget registry.

### Build and runner boundaries

- Add an explicit opt-in test feature, provisionally `ui-tests`. Gate framework dependencies and automation support so ordinary numerical-core and `cargo build-app` builds stay unchanged/lightweight.
- Do not use `debug_assertions` as the boundary: meaningful GPU/integration checks must also run release-optimized.
- Keep framework-driven UI cases as normal named Rust tests. Distinguish GPU requirements from the absence of a desktop/window requirement.
- Put the small real-window suite in a separate gated test executable sharing `platform::host`. AppKit requires a main-thread event loop; do not rely on ordinary Rust test worker threads for this runner.
- Normal application binaries expose no `--smoke*` routes or capture environment switches after migration. `cargo test --all-features` must not unexpectedly open windows or write personal state; native-window execution is explicit.
- Define one documented command per check class with filtering and explicit capture/output options. Exact target names and feature wiring are settled by the proof, not a second Cargo wrapper.

### Actions, assertions and completion

Initial composed scenarios cover existing contracts: Roll/Photo/Rendering controls and resets, numeric focus/disabled safety, source switching, crop/picker Apply/Cancel, correction, Analyze confirmation/cancellation/atomic reinitialization, Save/Open preservation, and normal/narrow/scaled layouts. Reuse isolated tests rather than duplicating every numerical case in UI automation.

Use semantic labels/roles where supported; canvas geometry comes from the actual layout and fixture patch description, not photographic coordinates or global screen pixels. Missing semantics should be repaired at the real control, not bypassed with test-only UI.

Wait on observable conditions with deadlines and useful failure state. No fixed screenshot sleeps or stdout-marker polling as the synchronization contract. Every runner has explicit completed-success, failed, timed-out or interrupted results; early closure never passes. Preserve visible-frame gates for native checks. Missing devices/session/permissions are reported as unavailable or failed, not silently converted into success. Exercise negative paths as well as successful completion.

### Screenshots and native limits

Capture named UI states and failure artifacts under ignored storage. Assertions establish behavior/geometry; an agent or human may inspect captures, but successful capture is not approval. Offscreen captures must be labelled as such and cannot replace surface/ColorSync checks.

Snapshot comparisons are optional until a real visual contract warrants them. Keep generated captures/diffs out of source by default. Checked-in visual baselines would be an explicit, reviewed test-fixture exception to the current generated-screenshot policy; settle that separately after the framework proof. Never auto-update baselines or widen numerical tolerances to obtain a pass.

The native suite remains small: window/surface presentation, resize, close/completion and selected input forwarding. Full macOS accessibility automation, native-dialog coverage and physical trackpad/keyboard/VoiceOver acceptance remain separate choices. Headless egui actions do not claim those boundaries.

## Ordered implementation slices

1. **Fixture regression migration and probe consolidation.** Replace private-file dependencies and hidden cache prerequisites in maintained automated checks. Make the remaining rendering/ICC/transfer prerequisites explicit while preserving broad gamut coverage and current Analyze numerical/transaction tests.
2. **UI framework proof.** Wire the optional dependency/feature and demonstrate semantic input, asynchronous state, custom GPU rendering and a named capture. Decide renderer adapters and snapshot policy from this proof; do not rewrite all UI tests first.
3. **Scenario/native migration.** Port one end-to-end scenario at a time into framework tests or the small native suite. Exercise failure/timeout/early-close behavior and production build exclusion. Remove equivalent smoke classes, CLI/env branches and capture scripts as replacements pass; do not leave parallel supported paths.
4. **Contributor handoff.** Organize core/fixture/UI/reference/native/optional-real-use commands in their existing guides. Update architecture only for implemented boundaries and the owning topic guides only for changed behavior. Replace named private-photo recipes with supplied-source acceptance instructions. Remove completed scope from this plan; delete it once all maintained behavior and remaining acceptance have proper homes.

## Acceptance and verification

- Ordinary core checks require no native libraries, new desktop/test-framework dependencies, photographs or fixture downloads. Normal editor builds contain no automation entrypoints/support.
- Maintained RAW/UI/reference regression commands work from a checkout plus documented native setup, without private files, stale `.cache` inputs, personal preferences or source writes. List/filter/run/capture behavior is discoverable.
- Every retired gate has exercised replacement coverage at unchanged numerical thresholds and required independence; deletion does not substitute self-comparison or screenshots for assertions.
- Small fixtures cover success, rejection, cancellation, stale results, persistence and coordinate/metadata boundaries. Scope/defaults remain valid with current camera basis, processing model and project schema.
- Structured UI tests exercise actual controls; native checks fail on interruption/zero-visible-frame/timeout and do not run unexpectedly. Captures and visual judgments remain distinct.
- Retained tools have purpose headers, supported commands and a present consumer/question. No completed plans, execution journals, rejected galleries or speculative utility collections remain.

Verify each slice with focused tests/lints/parser/link checks and inspect its diff. For fixture migration run applicable RAW/project/GPU/export tests and external references; for UI migration run both isolated/composed checks and explicitly selected native cases. Verify ordinary build/core dependency isolation and CLI removal. Whole-suite, large-file, photographic/resource and physical/device checks are separately scoped, not reflexive acceptance requirements for every slice.

## Open gates and exact next action

- Corrected-thumbnail coordination, general composed interaction/native presentation and Adobe DNG Validator conformance remain unproved; the named authored worker tests are not acceptance for those boundaries.
- Initial-path opening in `Editor::new` still recognizes only ARW; broader picker filters do not prove CLI startup support for other RAW suffixes. Keep this entry-point gap explicit until separately corrected.
- Does egui_kittest's offscreen renderer support the production callbacks/device/format needs with a narrow adapter? Prove this before committing to the framework.
- Are any reviewed visual baselines needed, and what narrowly justified renderer/platform expectations apply? No blanket golden-image suite is selected.
- Which additional optional Analyze/real-camera diagnostics answer a concrete current acceptance question? Unclear utility defaults to deletion, not archival retention.
- Full OS accessibility/dialog automation and real photographic/performance/monitor acceptance are not selected by this restructuring.

Next: make rendering/profile/transfer reference generation explicit and self-contained before UI framework proof. Keep native/OCIO/broad-gamut coverage until equivalent replacements pass. Do not begin wholesale UI or numerical-tool replacement before their replacement gates are demonstrated.
