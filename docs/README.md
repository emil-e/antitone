# Contributing to Antitone

Start with [build and run](build-and-run.md), then choose the part of the project you want to understand or change. You do not need to read the whole documentation set first.

## Understand the system

[Architecture](architecture.md) explains how the application runs: the event loop, workflows, scopes, services and ownership. The topic guides explain the models behind the code:

| If you are wondering… | Read… |
| --- | --- |
| How does a negative become a positive? | [Processing](processing.md) |
| How does Analyze choose its starting grade? | [Roll analysis](roll-analysis.md) |
| What does the bare-light reference correct? | [Flat-field correction](flat-field.md) |
| Why do preview, TIFF and EXR differ? | [Colour and output](colour-and-output.md) |
| How are independent creative looks authored offline? | [Look catalog and authoring](../assets/looks/README.md) |
| Which pixels does a crop, pick or export represent? | [Geometry](geometry.md) |
| How can features share film and picture locations? | [Spatial metadata](spatial-metadata.md) |
| How do tools, workers and previews stay coherent? | [Editor](editor.md) |
| How does the editor run slow work without freezing the UI? | [Architecture](architecture.md) |
| What is saved, and what makes a source compatible? | [Projects](projects.md) |
| What do the waveform and vectorscope actually measure? | [Scopes](scopes.md) |

Each guide explains the idea, the important boundaries and where to find its implementation. Types, comments and tests show the current mechanics; the guides are not parallel API specifications or a reason to freeze implementation choices.

## Module map

Where each area's implementation lives — navigation, not specification. Follow the links, then use each guide's "Working on this area" for the full module list:

| Area | Start here | Responsibility |
| --- | --- | --- |
| Processing | [processing](../src/processing.rs), [photographic](../src/photographic.rs), [colour](../src/color.rs) | Scalar reference, response and positive grading |
| Adjustment layers | [scalar processing](../src/adjustment_layers.rs), [GPU grades](../src/gpu_adjustment_layers.rs), [editor routing](../src/editor_processing.rs) | Immutable snapshots, analytic masks, source-owned layers, whole-document admission |
| RAW source | [raw](../src/raw.rs), [scan](../src/scan.rs), [source preparation](../src/editor_source.rs), [native bridge](../native/raw_bridge.cpp) | LibRaw ownership, camera RGB and capture diagnostics |
| DNG backend | [adapter](../src/dng.rs), [native owner](../native/dng_bridge.cpp) | Direct SDK development and decode-free inspection in the shared source model |
| Scan correction | [flat-field](../src/flat_field.rs), [GPU map](../src/gpu_flat_field.rs) | Source-coordinate illumination gains |
| Image recipe and repairs | [accepted recipe](../src/image_recipe.rs), [source evaluator](../src/repair_source.rs), [repair model](../src/dust_repair.rs), [controller](../src/editor_dust.rs), [GPU patches](../src/gpu_repairs.rs) | Shared accepted recipe; consumer-owned repair execution |
| Roll analysis | [spatial support](../src/scan_regions.rs), [estimator](../src/roll_analysis.rs), [fitter](../src/roll_analysis_gamma.rs) | Source-owned sampling hypotheses, sample-derived starting settings |
| GPU | [processing](../src/gpu.rs), [renderer](../src/renderer.rs) | Interactive processing and reusable rendering resources |
| Rendering/output | [rendering](../src/rendering.rs), [export](../src/export.rs), [native transform](../native/rendering_transform.h) | ACES, file encoding and output safety |
| Creative looks | [asset/import](../src/looks.rs), [native generation](../src/looks_builtin.rs), [GPU execution](../src/gpu_look.rs), [selection](../src/editor_looks.rs) | Project-wide embedded immutable LUTs before selected-output ACES |
| Geometry | [geometry](../src/geometry.rs), [coverage](../src/coverage.rs), [crop](../src/crop.rs), [auto-crop](../src/auto_crop.rs) | Coordinate mappings, source-cell coverage, measured-frame proposals |
| Viewer | [working owner](../src/editor_working_view.rs), [GPU view](../src/gpu_working_view.rs), [viewer](../src/viewer.rs), [original-view owner](../src/editor_original_view.rs), [refinement](../src/refinement.rs), [viewport](../src/viewport.rs) | Physical-view grading/inspection, original comparison, refinement |
| Scopes | [measurement](../src/scopes.rs), [GPU reduction](../src/gpu_scopes.rs) | Approximate visible-image output statistics |
| Editor | [editor](../src/editor.rs) and its feature modules | Live state, controls and transactions |
| Workflows | [application](../src/application.rs), [blocking admission](../src/blocking_jobs.rs), [measurement](../src/editor_measurement.rs), [workflow modules](../src/editor_export_workflow.rs) | UI-thread async execution, borrowed UI context, off-thread services |
| Persistence | [project](../src/project.rs), [document](../src/editor_document.rs), [document owner](../src/editor_document_scope.rs) | Accepted state, history and cache lifetime, compatibility |
| Host | [host](../src/platform/host.rs), [macOS](../src/platform/macos.rs), [Windows](../src/platform/windows.rs), [full-display inspection](../src/platform/fullscreen.rs) | Event loop, surface, presentation, native menus, window coverage |
| Branding | [assets](../assets/branding/README.md), [generator](../native/branding_assets.swift), [macOS packaging](../scripts/package-macos.py) | Portable artwork and relocatable local-test application bundle |

## Make a change

Use the [development guide](development.md) for engineering standards, the normal edit/test loop and targeted subsystem checks. [Diagnostics](diagnostics.md) covers sample-dependent, numerical, export and presentation exercises; [native development](native-development.md) covers native scheduling, build reuse and runtime limits. [Build and delivery](build-and-delivery.md) explains the automated local setup/check/package workflow, native build ownership and local-package versus public-release boundaries.

The supported platforms are macOS on Apple Silicon and Windows 11 on x64. Local RAWs and generated photographic outputs are private, ignored inputs/products—not contributor prerequisites or repository fixtures to upload.

For the provisional Dust tool, read the [editor workflow](editor.md#provisional-dust-cleanup).
