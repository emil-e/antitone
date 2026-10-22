# Canon and Nikon RAW support (broader LibRaw acceptance)

Status: plan drafted from research; awaiting sample acquisition before implementation.

## Agreed scope

Accept Canon (CR3, CR2) and Nikon (NEF) camera RAWs through the established LibRaw
development path, alongside the existing Sony ARW and DNG inputs. This is delegated by
the user (2026-10-22) after web research into which cameras people actually use for
film scanning; the deliverable agreed was "a work plan for CR2/CR3 + NEF support (or a
broader format-acceptance spike)".

Why these first: community scanning threads (r/analog, Negative Lab Pro setup threads)
are dominated by Canon (legacy 5D/6D macro donors plus EOS R) and Nikon (D850 is *the*
repeatedly recommended scanning body), after Sony. Share ranking is inferred from
thread frequency, not measured. Every peer negative-conversion tool (Negative Lab Pro,
SmartConvert, Grain2Pixel, VueScan ≥9.8.43, darktable Negadoctor) accepts
CR2/CR3/NEF/ARW/RAF/ORF/RW2/DNG — this task closes Antitone's gap against that
baseline.

### Out of scope (separate future tasks)

- X-Trans RAF (Fuji X): non-Bayer CFA, needs a different demosaic story.
- Pixel-shift composites (Sony ARQ, Lumix S5 hi-res RW2): community norm is conversion
  to DNG (PixelShift2DNG), which the DNG path already covers.
- ORF, RW2 (single-frame), PEF, medium format, floating-point and four-colour
  sensors. NRW included only if trivial (same code path as NEF and a sample exists).

## Current state (established from code)

- LibRaw 0.22.0 is already the non-DNG decode/develop engine; the picker already
  lists CR2/CR3/NEF etc. as hints (`src/editor_files.rs`). The gates live in
  `native/raw_bridge.cpp`:
  - `antitone_raw_open`: requires `colors == 3`, non-zero Bayer `filters`
    (`filters != 9` excludes X-Trans).
  - `antitone_raw_unpack`: rejects any declared repeated black pattern
    (`cblack[4] || cblack[5]`), **including 1×1 scalar-equivalent declarations**.
- The authored DNG fixtures (`tests/fixtures/raw/`) already trigger exactly that
  1×1 case (`generate-raw-fixture.py` output is rejected by the production bridge) —
  the deterministic starting case the retired `docs/plans/raw-compatibility.md` plan
  called for (removed in 91f4152; its goal/scope is preserved here).
- Sony-specific code (`raw_sony.rs`, `raw_lens.rs`) is embedded lens correction, not a
  decode gate; other makes should yield `SonyLensDataStatus::Absent` — verify.
- Roll calibration is per camera basis (`CameraBasis` equality in `src/project.rs`);
  Canon/Nikon rolls need their own calibration/rolls, as Sony ones already do.
  Flat-field measurement is camera-agnostic.

## Open question that determines the shape of the work

Which Canon/Nikon files pass the current gates unchanged, and which are rejected only
by the black-pattern rule? LibRaw normalizes most Canon/Nikon black levels to
scalar/per-channel values, but per-body behavior must be inventoried, not assumed.
The work could collapse to "validation + tests + docs" if files already pass, or need
real black-pattern handling if bodies we care about (e.g. D850/Z-series with per-site
black data) do not.

## Phases

1. **Sample acquisition (blocked — needs user input).** Real CR3, CR2 and NEF files,
   ideally actual negative scans from the camera setups in question. Samples are
   private, read-only, ignored and never committed; downloading samples (e.g. from
   public raw-sample repositories) needs explicit authorization, and the sandbox
   blocks network access. Ask the user to supply files or authorize a source, plus
   which bodies matter (e.g. EOS R / 5D-series / D850 / Z-series).
2. **Inventory (read-only spike).** For each sample: dump LibRaw `idata`/`rawdata`
   (`colors`, `filters`, `cblack`, WB, camera matrix, `sizes.flip`, linearization),
   run the current bridge, and record pass/fail per gate. Map every assumption in
   `native/raw_bridge.cpp` and `src/raw.rs` (flip, black/white handling, WB basis,
   green handling, thumbnail selection, `ExifColorSpace`).
3. **Black-pattern acceptance.** Widen `antitone_raw_unpack` to accept patterns that
   reduce deterministically to scalar/per-channel levels (1×1 first, then uniform
   patterns), keeping rejection of genuinely spatial black variation unless the
   inventory shows a needed body requires more.
4. **Per-format validation.** Following the old plan's acceptance list: decode,
   sensor diagnostics, fixed-WB development, calibration, correction, proxy mapping,
   Save/Open, TIFF/AP0 EXR output; RGB16 agreement across thread policies; rejection
   of malformed/unsupported variants. Real-sample tests follow the existing
   `#[ignore]`d Sony pattern (`samples/DSC04378.ARW`); authored fixtures for the
   widened black-level cases via `scripts/generate-raw-fixture.py` recipes.
5. **Docs.** Update the supported-boundary text in `docs/processing.md` ("three-colour
   integer Bayer without repeated black-level patterns"), the README input sentence,
   and any compatibility notes — following the documentation skill.

## Acceptance

- Real-sample ignored tests for at least one CR3, one CR2 and one NEF covering the
  full path in phase 4, mirroring the Sony sample tests.
- Authored tests proving the widened black-pattern gate accepts 1×1/uniform
  declarations and still rejects spatial variation.
- Existing ARW and DNG behavior numerically unchanged (fixed matrices/expected
  values only touched with justified cause, per the development guide).
- Docs updated; plan kept current in checkpoint commits.
- Independent review before integration (per AGENTS.md), explicitly covering test
  quality — including which bodies the samples represent and the
  fixture/control-discovery assumptions.

## Blockers and next action

**Blocked on samples + body list from the user** (phase 1). Once supplied: phase 2
inventory, then re-confirm this plan's phase 3 scope before changing the gate.
