# Independent creative-look authoring

This catalog preserves independently derived, print-informed creative looks as **small versioned model definitions**. A shared offline tool evaluates them, bakes an output-independent ACEScct/AP1 LUT and checks its identity and numerical contract. The editor resolves the four selected/accepted models into the same immutable LUT representation used by imported looks. The four active definitions own independent names in `look.json` and retain the exact source-product references separately in `source_product`. The application, generated cube titles and authoring reports consume that metadata; [editor presentation](../../docs/editor.md#look-selection-prepares-before-acceptance) uses the same catalog metadata. [Colour/output](../../docs/colour-and-output.md#creative-looks-precede-output-rendering) owns placement and import semantics; [projects](../../docs/projects.md#embed-the-accepted-look-not-a-catalog-reference) owns appearance-preserving storage.

The current definitions are:

| ID | Selection status | Interpretation |
|---|---|---|
| `kodak-portra-endura` | selected | Source-reconstructed Portra Endura |
| `kodak-pro-image-ii` | accepted | Corrected Pro Image II paper response |
| `kodak-2383` | accepted | Print-film transmission, with the fixed 5000 K creative viewing adaptation |
| `ilford-multigrade-rc-deluxe-filter-2` | accepted | New RC Deluxe/filter 2, neutral scalar response |
| `kodak-endura-premier` | candidate | Premier paper response |
| `kodak-ektacolor-edge` | candidate | Edge paper response |
| `kodak-endura-metallic` | shelved | Diffuse colour/tone only, not metallic surface behaviour |
| `kodak-endura-premier-metallic` | shelved | Diffuse colour/tone only, not metallic surface behaviour |

`selected` records the canonical Portra reconstruction choice; `accepted` records qualitative content acceptance. Neither is release approval. `candidate` is unconfirmed, and `shelved` remains reproducible but is not active creative work. Explicitly baking a shelved definition does not change its status. Each `look.json` owns its identity, revision, status and limits.

## One implementation, two material families

A spectral definition supplies CMY attenuation spectra and a frozen effective-coefficient table. Published Status-A readings are not dye amounts: the independent derivation solves effective nonnegative coefficients against the original standard's whole-instrument spectral products. Minimum density stays in those coefficients; there is no separately invented base spectrum, extra optical-path factor or paper-white normalization. The same attenuation mathematics supports reflection or transmission interpretations, but that does not recover physical geometry, scattering, fluorescence, film base or projector/screen effects. Driving effective layers independently from AP1 channels is a creative convention, not a recovered negative/enlarger spectral response or calibrated scene-to-printer exposure.

The neutral scalar definition supplies fitted density parameters. Signed **linear AP1 luminance is reduced before** ACEScct/exposure mapping, then the selected paper curve returns neutral attenuation. It is not RGB-first compression, post-render desaturation, a fabricated CMY material or measured warmtone colour. Its density curve is evaluated analytically.

All definitions share:

- **0.6** exposure mapping, pivoted at AP1 0.18, with gray-only exposure offsets. No chromatic identity fit, arbitrary contrast/saturation normalization, strength blend or per-image adjustment.
- The fixed **p3rec50** D65/100-nit authoring reference and its existing radius-0.9 neutral-axis soft boundary, followed by the full pinned ACES inverse. This is a coordinate encoding for one AP1 creative look, not an extra display target or stacked scalar encoding.
- Ordinary selected-output ACES **after** the creative LUT. The LUT is not a ready-made Rec.709/display transform.
- **ACEScct/AP1/D60 input and output**, tetrahedral interpolation, explicit input-domain admission, and signed/HDR support. The displayed domain is `[-0.5, 1.0386752042]`; the native calculation retains the original gray-node construction so gray stays at 19/32 of the domain: lattice index 38/64, 57/96 or 76/128. The offline authoring checks retain explicit domain admission. Application execution clips finite input to this LUT window; it does not reinterpret that window as the limit of valid photographic data or substitute another look.
- The existing numerical gates: raw gray 1e-9, encoded/baked gray 2e-5 AP1, all-node reference round trip 2e-4, no inverse AP1 components outside `[0,1024]`, and actual-file/local tetrahedral agreement 2e-4 AP1. Nonfinite inputs/residuals fail closed rather than comparing as zero error.

`common/spectral.json` contains shared samples independently evaluated from Wyman/Sloan/Shirley's published analytical observer equations and Planck's law at **5000 K**, with half-weight integration endpoints. It is not a tabulated CIE observer/daylight dataset. `common/status-a.json` is an independent transcription of the original standard. Whole-instrument products are used directly, **not multiplied by the viewing illuminant again**.

The controlled spectral integration band is 400–700 nm. A wider plotted source axis is not a claim that every wavelength was measured, and omitted product weight is not a density-error bound. The 5000 K blackbody is **not D50 or xenon**. In particular, 2383 is a transmission-informed creative adaptation, not calibrated cinema projection or a camera-negative/film-grain simulation.

## What is authoritative

Each look has three small text files:

- **`look.json`**: identity/revision, canonical name, secondary source-product attribution for active looks, status, source/model digests, reviewed bake identities and interpretation/release limits. Product references identify the independently studied material, not manufacturer authorship or endorsement.
- **`model.json`**: frozen numerical runtime definition. Color models use exact spectral/coefficient tables; B&W uses its exact scalar parameters/range. Baking reads these values without refitting.
- **`source.json`**: original-publication citation/edition/pages/process conditions, independent numerical readings, source-curve reconstruction and source-ink/ambiguity intervals. Below-resolution dye tails retain explicit censor flags; reported zero is not proof of exactly zero physical absorption.

The numerical source ledger is evidence distinct from runtime solver agreement. Checks require the reconstruction to satisfy the unchanged **0.025-density outside-interval allowance**, visible-band coverage where relevant, and the frozen coefficients to reproduce reconstructed source densities within 1e-8. Intervals preserve unresolved channel/annotation/axis ambiguity; source fits are not unique recovery of manufacturer sensitometry. Graph-reading uncertainty estimates are not statistical confidence intervals or manufacturer tolerances.

These maintained checks validate numerical observations and equations. They **do not reread publisher artwork**, which is not included. Re-digitizing a publication requires obtaining and auditing that edition separately. Do not turn a source-correction or a new fit into an automatic bake-time upgrade of an existing definition.

## Native application generation

The current implementation evaluates all looks, including neutral density, through **one 3D-LUT format**. Built-in catalog models define the derived numeric assets; accepted projects embed those assets instead of requiring their old generators. Newly resolved built-ins currently use **129³ float32**. The single `DEFAULT_GRID` constant in [`looks_builtin.rs`](../../src/looks_builtin.rs) selects both the built-in cache specification and generation size. Grids with 32k + 1 nodes per axis preserve the frozen gray-node alignment; changing density does not change the domain or refit the model. Imports and previously embedded assets retain their own dimensions and samples, without automatic upgrades. Broader photographic/device acceptance remains open.

The application evaluates frozen materials in Rust double precision and uses offline-pinned OCIO 2.6.0 fixed-reference inverse GPU code to recover scene AP1. [`../ocio/looks-reference/`](../ocio/looks-reference/) contains those generated algorithms, shared reach/cusp tables and provenance, outside the model/source catalog. It is independent of selected sRGB/P3 output. End users do not run Python, a compiler or an OCIO installation. The application does not refit source curves, alter gray offsets or substitute a selected-output inverse.

Generation retains the existing all-node range, fixed-reference round-trip and gray gates. Independent C++ fixtures check the material port, and the diagnostic OCIO CPU forward path checks every generated node against the original **2e-4 fixed-reference RGB** limit. That limit is not absolute scene-AP1 CPU/GPU inverse agreement: floating-point differences in HDR AP1 can exceed 2e-4 while the fixed-reference residual remains much smaller. Neither software agreement nor a GPU inverse/forward round trip proves photographic equivalence or cross-device acceptance.

Selection resolves asynchronously and caches successful assets persistently by definition/generator/specification identity. GPU sample resources are weakly deduplicated and retained by live bindings; image budgets conservatively charge their actual shared LUT and intermediate costs, remeasuring mutable image weights. None has no LUT allocation or sampling pass. In-flight/mandatory working resources remain distinct from optional retention and are not an aggregate memory-pressure guarantee. Tetrahedral execution still fetches four nodes per pixel at every size. Larger grids do increase generation, upload, cache loading and project compression costs; those are separate from ordinary grading edits. Broader hardware/latency acceptance remains open.

## Generate and check

Use Python **3.9+**, with only the standard library. Native operations additionally need the existing pinned OCIO **2.6.0** installation and a C++17 compiler. Use the normal [native setup](../../README.md#build-and-run); there is no separate dependency bootstrap. A separate worktree can set `OCIO_PREFIX` to an existing compatible installation, as described in [native-build reuse](../../docs/native-development.md#reusing-native-builds). `CXX` optionally selects the compiler; it defaults to `/usr/bin/clang++`.

Run from the repository root:

```sh
# Inspect identities and statuses; no native dependency needed.
python3 scripts/looks/looks.py list

# Validate every model, source ledger and shared scientific convention.
python3 scripts/looks/looks.py check --all

# Also compile and run synthetic gray/domain/mapping/reference checks.
python3 scripts/looks/looks.py check --all --native

# Bake, load through OCIO, check every node and require reviewed payload parity.
# The output directory must be fresh and under this checkout's ignored .cache/.
python3 scripts/looks/looks.py bake kodak-2383 --size 97
python3 scripts/looks/looks.py bake ilford-multigrade-rc-deluxe-filter-2 --size 65

# Fast schema, identity, source, malformed-data and CLI regressions.
python3 -m unittest discover -s scripts/looks -p 'test_*.py'

# Opt-in native synthetic tests, using the same existing installation.
LOOKS_NATIVE_TESTS=1 python3 -m unittest discover -s scripts/looks -p 'test_*.py'
```

A bake defaults to `.cache/looks/bakes/ID/SIZE/`, with `look.cube`, `native-checks.json` and `validation.json`. `--output .cache/looks/another-fresh-directory` selects another ignored destination. Source/schema/output-path rejection happens before building or replacing any destination. The driver atomically claims a fresh destination before generation, so a competing invocation cannot replace another bake's validation report. Native or payload failure is nonzero and is never a successful validation report. Build products and generated model headers are cached separately under `.cache/looks/build/` by model/native-source/compiler/dependency identity; only complete builds are atomically published into that cache.

Regenerate fixed-reference resources and compact native conformance fixtures into fresh ignored directories before inspecting/promoting a deliberate update:

```sh
python3 scripts/looks/generate_reference.py .cache/looks/reference-new
python3 scripts/looks/generate_conformance.py .cache/looks/conformance-new
```

The latter first requires reviewed 65³ payload parity, then extracts 256 deterministic nodes per selected look: cube corners, the neutral lattice, axis probes and seeded interior samples. `tests/fixtures/looks/native-65.json` stores encoded CPU outputs and pre-inverse fixed-reference values, not a shipping LUT. Application-generated samples are compared numerically; they do not claim the offline CPU bake's exact payload hash.

Native checks use synthetic inputs. They include the scalar luminance-first ordering witness; checking component-wise remapping would be the wrong invariant for B&W.

## Asset identity

The offline CLI retains reviewed **65³ and 97³** controls with frozen numeric payload hashes, independently of the application's current default grid size. The identity covers bytes from `LUT_3D_SIZE` through the domains and all node rows, with R changing fastest. Descriptive comments and `TITLE` are excluded: independent names and source-product attribution can change without changing the numeric look or model revision. Generated cubes retain attribution in comments, not their primary title. The recorded `reviewed_file_sha256` is the complete prior independent artifact digest for traceability, not the expected digest of a subsequently relabelled file. An offline bake at a reviewed size must match that payload exactly; do not refresh hashes merely to pass a failed check.

The eight definitions match their reviewed numeric payloads at both sizes on the pinned native stack. Byte identity across every compiler, architecture or OCIO build is not established. A mismatch is a reason to investigate, not to silently loosen a gate, narrow the domain, nudge an inverse target or normalize the material. A deliberate model/contract change needs a separately reviewed identity/revision.

The current 129³ choice provides approximation headroom: noise-free stress gradients expose density differences that can be subtle in textured photographs, while measured steady-rendering costs were similar on the tested M5 Pro. This is not a universal perceptual threshold or an exact replacement for direct model evaluation. Interpolation agreement, tiny solver residuals and continuity do not establish photographic/monitor acceptance or universal absence of visible ripples. The CMY path retains its coefficient-table approximation and F32 OCIO arithmetic. Floating-point shadow ramps show local slope ripples; increasing density through 97³ and 129³ reduces but does not eliminate them. Cinema Print's broad high contrast and near-black compression already exist in the direct frozen model, which approaches a small nonzero black floor. These are separate observations: a plausible print character does not establish adequate LUT smoothness, and interpolation error is not a reason to refit or normalize the material. Known wider-reference/Rec.2020 output issues are not fixed by this toolkit; it intentionally retains one reviewed authoring reference.

## Rights and exclusions

Only independently authored numerical observations/models and shared implementation are retained here. No external photographic profile, imported finished LUT, reference photograph, experimental gallery, PDF or publisher scan is part of the catalog or a generation dependency. No full generated cubes, executables or logs are maintained assets. Pinned reference algorithms/tables and compact numerical test observations are maintained with their provenance.

Original publications, numerical facts, fitted assets, software and source presentation have distinct rights. Every definition records **release clearance as unresolved**; independent derivation is not a legal clean-room certification or an automatic redistribution grant. Keep source/artwork and generated-asset clearance separate from local content acceptance and future packaging decisions. The neutral operating-point parameterization follows OCIO's BSD-3-Clause code; retain its existing [notice](../ocio/LICENSE-OpenColorIO.txt).
