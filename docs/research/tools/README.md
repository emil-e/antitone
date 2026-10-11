# Research tools and cache boundaries

Authored research source was promoted here from ignored `.cache/probes` during the documentation reorganization. This is a research source collection, **not another production backend or a fully maintained test suite**. Run from the repository root. Generated results still go into `.cache/probes`; the preserved source/evidence must not be overwritten by a reproduction run.

## Inventory and compatibility

| Source | Role / status |
| --- | --- |
| `film/Cargo.toml`, `film/src/bin/film-production-check.rs` | Actual production scalar/GPU/complete-target gate; uses read-only `samples/DSC04393.ARW`, no archived source fixture required |
| `film/src/bin/flat-field-inspect.rs`, `film/src/flat_field.rs`, `film/src/bin/flat-field-gpu-check.rs` | Reference-RAW rectangle/smoothing analysis and scalar/GPU source/render verification. [Processing checkpoint](../../plans/completed/flat-field-correction.md#verified-processing-checkpoint), [estimator evidence](../evidence/flat-field-initial.txt), [GPU evidence/reproduction](../evidence/flat-field-gpu.txt). Generated CSVs remain private/cache-only |
| `film/src/bin/preview-budget-check.rs` | Large Retina Fit request compares default Metal rejection with production adapter-supported buffer limits; no RAW IO. [Fix/verification record](../../plans/completed/preview-stability.md) |
| `film/src/bin/film-wide-precision.rs`, `film/*.wgsl` | Resolved candidate arithmetic sweep/microbenchmark; uses that RAW and the locally generated `dmin-trace/run2/fur-scan-ap0-f64.bin` fixture |
| `film/src/bin/camera-u16-precision.rs` | Initial compact-input prototype; preserves expected original-formula extreme-EV failures, not the final production gate |
| Other `film/src/` bins | Historical Dmin/DRT/toe/image experiments. Several predate current source/export APIs and do not compile unchanged; preserved for evidence, not automatically modernized |
| `film-response-docs/{extract-curves.py,fit-curves.py,read.swift,plot-fits.swift}` | Owned manufacturer-drawing extraction/fitting/inspection; local downloaded PDFs and derived data, not independent film measurements |
| `capture-response-controls.py`, `capture-controls.py`, `controls-window-id.m` | macOS window-only layout capture; response harness uses current smoke, old harness is historical. Requires desktop/screen-capture permission, built app/helper and local RAWs |
| `{dmin-trace,drt-lift,film-model-images}/*.swift`, `film-identification.swift`, `read-kodak.swift` | Historical inspection/compositing helpers with fixed local input/output paths; not application automation/presentation references |
| `native/*.cpp`, `native/build-internals.py` | Historical public/internal OCIO, clipping, profile-generation and image-identification diagnostics; native include/library/build paths and source versions matter |

The separate research crate's dependency path points to the root package; it is not part of the production feature graph. Build **named bins**, not `--all-targets`/all research bins. Shader source is included from this durable directory, while RAW-derived fixtures/output remain in local cache. Historical wide/float-source snapshots must not be restored wholesale into production or confused with supported upload APIs.

## Current gate reproduction

Use the checkout-local toolchain/native libraries. These commands do not install global packages:

```sh
CARGO_TARGET_DIR="$PWD/target" CC=/usr/bin/clang CXX=/usr/bin/clang++ \
  ./scripts/cargo.sh build --offline --locked --release \
  --manifest-path docs/research/tools/film/Cargo.toml \
  --bin film-production-check --bin film-wide-precision
DYLD_LIBRARY_PATH="$PWD/.cache/deps/ocio-install/lib" \
  target/release/film-production-check
# Requires the locally generated fur fixture described above:
DYLD_LIBRARY_PATH="$PWD/.cache/deps/ocio-install/lib" \
  target/release/film-wide-precision
```

The production gate uses diagnostic references/settings, not an exact user-session reconstruction. The older wide probe checks its archived source fixture before arithmetic/timing. Creating that fixture through the old Dmin trace may require deliberate adaptation to current APIs; that is not performed by this documentation migration. Production tests in `src/gpu_tests.rs` and `tests/gpu_film_response.rs` remain the maintained regressions.

Historical tools may have fixed fresh-output-directory requirements and no-overwrite exports. Do not rerun them blindly over prior photographic results. Native internal diagnostics require the exact pinned OCIO build and object layout. `build-internals.py` writes objects/executables/logs into `.cache/probes`, not next to durable source. The candidate-internals diagnostic intentionally still needs the local copied upstream `prophoto-boundary-candidate.cpp`; that unapproved experiment was not promoted as owned production code.

## Manufacturer study / capture

Extraction and fitting use Python standard library only. With the cited PDFs already downloaded to `.cache/probes/film-response-docs/`, run `python3 docs/research/tools/film-response-docs/extract-curves.py`, then `fit-curves.py`. Swift PDF/plot helpers use the macOS toolchain. Consult [the study](../film-response-model-study.md) for URLs, drawing axes and measurement/copyright limitations; no downloaded PDF/trace/plot is redistributed here.

For current response layout capture, first build the release editor per [the runbook](../../build-and-probes.md), create `.cache/probes/film-model-images`, then:

```sh
/usr/bin/clang -fobjc-arc docs/research/tools/controls-window-id.m \
  -framework Foundation -framework CoreGraphics \
  -o .cache/probes/controls-window-id
python3 docs/research/tools/capture-response-controls.py
```

This captures only the application window during timed smoke holds; it does not automate Lightroom/Resolve. Output images/logs remain private/regenerable. UI/screen-capture work requires an available unlocked desktop and normal permissions.

## What remains in `.cache` and why

- `.cache/deps/`, `.cache/rustup/`: downloaded dependencies/toolchain, build trees/libraries; reconstruct via existing build scripts. They are not distributable app packaging.
- Generated camera/rendering fixtures, regenerated OCIO intermediates, binaries/objects, duplicate build logs and full-resolution TIFF/EXR/PNG outputs: rebuildable diagnostics, not authoritative source.
- Sample-derived photography/user screenshots: private local material, not automatically committed. The RAWs remain read-only under ignored `samples/`.
- Kodak PDFs/text/drawing derivatives and the copied/modified upstream OCIO boundary experiment: third-party material kept local. URLs, source provenance, reported findings and owned extraction/internal-inspection tools are durable; no wholesale upstream vendoring or copyrighted-document promotion.
- `film-integration-baseline/`: obsolete source/WIP checkpoint, not a restoration path or necessary application dependency. Its diagnostic findings are preserved in the reports.

Do not delete this user's cache as part of a documentation task. After generated inputs are available again, production commands should not depend on hidden authored source. Preserve new reusable tools and decisive compact results outside the cache when a future investigation finishes; see [evidence guide](../evidence/README.md). Historical exact-image reproduction may still require local/private inputs.
