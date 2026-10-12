# Research and verification tools

Current helpers complement the maintained root tests and [specialist verification guide](../../build-and-probes.md). Run from the repository root; generated results go into ignored `.cache/probes`, not beside authored source or archived evidence.

## Current verification tools

- Root `tests/{camera_inversion,camera_project,camera_raw,gpu_film_response,viewer}.rs` and the explicit real-RAW flat-field editor regression maintain model-4 numerical/source/output checks; [commands and limits](../../build-and-probes.md).
- `native/raw_development_probe.cpp` checks pinned-LibRaw complete RGB16/metadata/immutable diagnostics and scoped thread policy; [build/reproduction guide](raw-development.md). This is not an alternate developer.
- Root `scripts/probe-*.sh` reproduce OCIO/GPU, gamut, profile and macOS presentation comparisons; [reference guide](../../build-and-probes.md). Native setup is described in [the README](../../../README.md).

## Manufacturer study / capture

Manufacturer extraction/fitting uses Python's standard library. With the cited PDFs already downloaded into `.cache/probes/film-response-docs/`:

```sh
python3 docs/research/tools/film-response-docs/extract-curves.py
python3 docs/research/tools/film-response-docs/fit-curves.py
```

Swift PDF/plot helpers use the macOS toolchain. [The study](../film-response-model-study.md) records URLs, drawing axes and measurement/copyright limitations. Downloaded drawings/traces are not redistributed, and these fits are not independent film measurements.

For current response-layout capture, build the editor, create `.cache/probes/film-model-images`, then:

```sh
/usr/bin/clang -fobjc-arc docs/research/tools/controls-window-id.m \
  -framework Foundation -framework CoreGraphics \
  -o .cache/probes/controls-window-id
python3 docs/research/tools/capture-response-controls.py
```

This captures only the application window during timed smoke holds. It needs local RAWs, an unlocked desktop and normal screen-capture permissions; it does not automate Lightroom/Resolve. Photographic screenshots and logs stay private/cache-local.

## Historical experiments

The `film/` crate, older capture/compositing helpers and `native/` internal diagnostics under this directory preserve research methodology. Many depend on AP0/model-2/3 APIs, private fixtures, matching OCIO object layouts or old fixed paths; they are not a supported current test suite.

[Archived inventory and reproduction notes](../history/tools-guide.txt) identify those sources and their compatibility requirements. The source remains here, but the old commands—including the removed Cargo wrapper—describe historical checkouts. Build only explicitly compatible named bins, not all research targets. Historical float/wide source snapshots are not production upload paths, and experiments are not alternate inversion/rendering backends.

## Cache and evidence boundaries

- `.cache/native-deps/` (LibRaw/OpenMP) and `.cache/deps/` (OCIO) hold reconstructible native inputs/products; setup is in the project README. Historical `.cache/rustup/` inputs may remain, but current builds use normal project toolchain selection.
- `.cache/probes/` holds regenerated camera/rendering fixtures, objects/binaries, full-size exports, plots, screenshots and duplicate logs. Samples remain read-only under ignored `samples/`.
- Downloaded copyrighted documents and copied/modified upstream experiments remain local. Owned tools, source provenance and selected decisive results belong in research/evidence; they are not automatically promoted into production.
- Obsolete `film-integration-baseline/` snapshots are not a restoration path or application dependency.

[Selected evidence](../evidence/README.md) records durable outcomes. Its archived contents are not overwritten by reproduction runs.
