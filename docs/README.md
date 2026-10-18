# Contributing to Antitone

Start with [build and run](../README.md), then choose the part of the project you want to understand or change. You do not need to read the whole documentation set first.

## Understand the system

[Architecture](architecture.md) maps code ownership and execution. The topic guides explain the models behind that code:

| If you are wondering… | Read… |
| --- | --- |
| How does a negative become a positive? | [Processing](processing.md) |
| How does Analyze choose its starting grade? | [Roll analysis](roll-analysis.md) |
| What does the bare-light reference correct? | [Flat-field correction](flat-field.md) |
| Why do preview, TIFF and EXR differ? | [Colour and output](colour-and-output.md) |
| How are independent creative looks authored offline? | [Look catalog and authoring](../assets/looks/README.md) |
| Which pixels does a crop, pick or export represent? | [Geometry](geometry.md) |
| How do tools, workers and previews stay coherent? | [Editor](editor.md) |
| What is saved, and what makes a source compatible? | [Projects](projects.md) |
| What do the waveform and vectorscope actually measure? | [Scopes](scopes.md) |

Each guide explains the idea, the important boundaries and where to find its implementation. Types, comments and tests show the current mechanics; the guides are not parallel API specifications or a reason to freeze implementation choices.

## Make a change

Use the [development guide](development.md) for engineering standards, the normal edit/test loop and targeted subsystem checks. [Diagnostics](diagnostics.md) covers sample-dependent, numerical, export and presentation exercises; [native development](native-development.md) covers the production RAW diagnostic.

The current platform is macOS on Apple Silicon. Local RAWs and generated photographic outputs are private, ignored inputs/products—not contributor prerequisites or repository fixtures to upload.

## Understand the choices and unfinished work

[Decisions](decisions/README.md) explain consequential tradeoffs. [PLAN.md](../PLAN.md) gives current priorities, [work plans](plans/README.md) describe substantial unfinished packages, and [feedback](feedback.md) collects unresolved observations. A proposed feature in a plan is not implemented behavior.

For the provisional Dust tool, read the [editor workflow](editor.md#provisional-dust-cleanup) and [remaining acceptance/output boundaries](plans/dust-repair.md). The [dust detection and repair research](research/dust-removal.md) and its [comparison tools](../scripts/dust-experiments/README.md) document experimental evidence rather than accepted product behavior.
