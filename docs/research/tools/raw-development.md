# Read-only RAW development/parallelism probe

Current model-4 verification tool, not an alternate developer. Owned source: [`native/raw_development_probe.cpp`](../../../native/raw_development_probe.cpp). It calls production bridge/settings with one roll-fixed WB reference, reports open/unpack/process timings and hashes **every developed RGB16 code** (fourth ignored). It emits no photographic output and leaves RAWs untouched. [Native decision](../../decisions/parallel-libraw.md); [selected results/limits](../evidence/rendering-performance.txt).

## Source-built production runtime

From repository/worktree root: `./scripts/build-libraw.sh` builds checksum-pinned LLVM OpenMP 22.1.8 and LibRaw 0.22.0 in private `.cache/native-deps`, targeting arm64/macOS 14. No global installs/Homebrew runtime input. Verify `otool -l .cache/native-deps/openmp-22.1.8-install/lib/libomp.dylib`: `@rpath/libomp.dylib`, minimum 14.0; `otool -L` has only system runtime dependency. Local linkage is development setup, not a distributable bundle.

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

Current `--decode-only` means time decoding without explicit whole-frame statistics: it still calls the production validated decode function. Regional/full statistics are requested afterward for equality checks. Default mode includes explicit full statistics in unpack timing. Thread arguments are scoped per call; the tool asserts prior OpenMP thread/dynamic/blocktime ICVs are restored. Invalid budgets reject. Hashing/equality reads are outside reported development time.

## Pre-change baseline comparison

The old non-OpenMP LibRaw archive in `.cache/deps/LibRaw-0.22.0` was left untouched in the performance worktree. Preserve a reproducible snapshot of **committed** baseline bridge/header `5efb302` under ignored storage, never other worktrees' uncommitted code:

```sh
mkdir -p .cache/native-deps/baseline
git show 5efb302:native/raw_bridge.cpp > .cache/native-deps/baseline/raw_bridge.cpp
git show 5efb302:native/raw_bridge.h > .cache/native-deps/baseline/raw_bridge.h
/usr/bin/clang++ -std=c++17 -O3 -Wall -Wextra -Werror \
  -Wno-deprecated-declarations -DANTITONE_BASELINE_BRIDGE \
  -I.cache/native-deps/baseline -I.cache/deps/LibRaw-0.22.0 \
  native/raw_development_probe.cpp .cache/native-deps/baseline/raw_bridge.cpp \
  .cache/deps/LibRaw-0.22.0/lib/libraw_r.a -lz -o .cache/native-deps/baseline/probe
.cache/native-deps/baseline/probe samples/DSC04393.ARW samples/DSC04397.ARW
```

On a fresh checkout, create a separate serial baseline tree from the same checksum-pinned LibRaw archive and original `Makefile.devel` flags; do not overwrite the production OpenMP tree to compare builds.

Repeat for all four local sources with the same WB reference. Compare `version`, dimensions, maxima/warnings, RGB/metadata/pre-mul and full/region sensor SHA256 fields exactly—not only thumbnails. Regional statistics are checked before/after development; lazy full statistics must equal baseline. Complete-source comparisons do not establish arbitrary-camera support, physical responsiveness or photographic calibration. One-shot timings vary with load/thermals.

## Historical experiment / acceptance limits

The initial trial used the already installed LLVM 22.1.8 runtime in a private LibRaw clone. Its minimum OS was 26, so that runtime was not adopted. Passing the entire LLVM resource directory with `-I` shadowed Apple's SDK headers; `-idirafter` fixed that experiment. Production now installs only standalone runtime headers locally and source-builds for 14.

The adopted source-built runtime matched baseline complete RGB16, camera metadata/pre-multipliers, dimensions/maxima/warnings and immutable full/region diagnostics on four sources at 1/2/4/8 threads. Pure CPU-budget and real-RAW lazy-diagnostic regressions plus unchanged GPU numerical gates accompany it. Physical OS pressure, broader cameras/hardware, actual macOS-14 execution and final application bundling/licensing/codesigning remain separate checks. Source identity is unchanged only while developed output/metadata remain identical; never weaken gates or silently reinterpret caches/projects if that invariant fails.
