# LibRaw DHT corrections

[dht.patch](dht.patch) corrects allocation, concurrency, range and border handling in LibRaw's Bayer DHT implementation. It applies to [`src/demosaic/dht_demosaic.cpp` in LibRaw 0.22.3](https://github.com/LibRaw/LibRaw/blob/cf02d66eb5c19d4e46df7aee0582e47a4918509a/src/demosaic/dht_demosaic.cpp). The patch contains library corrections only; Antitone's demosaicer selection and pixel-producer revision live outside it.

## Problems and solutions

### Workspace allocation and offset bounds

The constructor assigns unchecked `malloc` and `calloc` results to `nraw` and `ndir`. A failed RGB allocation is subsequently dereferenced; a failed direction allocation leaves an unusable object. Adding throwing checks alone would also require cleanup when construction fails after the first allocation: the object's destructor does not run for a failed constructor.

The patch gives both arrays direct `std::unique_ptr` ownership and uses throwing array allocation. Completed members release their storage during constructor unwinding. LibRaw's existing `dcraw_process()` handler catches `std::bad_alloc` and returns an insufficient-memory error. The direction array remains zero-initialized.

DHT's `nr_offset()` and traversal use signed `int` offsets. The padded pixel count is computed in `size_t` and rejected with `LIBRAW_EXCEPTION_TOOBIG` before allocation if it exceeds `INT_MAX`, avoiding overflow in the subsequent indexing.

### In-place neighbour updates under OpenMP

Three work-sharing loops allow one iteration to read a location while another modifies it:

| Pass | Shared state and dependency |
| --- | --- |
| `hide_hots()` | Reads neighbouring `nraw` samples and temporarily replaces detected extreme samples in that same array. |
| Final `refine_ihv_dirs()` pass | Reads cardinal neighbours in `ndir` and changes the current direction byte. |
| Final `refine_idiag_dirs()` pass | Reads eight surrounding direction bytes and changes the current byte. |

These unsynchronized accesses are data races. Even if individual byte accesses are atomic on the machine, the algorithm can observe different neighbour states depending on execution order. Authored weak-channel mosaics with hot/dead samples expose output differences between OpenMP team sizes.

The patch executes these three passes in their defined serial row order. The two checkerboard HV refinements remain parallel: each updates one parity while reading the other. Independent direction-generation and colour-reconstruction stages also remain parallel, with their existing stage barriers.

Ordered execution preserves single-threaded update semantics. Atomic loads/stores alone would not establish that order; snapshot-based passes would change the update semantics and require additional storage. This correction trades some parallel throughput for deterministic updates without another full-image buffer.

### Zero-channel bounds and biased minima

DHT represents zero internally as `0.5` **in native RGB16 code units**, keeping ratios defined. Three range calculations conflict with that representation:

- A channel with no nonzero samples has an integer maximum of zero. Clamping interpolated green to zero reintroduces zero denominators; an all-black image can reach a NaN-to-`ushort` conversion.
- Zero observations are skipped when updating minima. If the initial minimum for a channel is positive, later zeros cannot lower it, and the global clamp can invent light inside a zero-signal region.
- Unconditionally adding `0.5` to minima raises positive bounds above the measured samples. For example, a constant green value of 1024 can acquire a lower bound of 1024.5, biasing ratio-derived red in an otherwise constant field. Which bound is affected depends on the first CFA site's colour.

The patch makes maxima floating-point with an initial value of `0.5`, includes zero observations in minima, and floors minima at `0.5` instead of incrementing them. Ratios remain positive internally; half-code results truncate back to zero in the integer image. Measured CFA samples retain their original integer values.

### Image borders

The padded workspace is initialized to `0.5`; it is an arithmetic halo, not observed light outside the image. Interpolation near it can introduce colour errors on constant fields.

After copying DHT's result to the image, the patch calls LibRaw's existing `border_interpolate(4)`. This reconstructs missing channels in the outer four pixels from measured CFA neighbours while preserving measured sites. The interior retains DHT reconstruction.

## Regression coverage

[dht_safety.cpp](dht_safety.cpp) includes the actual DHT implementation and supplies authored Bayer data, so it needs neither Antitone's renderer nor private RAW files. Its native state declares coherent image/intermediate dimensions, three colours and the CFA layout. LibRaw owners are heap-allocated, as the large object can exhaust a worker thread's stack.

The checks cover:

- Discovery of actual array allocations, failure at each discovered site, partial/successful cleanup, unchanged input and offset rejection before allocation.
- Black, saturated, absent-channel and unbalanced constant fields across all four Bayer phases, including borders. Nonzero constant output has a one-code rounding allowance; absent-channel output must remain exactly zero.
- A bright-to-zero transition, requiring a region well away from the transition to remain zero.
- Small 2×2, odd/even and non-square images.
- Measured-site preservation, observed OpenMP teams of 1/2/8 and simultaneous independent owners.
- Actual parallel row kernels dispatched through `std::thread`, with joins exposing stage barriers to ThreadSanitizer without depending on OpenMP-runtime instrumentation.

Team/owner equality checks establish consistency; authored constant/zero-light expectations independently check numerical correctness. For a stock-LibRaw report, isolate the relevant case: the aggregate executable starts with allocation discovery that expects the patched array allocator and can stop before reaching a different defect.

## Build and sanitizer checks

Run the normal native setup from the repository root. It verifies the archive and source hashes, applies the patch and runs the OpenMP regression executable:

```sh
./scripts/build-libraw.sh
```

On Windows, use `scripts/build-libraw.ps1`. Both scripts consume [dht-hashes.txt](dht-hashes.txt) and select a build directory by the corrected-source hash. Patch changes require updating that manifest. [Native development](../../docs/native-development.md#reusing-native-builds) covers compatible dependency reuse.

For additional checks on macOS, after native setup:

```sh
revision=$(awk -F= '$1 == "patched" {print substr($2, 1, 12)}' native/libraw/dht-hashes.txt)
libraw="$PWD/.cache/native-deps/LibRaw-0.22.3-$revision-openmp"
openmp="$PWD/.cache/native-deps/openmp-22.1.8-install"
mkdir -p .cache/libraw-checks
for sanitizer in thread address,undefined,float-cast-overflow; do
    name=$(printf '%s' "$sanitizer" | tr , -)
    clang++ -std=c++17 -g -O1 -fsanitize="$sanitizer" -I"$libraw" \
        native/libraw/dht_safety.cpp "$libraw/lib/libraw_r.a" \
        -L"$openmp/lib" -Wl,-rpath,"$openmp/lib" -lomp -lz \
        -o ".cache/libraw-checks/$name"
    TSAN_OPTIONS=halt_on_error=1 UBSAN_OPTIONS=halt_on_error=1 \
        ".cache/libraw-checks/$name"
done
```

These commands instrument the included DHT source and harness, not the rest of LibRaw. They deliberately omit OpenMP compilation for the explicit-thread row-kernel check; the normal setup executable separately exercises real OpenMP teams.

## Preparing an upstream submission

The allocation/indexing, in-place concurrency, numeric bounds and border changes can be submitted independently with their corresponding regression cases. Compare the target upstream revision before applying the patch, and distinguish deterministic scheduling changes from numeric/border output changes. The numerical changes can alter extrema; the border change intentionally alters missing-channel estimates at image edges. Include authored failing-before/passing-after cases rather than relying on one photograph's identical thread-count fingerprints.
