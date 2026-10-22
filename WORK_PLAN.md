# Canon and Nikon RAW support (broader LibRaw acceptance)

Status: inventory complete; all sampled Canon/Nikon files pass current gates and
develop end-to-end. Implementation phases below are re-scoped accordingly.

## Agreed scope

Accept Canon (CR2, CR3/CRAW) and Nikon (NEF) camera RAWs through the established
LibRaw development path, alongside existing Sony ARW and DNG inputs, with the
user's acceptance criterion (2026-10-22): "unless it's rare and implementation is
annoying, it should work" — i.e. common bodies people own must work; rejection is
justified only for rare formats where support would be disproportionate.

Research basis (2026-10-22): community scanning setups are dominated by Canon
(legacy 5D/6D plus EOS R) and Nikon (D850 the most-recommended scanning body) after
Sony; every peer negative-conversion tool accepts CR2/CR3/NEF/ARW/RAF/ORF/RW2/DNG.
Share ranking is inferred from thread frequency, not measured.

## Design directives from the user (2026-10-22)

- Prefer appropriate abstractions over manufacturer-specific RAW handling; reuse
  existing code (e.g. lens corrections) where it genuinely applies, not
  gratuitously.
- Adapt existing abstractions where they are Sony-shaped but actually generic, so
  the code reads as if all these formats were always considered. Candidates
  identified: `RawMetadata.sony_file_type`/`sony_lens` fields on the make-agnostic
  metadata struct, the `raw_probe`/diagnostic output treating Sony fields as
  universal (`sony_type=65535` sentinel), and any make-conditional naming in the
  development path. Sony-specific code that is genuinely Sony-only (embedded Sony
  lens profiles in `raw_sony.rs`/`raw_lens.rs`) stays clearly scoped to Sony.

## Sample set (private, read-only, never committed; stored in this worktree's `samples/raw-downloads/`)

Actual camera-scanned negatives (pixls.us Play Raw threads, CC BY-SA / CC BY-NC-SA):

- `negative-scans/negadoctor-challenge-canondslr.CR2` — Canon EOS 600D (flip=6 portrait)
- `negative-scans/negadoctor-snowy-day-d780.NEF` — Nikon D780
- `negative-scans/autumn-scene-a7c.ARW` — Sony ILCE-7C (control)

Generic body coverage (raw.pixls.us, CC0): Canon 5D Mark II, 6D, EOS R (RAW + CRAW),
R6; Nikon D750 (12-bit compressed + 14-bit lossless), D7200, D850, Z 7; plus
`6d-sraw.CR2` as the excluded-layout control.

## Inventory results (established, `.cache/raw-inventory/` probe + `antitone-raw-probe`)

- **All sampled Canon/Nikon files pass both bridge gates** (`colors==3`, Bayer
  `filters`, integer `raw_image`, no repeated black pattern): CR2
  (lossless_jpeg), CR3/CRAW (crx), NEF (nikon_load_raw, 12/14-bit,
  compressed/lossless). No black-pattern widening is needed for these bodies.
- **End-to-end development works**: all files develop with zero repeat error and
  LibRaw-sRGB matrix agreement ~1.5e-5 via `antitone-raw-probe`, including
  full matrix, fixed-WB camera development, sensor diagnostics.
- **sRAW/mRAW is the first legitimate exclusion**: LibRaw presents it as
  `filters=0`, non-CFA (pre-demosaiced/decimated). Document as unsupported with a
  clear reason; users can shoot full RAW.
- **D780 negative scan**: blue channel sits ~170–460 codes above `black=1008`
  (verified correct via the same-offset Z 7 normal photo showing ~0 below-black
  sites). It trips `antitone-raw-probe`'s diagnostic >1%-channel flooring gate,
  which exists only in the probe; the library path handles near-black pixels via
  per-pixel eligibility. Legitimately dense LED-panel scan, not a decode defect —
  but it is a good real-world fixture for low-SNR-blue behavior.
- **Per-camera variation observed, all handled by existing generic paths**: WB
  scales (Nikon normalized G=1.0, Canon/Sony 1024-based; `as_shot_wb` already
  normalizes), `ExifColorSpace` 255 (Nikon) vs 1 (embedded preview path declines
  safely without an ICC profile), Canon raw margins, `flip=6` portrait.

## Remaining implementation phases

3. **Acceptance & naming.** No gate changes for the sampled classes. Re-scope
   any Sony-shaped naming per the design directives; keep the rejected-variant
   errors honest (sRAW/mRAW, X-Trans, spatial black patterns).
4. **Portrait-orientation handling.** Justified by the planned digital pipeline
   (simulating a digital RAW as a negative with grain/halation), where general
   photography exercises non-zero flip; scans themselves rarely need it (user,
   2026-10-22). Implement flip handling in the developed-raster path rather than
   only embedded previews, and verify geometry, crop/scan regions, auto-crop and
   flat-field consistency with the 600D fixture (`flip=6`), which exercises the
   path even though it is a scan.

### Lens correction position (user topic, 2026-10-22)

The embedded-correction boundary stays Sony-only, by evidence: Sony is the
only make with publicly validated makernote corrections (darktable's
interpretation, which we mirror); Canon CR2/CR3 and Nikon F NEF embed only
settings (DPP/NX-D apply corrections from vendor databases, Canon DLO from
   server downloads); LibRaw 0.22 exposes nothing for native RAW makernotes;
peers (darktable, RawTherapee, Adobe) use external measured databases
(LensFun/LCP) for Canon/Nikon. Under the "rare and annoying" criterion:

- Canon/Nikon scans are accepted without embedded lens correction. Scanning
  rigs use macro lenses near 1:1 where distortion is minimal, and illumination
  is already covered by measured flat-field — the established workflow Sony
  embedded data merely supplements.
- The internal profile/geometry math in `lens.rs` is vendor-neutral; only the
  payload reader is Sony-specific. Per the abstraction directive, phase 1
  renames Sony-shaped internals (`SonyLensProfile`/`SonyLensGeometry`) so the
  vendor payload is the swappable part — no second vendor is implemented now.
- Deferred, tied to the digital-pipeline future work: Nikon Z embedded
  distortion (makernote `0xc7d5`, ExifTool-documented, vignette semantics
  unvalidated publicly) and LensFun/LCP-style measured-database support for
  general lenses. The DNG path already applies vendor corrections via opcodes
  where converters bake them, which covers DNG-routed digital input.
5. **Real-sample tests.** `#[ignore]`d sample tests mirroring the Sony pattern for
   at least CR2, CR3/CRAW and NEF (develop, diagnostics, fixed-WB basis, export
   path where sample-dependent checks exist). Keep the probe's D780 dense-blue
   case as a documented diagnostic behavior, not an editor rejection.
6. **Black-pattern scalar case (small, optional).** The 1×1 repeated-black
   declaration (authored fixtures, some future bodies) still rejects at unpack;
   accept scalar-equivalent declarations while keeping spatial rejection. This
   is the old plan's starting case and remains a real defect for DNG-via-LibRaw.
7. **Docs.** processing.md's accepted-boundary sentence ("three-colour integer
   Bayer without repeated black-level patterns"), README input sentence, and
   the editor/docs scanning guidance — following the documentation skill.

## Acceptance

- Real-sample ignored tests for at least one CR2, one CR3 and one NEF across the
  full develop path; CRAW included; sRAW/mRAW rejection covered by a control
  fixture assertion.
- Portrait-scan consistency verified against the 600D fixture.
- Existing ARW and DNG behavior numerically unchanged.
- Docs updated; plan kept current in checkpoint commits.
- Independent review before integration, explicitly covering test quality and
  the make-specific/generic abstraction boundaries.

## Blockers

None currently. Open: whether ORF/RW2 (single-frame) follow in this task or a
follow-up once Canon/Nikon land (research says smaller shares; same code path).

## Artifact locations

- Samples: `samples/raw-downloads/` (this worktree, ignored).
- Disposable inventory probe + logs: `.cache/raw-inventory/` (not maintained).
- Full probe log: `.cache/raw-inventory/probe-full.log`.
