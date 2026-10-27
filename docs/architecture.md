# Architecture: how the application runs

Antitone is one Rust package with small native bridges: LibRaw/OpenMP for RAW development, the Adobe DNG SDK behind the `dng-sdk` feature, Metal on macOS and Vulkan on Windows through wgpu, and generated OCIO references behind `ocio-reference`. Cargo features add persistence, RAW development, GPU processing, rendering, export and the editor, so a contributor can work on one boundary without building the desktop stack.

This guide owns the application's structure: how work executes, where state lives, and which rules keep it sound. What features *mean* is explained by the topic guides ([README](README.md)); the [module map](README.md#module-map) lives there too.

## The event loop owns the UI thread

[`platform/host.rs`](../src/platform/host.rs) runs the winit event loop on the main thread; per-platform modules own window-system integration through [macOS](../src/platform/macos.rs) AppKit and [Windows](../src/platform/windows.rs) Win32, including native menus and presentation. AppKit termination requests use [native quit negotiation](editor.md#native-quit-negotiation). The UI owns live document and view state, and a frame paint must stay fast: no RAW development, GPU waits or file IO happens inside a paint callback.

## Workflows: slow work without a backend thread

Every long operation — opening a photo, calibration measurements, Analyze fitting, export — runs as an async task on a [`LocalExecutor`](../src/application.rs) on the UI thread, driven from winit wake events outside layout and painting. A *workflow* is feature-owned ([`editor_export_workflow.rs`](../src/editor_export_workflow.rs), [`editor_analysis_workflow.rs`](../src/editor_analysis_workflow.rs) and siblings): it captures its request's inputs when the UI action happens, receives a borrowed [`UiContext`](../src/application.rs), and awaits typed results from services that do the actual work off-thread. There is no general backend command/result loop; each feature owns its own small workflow, which keeps ownership of live UI state explicit.

Three rules keep the borrow sound:

- **Capture inputs at the UI action.** A workflow reads its source, settings and destination from the captured request, never lazily from live state that can change under it.
- **Never hold an `Editor` borrow or `egui::Ui` across an await.** Use short read/update closures around the state you touch.
- **Updates request repaint; progress does not require a frame.** A completed development or fit proceeds even while nothing is being painted.

The host destroys pending futures before retiring application, window and GPU state, and `UiContext` holds only a weak executor reference, so a workflow cannot outlive what would wake it.

## Scopes: nested, disposable UI state

Editor state is nested scopes rather than one flat struct ([`editor_document_scope.rs`](../src/editor_document_scope.rs)):

| Scope | Owns |
| --- | --- |
| [`DocumentScope`](../src/editor_document_scope.rs) | Accepted document data, history, [derived spatial metadata](spatial-metadata.md), cache lifetime and the dependent photo visit |
| [`PhotoScope`](../src/editor_photo_scope.rs) | One visit: source demand/readiness, measurement and working-source tasks, manual Crop drafts, local mask gestures, the adjustment-layer selection |
| [`GradeScope`](../src/editor_grade_scope.rs) | The effective processing identity and the purpose-bound original-reconstruction task |

Navigation creates fresh visits. Retiring one discards its transient tools without accepting geometry or changing source-owned adjustments; a rejected navigation or failed candidate preserves the current visit. A captured document is a data snapshot, not a cloned owner or history: retiring the owner closes publication even while snapshots and pixel leases stay alive. Accepted state, drafts and durable preferences are separated in [editor](editor.md#accepted-state-and-drafts).

Export instead owns an application-level batch: fixed recipes, resolved paths, correction and service handles. It acquires one original at a time without preparing viewer resources. Document caches supply optional reuse, not execution authority; closing one cannot retire the batch. Blocking workspace interaction during export is a separate UI choice.

## Services, workers and admission

Workflows never block; services run the work off the UI thread and workflows await typed results:

- **Blocking admission.** Synchronous foreground jobs share one editor-owned permit ([`blocking_jobs.rs`](../src/blocking_jobs.rs)), serializing conservative physical work across visits. A source wait occupies neither a blocking worker nor an additional permit.
- **Two native owners at most** ([`editor_development.rs`](../src/editor_development.rs)) serve foreground, visible and eager development; each runs one non-interruptible call with worker-local mutable RAW handles, background work yields to foreground demand, and the pair collapses to one owner under memory pressure. [Scheduling rationale](native-development.md#editor-scheduling) describes the tradeoff.
- **Thumbnail and embedded-preview processing** ([`editor_thumbnail_worker.rs`](../src/editor_thumbnail_worker.rs)) is a background path, not another RAW development pool: it never borrows the selected `Scan` from foreground operations, GPU processing is serial, and obsolete tickets never install images.
- **Lazy photo preparation** ([`source_store.rs`](../src/source_store.rs)) resolves Digital starting exposure during native/proxy acquisition, before processed-positive publication. Camera preview display shares decoded leases and in-flight work across same-document candidates. These candidates share immutable metering results while isolating pixel publication. AP1 production folds the completed gain into its source transform; consumers use normalized pixels and source identity rather than handling exposure readiness.
- **The CPU store** is a byte-bounded, oldest-unprotected LRU for native data, proxies and decoded camera references: active leases survive eviction, selected/visible proxies are protected separately, and native cache ownership is always optional. Retention settings and pressure behavior: [editor](editor.md#retention-settings-and-pressure).

Presentation follows the same separation: [`editor_activity.rs`](../src/editor_activity.rs) owns one foreground task-activity lifecycle (typed requests, progress, cancellation state), and [`editor_ui_dialogs.rs`](../src/editor_ui_dialogs.rs) owns decision framing, button focus and dismissal — controllers keep the payloads and dispatch.

## Results are proposals

Every awaited result carries request IDs, generations and input snapshots. Validate a complete result against current state before promoting anything: stale or superseded results are discarded, and successful candidates promote atomically — the UI installs Analyze's support and grade together, or not at all.

### Cooperative cancellation

Use the shared [cancellation signal](../src/cancellation.rs) for new cancellable workflows. Each operation gets a fresh, one-way signal with polling and async wakeup; capture its cancel-on-drop guard in the continuation so retirement before the first poll also signals stop. Analysis, region preparation and export use this pattern.

User Cancel keeps the continuation and cancelling feedback until acknowledgement. Owner retirement drops continuation rights while running work retains its inputs and physical admission through cleanup. Queued work can use `BlockingJobs::run_admitted_cancellable`; running work chooses its own safe checkpoints. Return cancellation distinctly from failure, never by parsing diagnostic text, and preserve already-committed side effects as successes. [Development checks](development.md#cancellation-checks) exercise these boundaries.

## Native and GPU ownership

- **Native development is consumed, not shared.** A mutable `RawFile` stays worker-local; a successful development becomes private frozen native ownership. A [`Scan`](../src/scan.rs) leases an immutable `DevelopedSource` — native RGB16, Sony-corrected f32 or direct-SDK DNG f32 samples with original CFA and metadata — keeping its own illumination-correction context. Interpretation snapshots can carry different derived exposure defaults while sharing frozen sample storage; rebinding a restored default leaves existing snapshots intact. Final destruction follows the last storage lease, including synchronized transfer to another thread.
- **One wgpu device/queue and one export engine** ([`gpu_export_area.rs`](../src/gpu_export_area.rs)) serve TIFF and EXR, view rendering and repairs; mutable processing state stays image/job-owned. The shared submission gate ([`gpu_submission.rs`](../src/gpu_submission.rs)) excludes new submissions while wgpu drains GPU work during a resize.
- **Grade routing reuses immutable constants by complete processing content** — roll WB plus ordered adjustment layers — so view state and metadata never recreate grades. [Output](colour-and-output.md#tiff-and-exr-serve-different-purposes) owns reconstruction and lifetime detail.
- **Image recipes capture accepted intent** ([processing](processing.md#one-accepted-image-recipe)): viewer, reference, scopes and export share accepted repairs; [navigation thumbnails](editor.md#positive-thumbnail-reuse) prioritize inexpensive proxy rendering.

### Pixel stages and spatial support

[`PixelPipeline`](../src/pixel_pipeline.rs) lowers accepted rendering settings into ordered stages after photographic grading. Each stage consumes pixels, generic raster coordinates/coverage and its own parameters; photographic grading and repair evaluation remain independent of those stages. The [GPU executor](../src/gpu_pixel_pipeline.rs) owns dispatch order, scratch buffers, reusable fields and valid-output extraction. The completed image MTF precedes grain, leaving the future halation position between them.

Each stage declares its input neighbourhood at the raster's pitch. [`Support`](../src/processing_support.rs) composes those margins; view, export and refinement adapters query them and own their acquisition/tiling loops. Native export/refinement expand source regions, physical views acquire padded rasters, and repair selection consumes the same generic support. Acquisition clips to actual source bounds; their outermost cells extend constantly for filtering. Intermediate viewport, composition and tile edges retain the declared neighbours. Extract or reconstruct the requested output after processing. The deferred [processing plan](../PROCESSING_PLAN.md) describes unifying planning and its execution contract across these adapters.

## UI test boundary

The opt-in `ui-tests` feature keeps egui_kittest cases in the Editor's private [test module](../src/editor_kittest.rs). The harness invokes production `host::App::ui` and drives the live application between frames, independently of painting, on an RGBA16Float renderer that shares the real device/queue and submission gate.

The separate [native-host executable](../tests/native_host.rs) runs the production host and launches a window only with explicit `--run`. [Development checks](development.md#explicit-native-host-checks) distinguish that composition coverage from physical input and appearance acceptance.

## Finding the right explanation

[README](README.md) maps questions to topic guides and files. [Development](development.md) maps changes to checks; [build and delivery](build-and-delivery.md) maps the local setup/check/package workflow and its limits.
