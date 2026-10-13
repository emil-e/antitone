# Contrast/saturation reference experiment

Reference/research probe: scalar candidates and private comparison renders, not a production UI or saved-grade path. The selected production families/defaults are defined separately in the [processing contract](../../../contracts/processing-model.md#photo-contrastpivot-and-saturation). [Design plan](../../../plans/contrast-and-saturation.md), [prior art and results](../../contrast-and-saturation.md#reference-experiment-checkpoint).

## Operators

All inputs/outputs are signed linear AP1. `Y` uses the actual `AP1_TO_XYZ` row; `1` means neutral RGB. ACEScct is transient. Identity amounts return the original vector exactly, including across blends. Nonfinite inputs/results and invalid proposals reject; no intermediate clamp, gain cap, epsilon division or source repair is used.

### Contrast

Let `E/D` be ACEScct encode/decode and `f(x)=D(E(p)+a*(E(x)-E(p)))`. This uses the specified linear toe, but **no high-end interchange clipping on decoding**; finite overflow rejects instead. Research ranges are `a=0.5..1.5`, linear pivot `p=0.01..1`. Rendered comparisons use `a=0.8/1.2`, `p=0.18`. These are the historical experiment bounds; the production contract owns supported ranges. The retained scalar implementation/tests also accept 0.4..1.6 to verify the approved production extension; comparison variant amounts are unchanged.

- **RGB endpoint:** `out_c=f(input_c)`.
- **Black-offset-ratio proxy:** define `b=f(0)`, `g(Y)=(f(Y)-b)/Y`, then `out=b*1+g(Y)*input`. At `Y=0`, use the analytic derivative `g(0)=f'(0)`. `operators.rs` evaluates the divided difference with linear branches/`expm1` and a continuous piecewise integral across the decode toe, rather than an arbitrary near-zero threshold. The cross-toe integral was added during production verification to prevent rounded constants/cancellation becoming a near-zero-Y gain jump; historical galleries were not regenerated for that corner fix.

This proxy matches the baseline's **neutral** tone curve/pivot and maps luminance to `f(Y)`. Its invariant is the direction/ratios of **`out-b*1`**, not unconditional ratios of output RGB, absolute chroma or perceived saturation. For positive Y it retains input chromaticity relative to the same transformed black. The inherent `b` is algebraic decomposition of the selected tone curve, not a second pedestal/correction: black is still `f(0)`, including its negative value under increased contrast. The extension is finite/continuous at signed zero-Y vectors and uses positive secant gain; signed chromaticity ratios may have no perceptual meaning.

Coupled RGB contrast generally has different luminance on nonneutrals. We match contrast strength on the **entire neutral curve**, not by secretly refitting each photograph. This is a declared comparison proxy, not Resolve HDR's undisclosed perceptual model. A future appearance-model comparison remains separate.

### Saturation

Research ranges: `s=0..2` for the algebraic models and `s=0.25..2` for density power. Comparisons use `0.7/1.3`.

| Endpoint | Equation | Distinction |
| --- | --- | --- |
| Constant Y | `Y*1+s*(input-Y*1)` | Preserves AP1 luminance; can create negative channels |
| Maximum anchor | `M*1+s*(input-M*1)`, `M=max(input)` | HSV H/V plus scaled S on positive RGB; algebraic signed continuation; maximum fixed; adds negative channels on strong boosts |
| Density power | `A*sign(input_c)*abs(input_c/A)^s`, `A=max(abs(input))` | Idealized relative-absorption model; preserves channel signs and maximum magnitude, but can change RGB hue ratios |

For positive RGB, density power follows relative transmission `T_c=input_c/M`, ideal density `D_c=-log10(T_c)`, and `out_c=M*10^(-s*D_c)`. Zero channels are defined directly by the power limit. The signed extension uses magnitude/sign; all-negative neutrals remain unchanged. Zero RGB returns zero. `s=0` is deliberately **not supported** for this candidate: a zero transmission stays zero for every `s>0`, so the limiting operation is not a uniform monochrome endpoint. For `s<1`, continuity at zero does not imply a finite derivative; it can amplify small signed channels/noise. Maximum selection is continuous but not everywhere differentiable in both anchor families.

This is print-density **inspiration**, not measured dye spectra/layer response, a 2383 emulation or a print LUT. AP1 channels are not physical dyes. Neutrals retain their tone and no second DRT is introduced. Kodak's material motivates the family, not these particular equations.

### Blends and strength matching

Each stage evaluates both endpoints on the **same stage input**, then interpolates their outputs in linear AP1 at `t=0,0.25,0.5,0.75,1`. Contrast goes first, saturation second. The blend is not parameter interpolation or sequential endpoint application; it promises neither endpoint invariant at intermediate t.

Maximum-anchor and constant-Y saturation have identical AP1 opponent distance `C=length(input-Y*1)` at the same amount, though different luminance. Density power does not. Alongside nominal amounts, match density power to the constant-Y `C` on a fixed mild-colour diagnostic `[0.22,0.18,0.14]`, with a bracketed scalar solve: `s≈0.646558/1.424893` matches baseline `0.7/1.3`. The tested match bracket is monotonic for that reference; this is not a universal matching function for arbitrary colours. No exposure/brightness normalization is applied. This is one-space/one-patch matching, not a claim of equally perceived or image-wide saturation.

The full matrix has 45 variants: identity, 10 contrast blends, 30 saturation blends (maximum, nominal power, matched power; both directions), and four contrast/saturation interactions at both blends `0.5`. The simpler photographic view retains 13: baseline, RGB/proxy contrast plus their halfway blend in both directions, and constant-Y/maximum-anchor saturation plus their halfway blend in both directions. Halfway is `t=0.5` output interpolation before ACES, not half the grade amount or a blend of displayed images. Print-density power remains research-only in the full matrix; quarter-point blends, interactions, synthetic input and sRGB are also omitted from the simpler view.

## Reproduction

Run from the repository root using the normal pinned native setup described in [the README](../../../../README.md). No new dependency/download/install is required. Populate local LibRaw/OpenMP/OCIO caches before native rendering; a worktree can reuse the same caches with ignored links. Samples are read-only and their directory is supplied explicitly.

```sh
cargo test --locked --offline --test contrast-saturation-reference

# Start with the simpler view; choose a NEW output directory (existing dirs refuse).
cargo run --locked --offline --release --features raw,export \
  --bin antitone-contrast-saturation-probe -- --simple \
  /absolute/path/to/samples .cache/probes/contrast-saturation/run

# Direct ImageIO conversion avoids sips's mandatory per-user scratch directory.
/usr/bin/clang -fobjc-arc docs/research/tools/contrast-saturation/to-png.m \
  -framework Foundation -framework ImageIO \
  -o .cache/probes/contrast-saturation/to-png
.cache/probes/contrast-saturation/to-png .cache/probes/contrast-saturation/run
python3 docs/research/tools/contrast-saturation/verify-png.py \
  .cache/probes/contrast-saturation/run

open .cache/probes/contrast-saturation/run/index.html
```

Omit `--simple` for the full matrix. Use `--synthetic` instead of the RAW directory for synthetic-only rendering; it cannot be combined with `--simple`. The PNG helper refuses existing outputs. Its verifier proves every RGB16 code matches the TIFF, then reinstates the exact TIFF ICC (ImageIO can substitute named system profiles). TIFFs carry the pinned Antitone target ICC and remain reference outputs. No application automation or monitor transform is part of this probe.

`run.txt` and `photographic-inputs.txt` record both photographic modes; the full matrix also writes `diagnostics.csv`/`metrics.csv`. Private images/manifests stay ignored. All image dimensions are `360×240`; synthetic bands are signed neutral, HDR neutral, midtone hue/chroma, HDR hue/chroma, channel-maximum crossover and signed near-zero Y.

In simple mode, `index.html` shows only three exposure choices each for ring (`+2.5/+3.0/+3.5 EV`) and canopy (`+1.5/+2.0/+2.5 EV`), without creative adjustments. `comparison.html` shows one photo and two independently selected endpoints or halfway blends at a time, with baseline available on either side. Its provisional exposures are dog `+2.5`, ring `+3.0`, canopy `+2.0 EV`, held identical across models. Exposure brackets apply a uniform linear AP1 gain before creative processing/ACES, not post-render brightening. All 45 TIFF/PNG pairs are P3 (39 comparison images plus six bracket images). Confirm exposure first; another chosen baseline requires regeneration. The explicit constants/shortlist live in `simple.rs`; templates/style are maintained beside it.

Photographs use current `Scan`/`Pipeline`, camera-native measurements and new-grade toe 2.5/shoulder off, not historical AP0 constants. The source recipe is explicit in `main.rs` and the output manifest: clear-film Dmin from dog, diagnostic Dmax from the current train RAW regression, roll-neutral fit on a visually white ring, photo exposure `+2.5 EV` in the full matrix or the per-photo values above in simple mode, default photo WB and no flat field. These are illustrative diagnostic grades, not recovered user projects. Both ACES targets process **two stratified original pixels per output cell before target-linear averaging**. This is a bounded photographic comparison, not full-resolution settled/export reconstruction or noise/aliasing acceptance.

## Checks and limits

The probe binary also checks the simple-mode shortlist, midpoint output interpolation and exact identity (`cargo test --locked --offline --features raw,export --bin antitone-contrast-saturation-probe`). Scalar tests cover exact amount identity, encoding round trips, common neutral response/pivot, monotonic neutral ramps, black-relative direction/Y, zero-Y and toe-boundary continuity, signed/HDR saturation invariants, endpoint output blending, strength matching, invalid proposals and overflow rejection. This is an f64 reference experiment, not a GPU precision/range gate.

Review small-photo overall character and synthetic transitions first. Do not infer skin protection, noisy-shadow safety, full-resolution texture quality, physical-monitor acceptance, corrected-source integration or preferred production defaults from these few uncorrected photographs. Those comparisons remain open. [Recorded snapshot and next choice](../../contrast-and-saturation.md#reference-experiment-checkpoint).
