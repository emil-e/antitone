# Spektrafilm-backed colour treatment

This catalogue supplies camera-negative and print-media processing for Antitone's native colour-treatment baker. It uses spektrafilm by Andrea Volpato, pinned at `3bb2c2d2801ff68b92019cf1dbcbb133d60832bc`, retaining the upstream stock/media names. [Provenance](provenance.json), [modifications](CHANGELOG.txt), the [GPLv3 code license](LICENSE-code.txt) and [CC BY-SA data license](LICENSE-data.txt) accompany it. Antitone's top-level MIT license does not relicense these components.

## Data and execution

The generator evaluates the upstream deterministic, nonspatial model. It exports characteristic curves and their DIR pre-correction, the donor/receiver matrix, windowed sensitivities, dye/base spectra, input-gamut remapping coordinates and enlarger calibration. Hanatos's spectral family is shared across stocks. The stock-specific optimized window is enabled; the chromaticity correction surface and additional camera UV/IR filters are disabled.

The Rust [consumer](../../src/looks/spektrafilm.rs) constructs a stock exposure lookup and pair-specific spectral printing lookup on demand, then samples one final ACEScct/AP1 LUT. Installed data does not contain final LUTs for every combination. The current on-demand spectral printing grid is 33³; its cubic interpolation is an implementation choice checked against source evaluations.

Each print medium has a sampled density-to-readout component combined with the pinned inverse authoring DRT. Supra Endura compresses gamut directly in the wider authoring reference, without a separate lightness rolloff; other media retain upstream sRGB finishing before conversion to that reference. Fixed exposure calibration is independent of these output choices. The component is not a measured scanner-device profile. The component stores ACEScct/AP1 coordinates; ordinary selected-output ACES still follows the accepted final LUT. End users need neither Python, an OCIO installation nor a compiler.

The camera-negative entry contains upstream negative-film profiles, not reversal films needing a different positive-output interpretation. Printing media include papers and upstream printing-film profiles. Without a selected negative, the interpreted positive enters a declared exposure adapter. Other media assign AP1 channel stops directly to layer exposure at authoring compression 0.6. Full film+paper uses spectral projection and print time calibrated to the frozen reference's neutral output Y=0.1.

Supra paper-only uses the historical Academy universal negative reference to obtain three APD flux constraints. A convex minimum-relative-entropy selector chooses a positive spectrum relative to the supplied 250D base. Its probability measure is summed APD action; the selected shape is invariant to a common flux scale. Positive relative-filter interpolation and held endpoints define transmission between and beyond the APD knots, with opaque support outside the supplied negative spectra. The TH-KG3 lamp and paper sensitivities then determine receiver exposures. These are explicit rendering assumptions, not recovered film layers or a named negative's characteristic curves.

Only adapter records outside the nonnegative reference cone move toward neutral along a constant-input-Y ray. Nonpositive input Y has zero extra input chroma. Receiver log exposure retains a Taylor continuation below the query threshold 1/128; signed working AP1 remains intact upstream. A scalar common-exposure solve holds original spectral viewed Y to the scalar neutral trajectory, preserving its tone-dependent tint. The neutral law retains gray 0.18 and authoring compression 0.6. Photometric weights include the original base, viewing illuminant and CIE Y; source observations use Brent roots and the native baker uses safeguarded Newton/bisection. No photograph determines the spectrum prior or an output colour fit.

Only final numeric LUTs execute on photographic pixels. Supra paper-only uses 129³ samples to resolve its saturated exposure gradients; other treatments use 65³. The finer grid reduces interpolation error at roughly eight times the sample storage. Explicit bake dimensions remain available for diagnostics.

Source interpretation, photographic grading and existing grain remain upstream of this treatment. The [output guide](../../docs/colour-and-output.md) owns selection and viewing, and [project storage](../../docs/projects.md#embed-accepted-colour-assets) owns saved numeric assets.

## Regeneration

Normal Cargo builds and packages embed the versioned components here. Scientific Python and an upstream checkout are needed only for explicit regeneration, currently on macOS Apple Silicon.

Use an existing Python **3.14.6** interpreter and the normal [pinned OCIO setup](../../docs/build-and-run.md#build-and-run), or set `OCIO_PREFIX` to a compatible shared installation. The [build tool](../../scripts/spektrafilm/build.py) fetches the immutable source revision into `.cache/spektrafilm/upstream` and creates a local virtual environment from [hash-locked headless wheels](../../scripts/spektrafilm/requirements.lock). It does not install the upstream GUI or a system interpreter.

```sh
python3 scripts/spektrafilm/build.py setup --python /path/to/python3.14
python3 scripts/spektrafilm/build.py generate --output .cache/spektrafilm-candidate \
  --apd-responsivities /path/to/APD-2010.csv
python3 scripts/spektrafilm/build.py check
```

The separately supplied APD CSV has columns `wavelength_nm,red,green,blue`; its numeric hash identifies the complete 186-row [S-2008-002 v1.1 (2010) table](https://github.com/aces-aswf/aces-core/blob/v1.0.3/documents/LaTeX/TB-2014-005/S-2008-002.pdf). The reference-derived components are a private trial with unresolved public redistribution clearance; the spektrafilm code/data licenses do not grant that clearance. Keep the original source notice and resolve publication permission before distributing them publicly.

Generation rejects a modified or differently pinned upstream checkout and writes only a fresh candidate. Its output includes numeric source observations and provenance for the source, dependency lock, exporter, reference helper and component bytes. The checker validates candidates before anything is copied into maintained assets. `--upstream` and `--python` allow reuse of an existing verified checkout/environment.

Inspect the candidate and stage runtime `*.z`, `index.rs`, `selection.json` and `provenance.json` here, with `conformance.json` in [test fixtures](../../tests/fixtures/spektrafilm/). Then run the native/component GPU conformance tests before committing the components and fixtures together:

```sh
cargo test --locked --release --features ui-tests --lib looks::spektrafilm
python3 -m unittest discover -s scripts -p 'test_*.py'
```

The scanner grid is currently 33³. A new upstream pin needs source and numerical review: data/parameter changes can regenerate, while algorithm changes may also require a native port update. If the fixed authoring DRT changes, regenerate its [reference resources](../ocio/README.md#fixed-reference-for-creative-look-generation) too. Compact data and model conformance are separate from the [public-release licensing review](../../NOTICE.md#local-packages-versus-public-releases).
