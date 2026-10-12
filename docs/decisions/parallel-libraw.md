# Bounded upstream parallel LibRaw development

Status: accepted by the user as the rendering-performance follow-up.

## Context

Pinned LibRaw 0.22.0 AHD development dominated initial import/photo switching. The old reentrant build did not enable upstream OpenMP; global sensor statistics also scanned every RAW even when unused. Experiments retained identical complete RGB16 values while substantially reducing development time. An installed Homebrew runtime required macOS 26, contrary to the macOS-14 target.

## Decision

Keep the same pinned LibRaw algorithm/settings, camera metadata/matrices, roll-fixed WB, RGB16 output and development identity. Enable its upstream OpenMP implementation with a checksum-pinned source-built LLVM OpenMP 22.1.8 runtime, installed only in checkout-local ignored storage. Build arm64 for macOS 14 with `@rpath/libomp.dylib`; do not use inherited system/Homebrew runtime overrides. Preserve license material with local dependency products.

Limit each native decode/process call by job priority, available CPUs and memory pressure. Scope/restore OpenMP task ICVs; idle workers sleep instead of spinning against foreground work. Current measured caps are eight foreground/two background threads, reduced under pressure, while reserving two available logical CPUs where possible. This bounds jobs rather than creating a full-roll RAW queue/cache.

Separate decoding from diagnostics: decode once and retain original RAW data; compute whole-sensor statistics only on explicit request. Regional picker/estimator diagnostics still read immutable original CFA, margins and black/white levels after development. Do not substitute developed RGB clipping flags or bypass supported-source validation.

## Consequences

Faster selected development without a custom demosaicer, approximate processing or alternate source path. Complete RGB16, metadata, pre-multipliers and full/region diagnostics matched the old build on four local sources at 1/2/4/8 threads. That finite evidence does not establish all cameras/hardware or photographic correctness; broaden comparisons when source support changes and do not silently accept differing output under the same development identity.

The checkout links a local dynamic runtime, not a shipped application bundle. Packaging/codesigning/notices and actual macOS-14 execution remain separate release work. Thread caps can change from new responsiveness measurements without changing pixel/development semantics. Generation cancellation still cannot interrupt an already executing LibRaw native call.

## Evidence

- [Performance plan/checkpoint](../plans/rendering-performance.md), [selected measurements](../research/evidence/rendering-performance.txt).
- [Reproduction/compatibility probe](../research/tools/raw-development.md), [runbook](../build-and-probes.md).
- [Established development](established-raw-development.md), [source/picker contract](../contracts/processing-model.md#camera-source-representation).
