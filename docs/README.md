# Contributing to Antitone

Before changing application code, read [architecture](architecture.md) and [development](development.md). Use [build and run](build-and-run.md) for setup, then choose the relevant topic guides; you do not need to read the whole documentation set.

## Understand the system

[Architecture](architecture.md) establishes the design fundamentals and essential execution boundaries. The topic guides explain the models behind the code:

| If you are wondering… | Read… |
| --- | --- |
| How does a negative become a positive? | [Processing](processing.md) |
| How does Analyze choose its starting grade? | [Roll analysis](roll-analysis.md) |
| What does the bare-light reference correct? | [Flat-field correction](flat-field.md) |
| Where do image MTF and grain enter the picture? | [Film Emulation](processing.md#film-emulation) |
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
| Processing | [processing](../src/processing.rs), [photographic](../src/photographic.rs), [colour](../src/color.rs) | Response parameters, grading controls and numerical admission |


| Adjustment layers | [controls and masks](../src/adjustment_layers.rs), [GPU grading](../src/gpu/grading.rs), [editor routing](../src/editor/processing.rs) | Immutable snapshots, analytic masks, source-owned layers, whole-document admission |

| RAW source | [raw](../src/raw.rs), [scan](../src/scan.rs), [source acquisition](../src/source/mod.rs), [native bridge](../native/raw_bridge.cpp) | LibRaw ownership and immutable camera RGB |


| DNG backend | [adapter](../src/dng.rs), [native owner](../native/dng_bridge.cpp) | Direct SDK development and decode-free inspection in the shared source model |
| Scan correction | [flat-field](../src/flat_field.rs), [GPU map](../src/gpu/flat.rs) | Source-coordinate illumination gains |
| Image recipe and repairs | [accepted recipe](../src/image_recipe.rs), [source evaluator](../src/repair_source.rs), [repair model](../src/dust_repair.rs), [controller](../src/editor/dust.rs), [GPU patches](../src/gpu/repairs.rs) | Shared accepted recipe; consumer-owned repair execution |
| Roll analysis | [spatial support](../src/scan_regions.rs), [estimator](../src/roll_analysis.rs), [fitter](../src/roll_analysis/gamma.rs) | Source-owned sampling hypotheses, sample-derived starting settings |
| GPU | [processing](../src/gpu.rs), [renderer](../src/renderer.rs) | Interactive processing and reusable rendering resources |
| Rendering/output | [rendering](../src/rendering.rs), [export](../src/export.rs), [native transform](../native/rendering_transform.h) | ACES, file encoding and output safety |
| Film Emulation | [model/settings](../src/film_emulation.rs), [stage executor](../src/gpu/pixel_pipeline.rs) | Source-scale image MTF and RGB grain before look/output |
| Creative looks | [asset/import](../src/looks.rs), [models](../src/looks/builtin.rs), [baking](../src/looks/bake.rs), [GPU execution](../src/gpu/look.rs), [selection](../src/editor/look.rs) | Project-wide embedded immutable LUTs before selected-output ACES |
| Geometry | [geometry](../src/geometry.rs), [coverage](../src/coverage.rs), [crop](../src/crop.rs), [auto-crop](../src/auto_crop.rs) | Coordinate mappings, source-cell coverage, measured-frame proposals |
| Viewer | [working owner](../src/editor/working_view.rs), [GPU view](../src/gpu/view.rs), [viewer](../src/viewer.rs), [original-view owner](../src/editor/original_view.rs), [refinement](../src/refinement.rs), [viewport](../src/viewport.rs) | Physical-view grading/inspection, original comparison, refinement |
| Scopes | [measurement](../src/scopes.rs), [GPU reduction](../src/gpu_scopes.rs) | Approximate visible-image output statistics |
| Editor | [editor](../src/editor.rs) and its feature modules | Live state, controls and transactions |
| Workflows | [application](../src/application.rs), [blocking execution](../src/background.rs), [picking](../src/editor/picking.rs), [export](../src/editor/export.rs) | Feature-owned async execution, borrowed UI context, off-thread work |
| Persistence | [project](../src/project.rs), [document](../src/editor/document.rs), [document owner](../src/editor/document_scope.rs) | Accepted state, history and cache lifetime, compatibility |

| Host | [host](../src/platform/host.rs), [macOS](../src/platform/macos.rs), [Windows](../src/platform/windows.rs), [full-display inspection](../src/platform/fullscreen.rs) | Event loop, surface, presentation, native menus, window coverage |
| Branding | [assets](../assets/branding/README.md), [generator](../native/branding_assets.swift), [macOS packaging](../scripts/package-macos.py) | Portable artwork and relocatable local-test application bundle |

## Make a change

Use the [development guide](development.md) for engineering standards, the normal edit/test loop and targeted subsystem checks. [Diagnostics](diagnostics.md) covers sample-dependent, numerical, export and presentation exercises; [native development](native-development.md) covers native scheduling, build reuse and runtime limits. [Build and delivery](build-and-delivery.md) explains the automated local setup/check/package workflow, native build ownership and local-package versus public-release boundaries.

The supported platforms are macOS on Apple Silicon and Windows 11 on x64. Local RAWs and generated photographic outputs are private, ignored inputs/products—not contributor prerequisites or repository fixtures to upload.

For the provisional Dust tool, read the [editor workflow](editor.md#provisional-dust-cleanup).
