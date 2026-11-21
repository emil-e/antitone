# Native development

Native development uses checkout-local builds of pinned LibRaw/RawSpeed/OpenMP/OCIO/DNG-SDK dependencies. This guide covers how the editor schedules native RAW development, how those builds are reused across worktrees, and their runtime limits.

## Editor scheduling


The editor's two [native owners](../src/source/loader.rs) at most serve foreground, visible-thumbnail and eager demand; each owner runs one synchronous unpack/development call at a time and keeps its own mutable RAW handles, so started work stays with its owner between phases. A pending foreground demand can start beside a running call on the free owner, while background work starts only while no foreground demand exists. Every editor development uses the foreground thread cap and CPU reservation; concurrent owners split that budget at each call. Optional retention and prefetch follow [cache pressure policy](editor.md#retention-settings-and-pressure), independently of native concurrency. The async source loader owns these workers and joins matching captured source keys across document caches. Priority travels with each acquisition through the cache; cancellation and immutable result ownership remain independent per consumer.




Worker-local ownership is a deliberate conservative boundary, not a LibRaw mandate: concurrent mutable use breaks the bridge's lend contract, while sequential thread migration is unaudited (the pinned source uses no thread-local state); relaxing it takes a migration audit, the lend contract revalidated across calls, and DNG SDK/XMP transaction serialization reviewed. Overlap trades contention during concurrent calls for roll throughput while keeping mutable ownership simple: worker-local handles, no general scheduler. Standalone APIs retain their explicit priority budgets.


The editor's direct-DNG route uses that same budget inside one SDK transaction, not a second source queue. SDK warp tiles can execute in parallel; workers join before cleanup/publication, and other SDK stages remain serial. The [bridge](../native/dng_bridge.cpp) serializes SDK transactions process-wide, so an SDK development step holds native execution exclusively instead of taking a split team: a second owner cannot overlap it, and a waiting SDK step cannot halve the team of the call that is actually running. Overlap therefore carries roll throughput through LibRaw decode, and SDK work keeps the throughput a single owner gives it. Header inspection during open is a serialized single-threaded transaction, so it has no team to divide.

## RAW unpacking

The [RawSpeed adapter](../native/rawspeed/adapter.cpp) supplies sensor samples through LibRaw's RawSpeed3 handoff; LibRaw retains metadata interpretation and development. Eligibility follows codec support, not camera make. LibRaw's enabled families and additional modern codecs in the [bridge](../native/raw_bridge.cpp) attempt RawSpeed under its format/camera support checks. Unsupported files, decoder errors and incompatible rasters retain LibRaw's original unpacker. Direct SDK DNG routing remains separate.

Before publishing pixels, the adapter checks the uncropped integer raster's dimensions, channel count, CFA storage and pitch against LibRaw. It disables RawSpeed crop, bad-pixel interpolation, DNG opcodes, Fuji rotation and PhaseOne corrections. Curve-bearing codecs use LibRaw's lookup without RawSpeed dithering. Each native owner retains its own decoded image, not a decoder borrowing the temporary file buffer; retained-source accounting includes that pixel backing. Only the embedded, immutable camera database is shared.

Normal native setup runs authored sample/curve/ownership checks and actual LibRaw adoption/rejection with exact sensor and DHT parity at one and four threads. Debug `RAW unpack` events identify the selected backend and provider status (`-2`: no attempt, `0`: accepted decode, `5`: unsupported/layout rejection, other values: provider failure). Pixel-producer provenance includes the RawSpeed/adapter revision.

## Bayer GPU development

The editor supplies [DHT](../native/dht/README.md) with the host's device and guarded queue. LibRaw lends a prepared Bayer RGBG image only during synchronous development; the callback is cleared before return, including exceptional exits. Successful GPU work replaces the image after completed readback, then existing native highlights and Sony reconstruction continue. Unsupported layouts and unavailable acceleration retain LibRaw development. Standalone native APIs without a supplied accelerator remain CPU-only; SDK DNG and non-Bayer paths are unchanged.

One shared loader serializes GPU demosaicing. Its current working allowance is the smaller of one quarter of physical RAM and 3 GiB, including buffers, readback and input-upload staging. Device limits, warning/critical memory pressure or GPU failures select CPU DHT without publishing partial output. Pipelines survive calls, but scratch allocations do not remain as an idle multi-gigabyte cache. This trades allocation cost for bounded, call-owned working storage independently of optional image retention. `gpu_bayer_demosaic` debug events identify successful acceleration; rejection events explain CPU fallback.

## Interpreting hardware scaling

The CPU thread budget does not parallelize every native phase. In the LibRaw 0.22.3 fallback, `sony_ljpeg_load_raw()` walks lossless-JPEG tiles and rows serially. Bayer DHT parallelizes independent reconstruction rows; its hot-pixel correction and final direction refinements update neighbours in place and execute in scan order. Its padded f32 RGB and direction-byte workspace costs roughly 13 bytes per source pixel, independent of team size, and is released before source publication. Concurrent owners each need that workspace, outside optional retention limits. The [bridge](../native/raw_bridge.cpp) scopes the same thread budget around RawSpeed unpack and LibRaw development. Both use one pinned LLVM OpenMP runtime; Windows RAW components compile with Visual Studio's Clang tools. Other source formats and the SDK have different costs.

Measure unpack and development separately before changing the eight-thread cap or adding owners. Faster unpacking leaves the CPU fallback's serial passes intact; GPU development has its own submission, transfer and working-storage costs. More development threads can shorten only the parallel portion; independent-source overlap uses separate mutable owners, foreground-yield checks and a shared thread budget. A native team budget describes participating threads, not CPU/GPU occupancy.

The maintained CPU-only `composed_native_overlap_throughput_and_foreground_latency` diagnostic measures the native schedule at production priorities on real sources: a solo development, two overlapped background ones, and foreground demands arriving beside queued and in-flight background work. It reports wall times and per-image contention, fails unless every schedule reproduces the solo development pixel for pixel, and each run prints the pressure reading it observed. The `raw_development` events in a [debug session log](diagnostics.md#collecting-a-session-log) report the thread team passed to each successful LibRaw call, confirming that the budget split reached the native call. Treat the timings as observations for that machine and source class, not acceptance thresholds:

```sh
ANTITONE_RAW_SAMPLES=/absolute/samples ANTITONE_LOG=debug \
ANTITONE_LOG_DIR="$PWD/.cache/logs" cargo test --locked --offline --release \
  --features editor --lib composed_native_overlap -- --ignored --nocapture
```

When high-end hardware is unavailable, report conditional projections rather than measured speedups. A simple model separates unaccelerated decode/host/queue time from scalable processing time: `T_projected = T_unaccelerated + T_scalable / r`, where `r` is an explicitly assumed stage-speed ratio, not the ratio of advertised core counts. Matched local thread trials can constrain that assumption but do not validate another chip's memory bandwidth, scheduling, thermals, GPU feeding or mixed-load responsiveness. Keep hardware scaling acceptance open until those observations exist.

## Reusing native builds

Separate worktrees can use existing compatible dependencies without a second bootstrap. Set `LIBRAW_SOURCE` to the LibRaw source/build directory containing its static library: `lib/libraw_r.a` on macOS or `lib/libraw_static.lib` on Windows. Opt-in `ocio-reference` builds and native reference probes additionally accept `OCIO_PREFIX` pointing to an installed prefix containing `include/` and `lib/`; normal editor builds do not link OCIO. Use absolute paths. These are build/probe overrides; `setup-native.sh` still builds the pinned checkout-local dependencies.

Normal setup applies the same [pinned DHT correction and regression checks](../native/libraw/README.md) on both platforms, using Git and verifying the complete source file against [known hashes](../native/libraw/dht-hashes.txt). Antitone owns the [RAW pixel-producer revision](../src/raw.rs) used for cache identity and provenance. LibRaw build directories include the correction hash and RawSpeed-enabled variant. RawSpeed/pugixml and LLVM OpenMP have separate [source](../native/rawspeed/pins.txt) and [runtime](../native/openmp/pins.txt) pins. Re-run native setup after pin or adapter changes, and reuse only matching builds.

RawSpeed's installed headers/libraries and OpenMP remain at their checkout-local prefixes, even with `LIBRAW_SOURCE`. Matching ignored native-cache links can share these dependencies. macOS uses `openmp-<version>-install`; Windows uses `openmp-<version>-windows-x64-install`, with Cargo staging its DLL beside build/test executables. Do not substitute a system/Homebrew or Visual Studio OpenMP runtime: the RAW components need the same LLVM runtime for scoped thread budgets. Little CMS is built statically through Cargo; `LCMS2_LIB_DIR` is not a supported override. Different native versions/settings require compatibility and numerical validation, not just a successful link.

## Repeated editor switching and busy input

[`editor_switching_observation.rs`](../native/editor_switching_observation.rs) exercises production source-card input, backend/thumbnail workers and offscreen GPU callbacks. It keeps two 8704×6144 sources (`DSC04378.ARW`, `DSC04393.ARW`) under 1100 MiB CPU/16 MiB GPU optional limits, with fresh temporary thumbnails. Two rapid pending replacements precede 17 accepted physical views. Native/backend workers drain before observation ends. It does not launch a window, edit samples or persist preferences.

```sh
ANTITONE_SWITCH_SAMPLES=/absolute/path/to/samples \
  cargo test --locked --offline --release --features editor --lib \
  editor::thumbnails::switch_observation::retained_source_switching_and_busy_input \
  -- --ignored --exact --nocapture
```

The [measurement helper](../native/cache_observation.rs) samples process footprint, resident memory and native GPU allocations. Native calls are noninterruptible, and sampled peaks are not exact maxima. Production cache admission uses [fixed preferences](editor.md#responsiveness-and-optional-caches).

The exercise observes native development counts, Ready delivery and accepted physical-view readiness separately, including source replacement while loading. A retained native hit does not imply cached physical AP1 or original comparison output. Offscreen/single-pass measurements do not establish physical display latency or default-budget whole-roll residency.

## Runtime and acceptance limits

`otool -l .cache/native-deps/openmp-22.1.8-install/lib/libomp.dylib` should report `@rpath/libomp.dylib` and minimum OS 14.0; `otool -L` should show only system dependencies. The [local delivery workflow](build-and-delivery.md) copies OpenMP into the app, replaces checkout search paths with bundle-relative loading and verifies the extracted ZIP. Cargo-built binaries still use the local native prefix. Runtime inspection and relocated execution do not establish clean-machine/macOS-14 execution or public-release clearance.
