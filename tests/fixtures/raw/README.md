# Authored RAW test data

These DNGs are original Antitone test data, generated entirely from integer constants by [the maintained recipe](../../../scripts/generate-raw-fixture.py). They contain no photograph, camera sample, private metadata or third-party image. Their shared synthetic identity is **Antitone Authored Bayer patches**; it does not represent a physical camera or film stock.

## Layout and metadata

- 98,950 bytes; little-endian classic TIFF/DNG 1.1, one uncompressed unsigned 16-bit CFA strip, 256×192 sensor sites.
- RGGB: `CFARepeatPatternDim=[2,2]`, `CFAPattern=[0,1,1,2]`, `CFAPlaneColor=[0,1,2]`, conventional rectangular layout. Plane indexes and the RGB plane-colour list are distinct.
- Scalar black 64, white 4095. `BlackLevelRepeatDim` is omitted, using its DNG default 1×1. Explicit repeat dimensions remain in pinned LibRaw's `cblack` even for scalar black, which the production bridge rejects.
- Three neutral gains (`AsShotNeutral=[1,1,1]`); D65 `ColorMatrix1` is the XYZ→linear-sRGB camera matrix, stored as signed rationals. No second illuminant/calibration is needed for this declared basis.
- Active sensor area is the whole sensor. Default crop starts at (8,8), size 240×176; normal camera development retains 256×192. Orientation is top-left. No embedded preview, wall-clock value, random seed or path-dependent metadata.

The grid has four columns and three rows of 64×64 tiles. Values below are camera RGB **above black**; each sensor site stores its CFA channel plus 64:

| Row | Column 0 | Column 1 | Column 2 | Column 3 |
| --- | --- | --- | --- | --- |
| 0 | (3000,3000,3000), Dmin patch | (128,128,128), Dmax patch | (1000,1000,1000) | (2400,1200,600) |
| 1 | (1200,2400,600) | (600,1200,2400) | (3200,3200,3200), above base | (600,1000,1800) |
| 2 | (256,256,256) | (512,512,512) | (1024,1024,1024) | (2048,2048,2048) |

Tile joins use 32-pixel integer-rounded bilinear ramps; centres remain uniform. Smooth joins avoid artificial AHD undershoot from abrupt colour steps. The Dmin/Dmax test rectangles are (16,16,32,32) and (80,16,32,32). Uniform developed values are expected from `(sensor-black)/(white-black)`, with a two-RGB16-code allowance; they are not outputs recorded from a reference run. The above-base patch and Dmax neighbourhood retain signed/HDR model-4 working values.

## Matching correction pair

`bare-light.dng` and `illuminated-patches.dng` are each 246,406 bytes, with 384×320 sensor/developed dimensions and a default crop (8,8,368,304). Other metadata matches the patch fixture. The three patch/correction files total 591,762 bytes.

The bare light has a nominal signal of 2400 in every channel. The scan repeats the twelve patch values across six columns and five rows, with the same smooth tile joins. Both multiply their nominal signal by the same channel-dependent affine illumination at pixel centres:

```text
u = (2*x + 1 - 384) / 384
v = (2*y + 1 - 320) / 320
RGB light = 1 + (0.08,-0.05,0.03)*u + (-0.04,0.06,0.08)*v
```

The recipe evaluates that field as exact integer ratios before rounding to sensor codes. All sites remain above black and below white. Accepted support (16,16,352,288) contains the required 256×256 centre patch.

[Correction tests](../../raw_flat_field.rs) derive expected gains and patch values from this field, including the estimator's measured centre normalization. Their allowances account for authored sensor quantization and estimation, not captured output baselines.

## Analyze roll

`roll-0.dng` and `roll-1.dng` are each 524,934 bytes with 512×512 sensors and crop (8,8,496,496). Their neutral picture rectangle is (96,96,320,320): a two-axis signal ramp from 256 to 2048, with the second frame's picture signal multiplied by 3/4. A 32-pixel transition joins each picture to a broad neutral film-base plateau of 3000. The recipe uses integer arithmetic and the same camera basis as the other fixtures.

[Roll tests](../../raw_roll.rs) sample the declared plateau and ramp through production 8×8 capture footprints. Corrected [framework Analyze cases](../../../docs/development.md#roll-analysis) use the correction pair and a temporary second copy of the illuminated negative to meet complete-roll sample minimums. Working-view cases use `roll-0.dng`'s 496×496 active area to exercise native-pixel navigation with a bounded viewport. The neutral ramp supplies known statistics, not photographic tonal-colour fitting or material identity.

## Embedded preview

`preview.dng` is 99,305 bytes. Its raw sensor pixels equal `patches.dng`; it adds an Exif sRGB declaration and a JPEG thumbnail IFD with orientation 6. The 16×8 baseline JPEG contains two authored neutral 8×8 blocks at codes 64 and 192. The maintained recipe encodes their DC-only coefficients with fixed quantization/Huffman tables; no downloaded image or general-purpose image encoder is needed.

LibRaw can extract the JPEG without unpacking/developing RAW. Decoding with orientation produces an 8×16 display-referred placeholder with two known grey bands. It is a deterministic decoder/worker input, not a realistic proprietary-camera preview.

All six DNGs total 1,740,935 bytes (about 1.66 MiB).

## Checks and maintenance

Cache tests use temporary copies to distinguish saved development identity from disposable cache freshness: appending bytes changes the cache revision without changing the declared sensor data. Such checks establish correctness, not latency budgets.

From the repository root, after normal native setup:

```sh
cargo test --locked --offline --features raw --test raw_dng --test raw_flat_field --test raw_roll
cargo test --locked --offline --features raw,export,project,ocio-reference --test raw_dng --test raw_flat_field --test raw_roll
# Native/editor/Metal worker checks; no desktop window, preferences or user thumbnail cache:
cargo test --locked --offline --features editor --lib authored_ -- --test-threads=1
# Independent ImageIO storage/profile check; Apple Clang, temporary products, no window:
cargo test --locked --offline --features raw,export,ocio-reference --test raw_dng \
  authored_dng_tiffs_reopen_with_independent_imageio_storage_and_profiles \
  -- --ignored --nocapture
python3 scripts/generate-raw-fixture.py --check
# Deliberate fixture maintenance only:
python3 scripts/generate-raw-fixture.py
```

Ordinary tests read the repository binaries; they never generate or download them. Exports and projects go to isolated temporary directories. The recipe uses only Python's standard library and `--check` compares bytes without writing.

Sensor-white/black rejection tests edit isolated temporary copies, never the repository data. Numerical gates and eligible-sample requirements live in the tests.

The scalar rendering/export comparisons use the opt-in `ocio-reference` feature. Production GPU export/worker checks are documented in [development](../../../docs/development.md#export-checks); normal editor builds do not link OCIO.

TIFF/EXR pixel comparisons verify serialization against the actual pipeline, not independent photographic calibration. The opt-in [ImageIO check](../../../docs/diagnostics.md#export) independently reopens TIFF storage and profiles.

## Limits

Pinned LibRaw supplies `[1,1,1,0]` for this three-plane DNG's raw metadata WB. Automatic WB selection shares green with the unspecified fourth gain through the [generic three-color WB boundary](../../../docs/processing.md#start-with-the-cameras-measurements); it does not alter raw metadata or explicit/saved WB. The authored worker tests establish their named async flows, not general composed-UI or native presentation coverage.

Tag choices follow [DNG definitions](https://www.adobe.com/go/dng_spec) and were checked against pinned LibRaw's parser. Adobe DNG Validator acceptance is not established. Successful LibRaw decoding is not general DNG conformance.

Synthetic fixtures do not establish photographic correction/Analyze quality, desktop presentation, physical colour or realistic resource/performance acceptance. [Framework UI checks](../../../docs/development.md#framework-ui-regressions), [thumbnail/real-use acceptance](../../../docs/plans/visual-feedback.md) and [native/physical interaction](../../../docs/plans/inspection-and-interaction.md) address separate boundaries.
