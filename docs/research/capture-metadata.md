# Capture metadata, not RAW byte identity

The user rejected immutable-RAW-content identity and subsequently deferred capture-identity/relink design as premature. Full RAW hashing is removed outright, with no metadata replacement or old-project compatibility. [Accepted decision](../decisions/no-raw-content-hashing.md). This header inventory remains research evidence, not an implementation requirement or a selected relink policy.

## Header-only evidence

On main `6809ff5` in `.worktrees/rendering-performance`, `native/raw_identity_probe.cpp` opened metadata through the existing pinned LibRaw 0.22.0/OpenMP build, without unpacking, development, full-file hashing or sample modification. Checked 44 supplied Sony ILCE-1M2 ARWs: `DSC04378`…`DSC04416`, the four flat-field reference inputs `DSC01737`/`DSC01739`/`DSC01811`/`DSC01813`, and `DSC01591`.

| Metadata | Observed presence / distinctness |
| --- | --- |
| `ImageUniqueID` | absent in all 44 |
| `RawDataUniqueID` | all-zero/absent in all 44 |
| Public/internal body serial | absent in all 44 |
| `other.shot_order`, Sony image count/sequence index | absent/zero in all 44 |
| Sony shots since power-up | nonzero in 38, only 15 distinct values; unsuitable alone |
| EXIF `DateTimeOriginal` (`0x9003`) | present in all 44; 19-character valid calendar timestamps, 44 distinct |
| EXIF `SubSecTimeOriginal` (`0x9291`) | present in all 44; three decimal digits (milliseconds) |
| EXIF `OffsetTimeOriginal` (`0x9011`) | present in all 44; valid signed HH:MM offset |
| Make/model + original date/subseconds/offset tuple | 44 distinct among 44 checked inputs |

This is finite sample evidence, **not global uniqueness**. Two cameras can share a model and timestamp; clocks may reset; metadata can be absent, copied or edited. Filename alone does not establish capture identity. How a future relink workflow should choose candidates is not settled; first-match with explicit manual correction may be sufficient. Do not interpret this inventory as a requirement for strict automatic matching or a hash fallback.

Private full metadata, including actual capture dates/counters, stays in ignored `.cache/metadata-identity/headers.jsonl`; only field availability/format/counts are retained here. The header probe compiled with Apple Clang `-Wall -Wextra -Werror`. The existing stale GCC library-search-path warning did not prevent linking; it was not repaired as part of metadata research. No production/native bridge or project schema behavior changed.

## Upstream provenance and public API

Pinned `.cache/native-deps/LibRaw-0.22.0-openmp` source:

- `libraw/libraw_types.h`: `ImageUniqueID`/`RawDataUniqueID`, shooting body serials, `other.timestamp`/`shot_order`, Sony counters. Camera/body identity is not itself a capture identity.
- `src/metadata/tiff.cpp` and `src/metadata/exif_gps.cpp`: ImageUniqueID (`0xa420`), RawDataUniqueID (`0xc65d`) and body serial (`0xa431`) are read from metadata, not computed by LibRaw from RAW pixels/file contents. Availability depends on the input and parser.
- `libraw/libraw.h::set_exifparser_handler` and `libraw/libraw_types.h::exif_parser_callback` expose raw EXIF tag values through a public callback. TIFF/EXIF parser call sites save/restore the input position; the probe also restores it. No independent EXIF parser or Sony-specific processing repair is needed.
- `src/metadata/misc_parsers.cpp::get_timestamp` reads a 19-character date and calls `mktime`. The public epoch omits subseconds and does not preserve the original timezone representation. **Do not use that host-dependent epoch as a portable identity.** Read original calendar/subsecond/offset values through the callback instead.

Make/model plus original capture date/subseconds/offset was considered as a discriminator, adding serial or a meaningful supplied per-image ID when available. That design was not selected. Presence in local samples does not establish a requirement to persist identity or reject supported files with missing metadata. Production does not use these fields for relinking or grade acceptance.

## Reproduction

Run from the repository/worktree root using existing pinned native inputs; outputs remain ignored:

```sh
mkdir -p .cache/metadata-identity
/usr/bin/clang++ -std=c++17 -O2 -Wall -Wextra -Werror -mmacosx-version-min=14.0 \
  -I .cache/native-deps/LibRaw-0.22.0-openmp \
  native/raw_identity_probe.cpp .cache/native-deps/LibRaw-0.22.0-openmp/lib/libraw_r.a \
  -L .cache/native-deps/openmp-22.1.8-install/lib -lomp -lz \
  -Wl,-rpath,"$PWD/.cache/native-deps/openmp-22.1.8-install/lib" \
  -o .cache/metadata-identity/probe
.cache/metadata-identity/probe samples/DSC04378.ARW samples/DSC04416.ARW \
  > .cache/metadata-identity/headers.jsonl
```

The recorded 44-file inventory additionally used the source ranges/pairs above. The probe prints original metadata and must not be used to automatically promote private outputs. No native dependency setup/download was needed. Existing development, camera basis/matrices, signed/HDR AP1/AP0, geometry/flat-field processing and numerical gates remain outside this identity-policy change.
