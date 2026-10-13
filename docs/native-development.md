# Read-only RAW development diagnostic

[`native/raw_development_probe.cpp`](../native/raw_development_probe.cpp) calls the production bridge/settings with one roll-fixed WB reference. It reports open/unpack/process timings and hashes every developed RGB16 code, ignoring the fourth slot. It emits no photographic output and leaves RAWs untouched. [Rationale](decisions/established-raw-development.md).

## Build and run

Run from the repository root after [native setup](../README.md#build-and-run). The pinned LLVM OpenMP 22.1.8 and LibRaw 0.22.0 builds use private `.cache/native-deps`, arm64/macOS-14 targets and checkout-local runtime paths.

```sh
/usr/bin/clang++ -std=c++17 -O3 -Wall -Wextra -Werror \
  -Wno-deprecated-declarations -mmacosx-version-min=14.0 -Xclang -fopenmp \
  -Inative -I.cache/native-deps/LibRaw-0.22.0-openmp \
  -I.cache/native-deps/openmp-22.1.8-install/include \
  native/raw_development_probe.cpp native/raw_bridge.cpp \
  .cache/native-deps/LibRaw-0.22.0-openmp/lib/libraw_r.a \
  -L.cache/native-deps/openmp-22.1.8-install/lib \
  -Wl,-rpath,"$PWD/.cache/native-deps/openmp-22.1.8-install/lib" \
  -lomp -lz -o .cache/native-deps/probe
for threads in 1 2 4 8; do
  .cache/native-deps/probe samples/DSC04393.ARW samples/DSC04397.ARW \
    "$threads" --decode-only
done
```

`--decode-only` excludes whole-frame statistics from unpack timing, but still uses validated production decode. Regional/full statistics are requested afterward for equality checks. Default mode includes explicit full statistics in unpack timing. The tool asserts scoped OpenMP thread/dynamic/blocktime settings are restored; invalid budgets reject. Hashing/equality reads are outside development timing.

Compare version, dimensions, maxima/warnings, RGB/metadata/pre-multipliers and full/region sensor hashes across thread budgets with the same WB reference. Regional diagnostics must survive development unchanged. Thread policy must not alter development/cache identity. Complete-source comparisons do not establish arbitrary-camera support, photographic correctness or physical responsiveness; timings vary with load/thermals.

## Reusing native builds

Separate worktrees can use existing compatible dependencies without a second bootstrap. Set `LIBRAW_SOURCE` to the LibRaw source/build directory containing `lib/libraw_r.a`, and `OCIO_PREFIX` to an installed prefix containing `include/` and `lib/`. Use absolute paths. These are build/probe overrides; `setup-native.sh` still builds the pinned checkout-local dependencies.

The accepted OpenMP runtime must remain available at `.cache/native-deps/openmp-22.1.8-install`, for example through an ignored shared native-cache link. `LIBRAW_SOURCE` does not relocate it, and system/Homebrew runtimes are not supported substitutes. Cargo/rustup download caches are shared normally; each worktree has its own build products. Use absolute sample paths or read-only sample links.

Little CMS is built statically through Cargo; `LCMS2_LIB_DIR` is not a supported override. Different native versions/settings require compatibility and numerical validation, not just a successful link.

## Runtime and acceptance limits

`otool -l .cache/native-deps/openmp-22.1.8-install/lib/libomp.dylib` should report `@rpath/libomp.dylib` and minimum OS 14.0; `otool -L` should show only system dependencies. The [local macOS bundle](../README.md#build-and-run) keeps those runtime paths. Runtime identity, deployment-target inspection and local license files do not establish distributable packaging or actual macOS-14 execution. Bundling/codesigning/notices, broader cameras/devices and physical memory-pressure/input checks remain [release work](plans/completion.md#export-and-release-work).
