# Native development

Native development uses checkout-local builds of pinned LibRaw/OpenMP/OCIO/DNG-SDK dependencies. This guide covers how the editor schedules native RAW development, how those builds are reused across worktrees, and their runtime limits.

## Editor scheduling

The editor's two [native owners](../src/editor_development.rs) at most serve foreground, visible-thumbnail and eager demand; each owner runs one synchronous unpack/development call at a time and keeps its own mutable RAW handles, so started work stays with its owner between phases. A pending foreground demand can start beside a running call on the free owner, while background work starts only while no foreground demand exists; the second owner sits out under elevated memory pressure and rejoins at the next scheduler event after pressure clears. Every editor development uses the established high budget, reduced by pressure and the CPU reservation; concurrent owners split that budget at each call. Compatible same-key work joins with independent consumer cancellation and immutable result ownership. Worker-local ownership is a deliberate conservative boundary, not a LibRaw mandate: concurrent mutable use breaks the bridge's lend contract, while sequential thread migration is unaudited (the pinned source uses no thread-local state); relaxing it takes a migration audit, the lend contract revalidated across calls, and DNG SDK/XMP transaction serialization reviewed. Overlap trades a measured contention cost during concurrent calls for roll throughput through serial decode while keeping mutable ownership simple: worker-local handles, no general scheduler. Standalone APIs retain their explicit priority budgets.

The editor's direct-DNG route uses that same budget inside one SDK transaction, not a second source queue. SDK warp tiles and capture-eligibility rows can execute in parallel; workers join before cleanup/publication, and other SDK stages remain serial.

## Interpreting hardware scaling

The thread budget does not parallelize every native phase. In pinned LibRaw 0.22.0, `sony_ljpeg_load_raw()` walks the lossless-JPEG tiles and rows serially through one decoder/stream; that path has no OpenMP work-sharing region. By contrast, `ahd_interpolate()` parallelizes tile stripes and allocates one 6.5 MiB scratch buffer per requested thread (`26 x LIBRAW_AHD_TILE^2`, with tile side 512). These boundaries are visible in the prepared LibRaw source at `src/decoders/decoders_libraw_dcrdefs.cpp`, `src/demosaic/ahd_demosaic.cpp` and `libraw/libraw_const.h`. The [bridge](../native/raw_bridge.cpp) supplies scoped budgets around unpack and development, but cannot make the serial decoder use additional cores. This observation applies to that decoder, not every Sony format or DNG development.

Measure decode and development separately before changing the eight-thread cap or adding owners. A measured single-source example (serial unpack, sublinear process scaling, small cap-raise headroom) is recorded with the implementation that relied on it. More development threads can shorten only the parallel portion; independent-source overlap carries roll throughput through serial decode with separate mutable owners, foreground-yield checks and a shared thread budget. Neither a native budget nor the foreground permit is a measure of CPU/GPU occupancy.

The maintained `composed_native_overlap_throughput_and_foreground_latency` diagnostic measures the production schedule at production priorities on real sources: a solo development, two overlapped background ones, and foreground demands arriving beside queued and in-flight background work. It reports wall times and per-image contention, fails unless every schedule reproduces the solo development pixel for pixel, and each run prints the pressure reading it observed. The bridge's timing lines report the thread team each call actually used — the only direct confirmation that the budget split reached the native call. Treat the timings as observations for that machine and source class, not acceptance thresholds:

```sh
ANTITONE_RAW_SAMPLES=/absolute/samples cargo test --locked --offline --release \
  --features editor --lib composed_native_overlap -- --ignored --nocapture --test-threads=1
```

When high-end hardware is unavailable, report conditional projections rather than measured speedups. A simple model separates unaccelerated decode/host/queue time from scalable processing time: `T_projected = T_unaccelerated + T_scalable / r`, where `r` is an explicitly assumed stage-speed ratio, not the ratio of advertised core counts. Matched local thread trials can constrain that assumption but do not validate another chip's memory bandwidth, scheduling, thermals, GPU feeding or mixed-load responsiveness. Keep hardware scaling acceptance open until those observations exist.

## Reusing native builds

Separate worktrees can use existing compatible dependencies without a second bootstrap. Set `LIBRAW_SOURCE` to the LibRaw source/build directory containing `lib/libraw_r.a`. Opt-in `ocio-reference` builds and native reference probes additionally accept `OCIO_PREFIX` pointing to an installed prefix containing `include/` and `lib/`; normal editor builds do not link OCIO. Use absolute paths. These are build/probe overrides; `setup-native.sh` still builds the pinned checkout-local dependencies.

The accepted OpenMP runtime must remain available at `.cache/native-deps/openmp-22.1.8-install`, for example through an ignored shared native-cache link. `LIBRAW_SOURCE` does not relocate it, and system/Homebrew runtimes are not supported substitutes. Little CMS is built statically through Cargo; `LCMS2_LIB_DIR` is not a supported override. Different native versions/settings require compatibility and numerical validation, not just a successful link.

## Repeated editor switching and busy input

[`editor_switching_observation.rs`](../native/editor_switching_observation.rs) exercises production source-card input, backend/thumbnail workers and offscreen GPU callbacks. It keeps two 8704×6144 sources (`DSC04378.ARW`, `DSC04393.ARW`; WB header `DSC04397.ARW`) under 1100 MiB CPU/16 MiB GPU optional limits, with fresh temporary thumbnails. Two rapid pending replacements precede 17 accepted physical views. Native/backend workers drain before observation ends. It does not launch a window, edit samples or persist preferences.

```sh
ANTITONE_SWITCH_SAMPLES=/absolute/path/to/samples \
  cargo test --locked --offline --release --features editor --lib \
  editor::thumbnails::switch_observation::retained_source_switching_and_busy_input \
  -- --ignored --exact --nocapture --test-threads=1
```

Use only within agreed workload bounds. The [measurement helper](../native/cache_observation.rs) rejects sources over 600 MiB of explicitly estimated source-construction storage; the diagnostic starts only at known Normal pressure with at least 2 GiB reported free memory, and its sampler latches unknown/elevated pressure or stale sampling. Native calls are noninterruptible: stop thresholds are not hard allocation caps, and sampled peaks are not exact maxima. These measurements do not grant cache admission permission; [fixed preferences](editor.md#responsiveness-and-optional-caches) govern optional ownership.

The exercise observes native development counts, Ready delivery and accepted physical-view readiness separately, including source replacement while loading. A retained native hit does not imply cached physical AP1 or original comparison output. Offscreen/single-pass measurements do not establish physical display latency or default-budget whole-roll residency.

## Runtime and acceptance limits

`otool -l .cache/native-deps/openmp-22.1.8-install/lib/libomp.dylib` should report `@rpath/libomp.dylib` and minimum OS 14.0; `otool -L` should show only system dependencies. The [local delivery workflow](build-and-delivery.md) copies OpenMP into the app, replaces checkout search paths with bundle-relative loading and verifies the extracted ZIP. Cargo-built binaries still use the local native prefix. Runtime inspection and relocated execution do not establish clean-machine/macOS-14 execution or public-release clearance.
