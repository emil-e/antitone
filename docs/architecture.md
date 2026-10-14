# Architecture: where the work belongs

Antitone is one Rust package with small native bridges. The numerical core builds without the desktop stack. Cargo features add persistence, RAW development, GPU processing, rendering, export and the editor, so a contributor can work on one boundary without building everything.

For the photographic model, start with [processing](processing.md). This page is a map of the implementation and its ownership, not a second specification of every feature.

## Module map

| Area | Start here | Responsibility |
| --- | --- | --- |
| Processing | [processing](../src/processing.rs), [photographic](../src/photographic.rs), [colour](../src/color.rs) | Scalar reference, response and positive grading |
| RAW source | [raw](../src/raw.rs), [scan](../src/scan.rs), [native bridge](../native/raw_bridge.cpp) | LibRaw ownership, camera RGB and original capture diagnostics |
| Scan correction | [flat-field](../src/flat_field.rs), [GPU map](../src/gpu_flat_field.rs) | Source-coordinate illumination gains |
| Roll analysis | [estimator](../src/roll_analysis.rs), [fitter](../src/roll_analysis_gamma.rs) | Sample-derived starting settings, separate from pixel processing |
| GPU | [processing](../src/gpu.rs), [renderer](../src/renderer.rs) | Interactive processing and reusable rendering resources |
| Rendering/output | [rendering](../src/rendering.rs), [export](../src/export.rs), [native transform](../native/rendering_transform.h) | ACES, file encoding and output safety |
| Geometry | [geometry](../src/geometry.rs), [coverage](../src/coverage.rs), [crop](../src/crop.rs) | Coordinate mappings and actual source-cell coverage |
| Viewer | [viewer](../src/viewer.rs), [refinement](../src/refinement.rs), [viewport](../src/viewport.rs) | Approximate display, settled reconstruction and navigation |
| Scopes | [measurement](../src/scopes.rs), [GPU reduction](../src/gpu_scopes.rs) | Approximate whole-frame output statistics |
| Editor | [editor](../src/editor.rs) and its feature modules | Live state, controls, transactions and worker dispatch |
| Persistence | [project](../src/project.rs), [document](../src/editor_document.rs) | Accepted state and compatibility |
| Host | [host](../src/platform/host.rs), [macOS](../src/platform/macos.rs) | Event loop, surface, managed SDR presentation and native application icon |
| Branding | [assets](../assets/branding/README.md), [generator](../native/branding_assets.swift), [macOS packaging](../scripts/package-macos.py) | Portable artwork and checkout-local application bundle |

## Ownership and execution

The UI owns live document and view state. A backend thread serializes source loading, document preparation, picks, analysis, export and refinement. It retains the active `Scan`, whose `RawFile` owns LibRaw development and immutable original capture diagnostics.

Thumbnail and embedded-camera-preview workers are separate bounded background paths. They must not borrow the selected Scan away from foreground operations. Analysis opens/releases temporary sources while retaining only bounded statistics, rather than keeping a roll of full developments in memory.

UI, backend and image workers share a wgpu device/queue and reusable immutable GPU infrastructure. Mutable processing/storage and signatures remain image/job-owned. CPU processing supplies the numerical reference, measurements and export; it is not an interactive fallback when a GPU domain is unsupported.

Surface reconfiguration must not overlap submission on the shared queue. The host and background submitters use the same [submission gate](../src/platform/submission.rs), excluding new submissions while wgpu drains GPU work for a resize. CPU tile preparation, command encoding, staged uploads and explicit completion polls remain outside that gate.

Worker results are proposals. Request IDs, generations and source/settings snapshots determine whether they still belong to the current operation. Prepare and validate before replacing accepted state; discard stale results. Cancellation is cooperative, not a promise to interrupt LibRaw or already-submitted GPU work immediately.

## Editor workspace

[Shared presentation](editor.md#build-ui-from-shared-presentation) lays out supplied state and returns responses. Photographic adapters own value semantics; feature controllers own state, availability and dispatch. This keeps a presentation change from acquiring processing or persistence responsibility. [Decision dialogs](../src/editor_ui_dialogs.rs) own framing, button focus and dismissal; document, export and analysis controllers retain the decision payloads and transaction dispatch. [Feedback presentation](../src/editor_ui_feedback.rs) shares that shell and supplies severity/details; [operation feedback](../src/editor_feedback.rs) owns failure copy, acknowledgement and resolution without acquiring worker or persistence responsibility.

Accepted photographic state lives in [projects](projects.md). Tool drafts and viewport state are transient; durable UI preferences are stored independently. [Editor](editor.md) explains preview/cache distinctions and the input/transaction model.

## UI test boundary

The opt-in `ui-tests` feature keeps egui_kittest cases in the Editor’s private child test module, [editor_kittest](../src/editor_kittest.rs). The harness calls the production `host::App::ui` composition boundary; it does not replace the application host or expose editor state publicly. A test-only RGBA16Float renderer shares the real device/queue and submission gate, and executes production paint callbacks. [Framework UI regressions](development.md#framework-ui-regressions) are offscreen. The separate [native-host test executable](../tests/native_host.rs) uses the production host on the main thread and launches a window only with explicit `--run`; it checks focus/size forwarding, per-size surface paint and completion/close failures without exposing Editor state. Pointer/keyboard host forwarding and remaining application-smoke migration are unfinished; [commands/limits](development.md#explicit-native-host-checks).

## Finding the right explanation

- Pixels and grades: [processing](processing.md), [roll analysis](roll-analysis.md), [flat-field correction](flat-field.md).
- Image coverage: [geometry](geometry.md).
- Display and files: [colour/output](colour-and-output.md), [scopes](scopes.md).
- Application behavior: [editor](editor.md), [projects](projects.md).

[Development](development.md) maps changes to checks. [Decisions](decisions/README.md) explain selected tradeoffs, and [the roadmap](../PLAN.md) owns unfinished work and priorities.
