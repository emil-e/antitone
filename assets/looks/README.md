# Independent creative-look authoring

This catalog preserves independently derived, print-informed creative looks as **small versioned model definitions**. A shared offline tool evaluates them, bakes an output-independent ACEScct/AP1 LUT and checks its identity and numerical contract. It is **not wired into the editor, project format or export**.

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
- **ACEScct/AP1/D60 input and output**, tetrahedral interpolation, explicit input-domain admission, and signed/HDR support. The displayed domain is `[-0.5, 1.0386752042]`; the native calculation retains the original gray-node construction so gray falls on lattice index 38/64 or 57/96. There is no silent out-of-domain fallback.
- The existing numerical gates: raw gray 1e-9, encoded/baked gray 2e-5 AP1, all-node reference round trip 2e-4, no inverse AP1 components outside `[0,1024]`, and actual-file/local tetrahedral agreement 2e-4 AP1. Nonfinite inputs/residuals fail closed rather than comparing as zero error.

`common/spectral.json` contains shared samples independently evaluated from Wyman/Sloan/Shirley's published analytical observer equations and Planck's law at **5000 K**, with half-weight integration endpoints. It is not a tabulated CIE observer/daylight dataset. `common/status-a.json` is an independent transcription of the original standard. Whole-instrument products are used directly, **not multiplied by the viewing illuminant again**.

The controlled spectral integration band is 400–700 nm. A wider plotted source axis is not a claim that every wavelength was measured, and omitted product weight is not a density-error bound. The 5000 K blackbody is **not D50 or xenon**. In particular, 2383 is a transmission-informed creative adaptation, not calibrated cinema projection or a camera-negative/film-grain simulation.

## What is authoritative

Each look has three small text files:

- **`look.json`**: identity/revision, status, source/model digests, reviewed bake identities and interpretation/release limits.
- **`model.json`**: frozen numerical runtime definition. Color models use exact spectral/coefficient tables; B&W uses its exact scalar parameters/range. Baking reads these values without refitting.
- **`source.json`**: original-publication citation/edition/pages/process conditions, independent numerical readings, source-curve reconstruction and source-ink/ambiguity intervals. Below-resolution dye tails retain explicit censor flags; reported zero is not proof of exactly zero physical absorption.

The numerical source ledger is evidence distinct from runtime solver agreement. Checks require the reconstruction to satisfy the unchanged **0.025-density outside-interval allowance**, visible-band coverage where relevant, and the frozen coefficients to reproduce reconstructed source densities within 1e-8. Intervals preserve unresolved channel/annotation/axis ambiguity; source fits are not unique recovery of manufacturer sensitometry. Graph-reading uncertainty estimates are not statistical confidence intervals or manufacturer tolerances.

These maintained checks validate numerical observations and equations. They **do not reread publisher artwork**, which is not included. Re-digitizing a publication requires obtaining and auditing that edition separately. Do not turn a source-correction or a new fit into an automatic bake-time upgrade of an existing definition.

## Application execution direction (not implemented)

The agreed application direction is **one 3D-LUT execution contract**, including the neutral look, so material-derived looks can share a renderer with looks from other sources. Model definitions remain authoritative; their LUTs are derived execution artifacts, not replacements for the source data. Selection should resolve a cached LUT or generate and validate a missing one asynchronously, then upload it. Successful generations should be persistently cached by the complete numerical definition, generator version and LUT specification, rather than the display name. GPU residency is a separate bounded cache.

Cold-generation latency has not been measured for application use. It will determine whether first-use generation is sufficient or bundled prebakes are useful. The offline Python/compiler/OCIO toolchain is not an end-user runtime dependency to introduce implicitly: on-demand baking needs an application-ready implementation. Final sampling/precision, other-source admission, selection persistence, preview/export integration and EXR policy remain open.

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

Native checks use synthetic inputs. They include the scalar luminance-first ordering witness; checking component-wise remapping would be the wrong invariant for B&W.

## Asset identity

Both reviewed sizes, **65³ and 97³**, have frozen numeric payload hashes. The identity covers bytes from `LUT_3D_SIZE` through the domains and all node rows, with R changing fastest. Descriptive comments and `TITLE` are excluded. The complete prior independent file digest is separately recorded for traceability. A generation must match the reviewed payload exactly; do not refresh hashes merely to pass a failed check.

The eight definitions match their reviewed numeric payloads at both sizes on the pinned native stack. Byte identity across other compilers, architectures or OCIO builds is not established. A mismatch is a reason to investigate, not to silently loosen a gate, narrow the domain, nudge an inverse target or normalize the material. A deliberate model/contract change needs a separately reviewed identity/revision.

The 97³ bakes were used for photographic review and generally improved approximation over 65³, but **no final shipping approximation threshold is agreed**. Interpolation agreement, tiny solver residuals and continuity do not establish photographic/monitor acceptance or universal absence of visible ripples. The CMY path retains its coefficient-table approximation and F32 OCIO arithmetic. Known wider-reference/Rec.2020 output issues are not fixed by this toolkit; it intentionally retains one reviewed authoring reference.

## Rights and exclusions

Only independently authored numerical observations/models and shared implementation are retained here. No external photographic profile, imported finished LUT, reference photograph, experimental gallery, PDF or publisher scan is part of the catalog or a generation dependency. No generated cubes, binaries or logs are maintained assets yet.

Original publications, numerical facts, fitted assets, software and source presentation have distinct rights. Every definition records **release clearance as unresolved**; independent derivation is not a legal clean-room certification or an automatic redistribution grant. Keep source/artwork and generated-asset clearance separate from local content acceptance and future packaging decisions. The neutral operating-point parameterization follows OCIO's BSD-3-Clause code; retain its existing [notice](../ocio/LICENSE-OpenColorIO.txt).
