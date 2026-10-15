# Read-only RAW development diagnostic

Use [`native/raw_development_probe.cpp`](../native/raw_development_probe.cpp) when investigating production LibRaw/OpenMP thread policy or sensor-diagnostic immutability, not as a prerequisite for ordinary changes. It calls the production bridge/settings with one roll-fixed WB reference, reports open/unpack/process timings and hashes every developed RGB16 code, ignoring the fourth slot. It emits no photographic output and leaves RAWs untouched. [Rationale](decisions/established-raw-development.md).

The source must contain the diagnostic sensor rectangle `(4000,2600,128,128)`; the WB reference must supply valid explicit gains for the production bridge. The example paths are supplied read-only inputs, not repository fixtures or files installed by setup. Cross-budget digests compare repeated production development, not an independent demosaicing/calibration oracle.

## Editor scheduling

The editor's [single native priority worker](../src/editor_development.rs) serves foreground, visible-thumbnail and eager demand between synchronous unpack/development phases. Every editor development uses the established high budget, reduced by pressure and the CPU reservation. Compatible same-key work joins with independent consumer cancellation and immutable result ownership. An unrelated cold selection cannot interrupt a running native call; paused mutable RAW handles remain worker-local and are outside optional retention limits.

This trades some unrelated-cold-selection latency for simpler ownership and faster roll completion. Bounded comparisons observed about half a second additional cold-selection wait and about 20% shorter 36-source proper CPU/disk completion, with similar sampled memory peaks. These workload-specific snapshots do not establish a universal optimum or hardware minimum. Standalone APIs retain their explicit priority budgets.

## Build and run

Run from the repository root after [native setup](../README.md#build-and-run). The pinned LLVM OpenMP 22.1.8 and LibRaw 0.22.0 builds use private `.cache/native-deps`, arm64/macOS-14 targets and checkout-local runtime paths.

```sh
mkdir -p .cache/probes
/usr/bin/clang++ -std=c++17 -O3 -Wall -Wextra -Werror \
  -Wno-deprecated-declarations -mmacosx-version-min=14.0 -Xclang -fopenmp \
  -Inative -I.cache/native-deps/LibRaw-0.22.0-openmp \
  -I.cache/native-deps/openmp-22.1.8-install/include \
  native/raw_development_probe.cpp native/raw_bridge.cpp \
  .cache/native-deps/LibRaw-0.22.0-openmp/lib/libraw_r.a \
  -L.cache/native-deps/openmp-22.1.8-install/lib \
  -Wl,-rpath,"$PWD/.cache/native-deps/openmp-22.1.8-install/lib" \
  -lomp -lz -o .cache/probes/raw-development-probe
for threads in 1 2 4 8; do
  .cache/probes/raw-development-probe /absolute/source.ARW /absolute/roll-wb.ARW \
    "$threads" --decode-only
done
```

`--decode-only` excludes whole-frame statistics from unpack timing, but still uses validated production decode. Regional/full statistics are requested afterward for equality checks. Default mode includes explicit full statistics in unpack timing. The tool asserts scoped OpenMP thread/dynamic/blocktime settings are restored; invalid budgets reject. Hashing/equality reads are outside development timing.

A nonzero exit reports bridge, immutable-diagnostic or OpenMP-restoration failure. A successful exit does not automatically compare separate runs: inspect version, dimensions, maxima/warnings and RGB/metadata/pre-multipliers/full/region sensor hashes across thread budgets with the same WB reference. Regional diagnostics must survive development unchanged. Thread policy must not alter development/cache identity. Complete-source comparisons do not establish arbitrary-camera support, photographic correctness or physical responsiveness; timings vary with load/thermals.

## Reusing native builds

Separate worktrees can use existing compatible dependencies without a second bootstrap. Set `LIBRAW_SOURCE` to the LibRaw source/build directory containing `lib/libraw_r.a`, and `OCIO_PREFIX` to an installed prefix containing `include/` and `lib/`. Use absolute paths. These are build/probe overrides; `setup-native.sh` still builds the pinned checkout-local dependencies.

The accepted OpenMP runtime must remain available at `.cache/native-deps/openmp-22.1.8-install`, for example through an ignored shared native-cache link. `LIBRAW_SOURCE` does not relocate it, and system/Homebrew runtimes are not supported substitutes. Cargo/rustup download caches are shared normally; each worktree has its own build products. Use absolute sample paths or read-only sample links.

Little CMS is built statically through Cargo; `LCMS2_LIB_DIR` is not a supported override. Different native versions/settings require compatibility and numerical validation, not just a successful link.

## Repeated editor switching and busy input

[`editor_switching_observation.rs`](../native/editor_switching_observation.rs) exercises production source-card input, backend/thumbnail workers and offscreen GPU callbacks. It keeps two 8704×6144 sources (`DSC04378.ARW`, `DSC04393.ARW`; WB header `DSC04397.ARW`) under 1100 MiB CPU/16 MiB GPU optional limits, with fresh temporary thumbnails. Two rapid pending replacements precede 17 accepted physical views. Native/backend workers drain before observation ends. It does not launch a window, edit samples or persist preferences.

```sh
ANTITONE_SWITCH_SAMPLES=/absolute/path/to/samples \
  cargo test --locked --offline --release --features editor --lib \
  editor::thumbnails::switch_observation::retained_source_switching_and_busy_input \
  -- --ignored --exact --nocapture --test-threads=1
```

Use only within agreed workload bounds. The [measurement helper](../native/cache_observation.rs) rejects sources over 600 MiB logical RGB16/CFA storage and starts only at known Normal pressure with at least 2 GiB reported non-speculative free memory. Its sampler latches unknown/elevated pressure, changed reported swap counters, stale sampling or 2 GiB footprint growth. Native calls are noninterruptible: stop thresholds are not hard allocation caps. Cached Mach counters cannot certify quiet swapping, and sampled peaks are not exact maxima. These measurements do not grant cache admission permission; [fixed preferences](editor.md#responsiveness-and-optional-caches) govern optional ownership.

The exercise observes native development counts, Ready delivery and accepted physical-view readiness separately, including source replacement while loading. A retained native hit does not imply cached physical AP1 or original comparison output. It uses the production working-view lifecycle and does not wait for hidden original refinement. Offscreen/single-pass measurements do not establish physical display latency, default-budget whole-roll residency or broader pressure/hardware acceptance.

## Runtime and acceptance limits

`otool -l .cache/native-deps/openmp-22.1.8-install/lib/libomp.dylib` should report `@rpath/libomp.dylib` and minimum OS 14.0; `otool -L` should show only system dependencies. The [local macOS bundle](../README.md#build-and-run) keeps those runtime paths. Runtime identity, deployment-target inspection and local license files do not establish distributable packaging or actual macOS-14 execution. Bundling/codesigning/notices, broader cameras/devices and physical memory-pressure/input checks remain [release work](plans/completion.md#export-and-release-work).
