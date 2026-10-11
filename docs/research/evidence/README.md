# Selected verification evidence

Compact text logs promoted from `.cache/probes` during the documentation reorganization. They retain their original contents/paths and are **captured checkpoint outcomes, not a fresh run or CI/device certification**. Reproduction writes new outputs to cache; do not overwrite archived evidence in place.

## Native-camera replacement checkpoint

- [Model-4 numerical/source/editor/output snapshot](camera-inversion.txt): isolated flat-field baseline, unchanged tolerances, independent review fixes, real train/flat-field regressions and release build. No new onscreen or calibrated-colour acceptance. [Integration/handoff plan](../../plans/camera-inversion.md).

## Response-integration checkpoint (historical AP0 models)

Environment recorded by the runbook/history: Apple M5 Pro / `Mac17,9`, 48 GiB, macOS 26.6.2, checkout-local Rust 1.99.0, LibRaw 0.22.0, OCIO 2.6.0, Metal/wgpu 30.0.1. This is not M1 Pro/16 GB, macOS-14 or physical-monitor acceptance.

The detailed domain/sample/tolerance/timing definitions live in [the precision report](../film-response-precision-gate.md#production-film-response-checkpoint). That checkpoint closed 88 all-feature tests plus strict Clippy/format/release, original-chain regression, candidate/production sweeps, real-camera both-target comparisons and editor/layout smoke. Do not infer blanket correctness from the counts.

Retained under `film-model-images/` (formerly `.cache/probes/film-model-images/`):

- [All tests](film-model-images/response-all-tests.txt), [Clippy](film-model-images/response-clippy.txt), [release build](film-model-images/response-release-build.txt).
- [Production GPU tests](film-model-images/response-gpu-tests.txt), [scroll regression](film-model-images/response-scroll-test.txt).
- [Original camera fixture](film-model-images/response-original-camera-fixture.txt), [original complete chain](film-model-images/response-original-complete-chain.txt), [actual model diagnostic](film-model-images/response-probe-model.txt).
- [Editor smoke](film-model-images/response-editor-smoke.txt), [four-hold layout smoke](film-model-images/response-controls-smoke.txt).
- [Wide candidate arithmetic sweep](film-model-images/film-wide-precision.txt), [production real-camera/rendering gate](film-model-images/film-production-check.txt).
- [Preceding RGB16 conversion test](film-model-images/rgb16-conversion-tests.txt).

## Flat-field checkpoints

- [Initial independent estimator evidence](flat-field-initial.txt).
- [Production scalar/GPU processing and real-source gates](flat-field-gpu.txt).
- [Editor/persistence/thumbnail and corrected-file integration](flat-field-editor.txt); historical headless pass and zero-visible-frame failure.
- [Later corrected onscreen and full-size TIFF/AP0 EXR acceptance](flat-field-acceptance.txt); both approved RAW pairs pass the visible-frame gate. Native-dialog/physical-input acceptance remains separate.

User-accepted closure and verification limits: [completed plan](../../plans/completed/flat-field-correction.md#verified-automated-acceptance).

## Earlier independent gates

These remain separate historical checkpoints rather than being summed into one current test count:

- [Dmin independent whole-image search](dmin-whole-independent.txt) and [Dmin GPU check](dmin-trace/run2/gpu-check.txt); conditions in [the trace](../dmin-shadow-trace.md).
- [Final P3 coloured-grid gamut gate](p3-gamut-gate-final.txt) and [ColorSync/profile reference](p3-profile-reference.txt); conditions in [the output update](../p3-output-update.md).
- [Extended-sRGB presentation reference](presentation-reference.txt): offscreen/encoding evidence, not physical monitor matching.
- [Earlier original camera/GPU chain](ocio-and-camera-gpu-ap1.txt).

## Selection and regeneration

Preserve small decisive results rather than every compiler transcript. Large photographic outputs, duplicate logs, generated binary fixtures, downloaded PDFs and plots remain local/regenerable. Original path lists in research/history may refer to logs now retained here or to cache-only outputs; this guide identifies the retained subset.

Use [current build/probe commands](../../build-and-probes.md) and [relocated research-tool commands](../tools/README.md). Manufacturer fit metrics/source URLs are preserved in the study/scripts; [source/extraction metadata](manufacturer-sources.json) retains PDF hashes, axes and sampling provenance, not curve traces or drawings. Extracted copyrighted drawing data is not promoted wholesale. No private photographs or camera sample binaries were added here.
