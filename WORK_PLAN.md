# Digital input stage — design and first milestone

## Agreed scope

Design, then implement at M1, a first-class **Digital** input stage as an alternative to
negative inversion. Longer term the digital path will also carry negative simulation,
grain and halation, and slide-film input will arrive — so there will be **more than two**
input stages, and the abstraction must not be a two-way toggle.

**M1 goal:** coherent UX when importing normal photos (digital camera RAWs) and grading
them through the unchanged scene-referred pipeline. No JPEG/TIFF import at M1.

### Decisions locked (this session)

- **Default input stage = Digital.** Import is permissive and undemanding; the user never
  has to pick "Digital" first (import first, settings after).
- **Negative is the strict mode.** It enforces one shared scanning setup and **blocks**
  non-aligning sources (on switching *to* Negative and on add) "just like before"
  (`CameraBasis::require_source`). **Digital does not restrict** — mixing cameras /
  manufacturers is fine.
- **Calibration (Dmin/Dmax/input_floor) belongs to Negative's input settings**, not a
  separate top-level concept. It is measured (Dmin/Dmax picks), shared across a negative
  roll, and basis-specific — which is exactly why it is negative-owned and why mixed-basis
  is a real constraint for negatives and a non-issue for digital.
- **Tools/sections hide per input stage** (not merely disable): Analyze, Auto-crop,
  Flat-field, Calibration, Gamma & balance, Toe & shoulder are negative-only. Orientation
  and the shared Roll grade (WB, Exposure) show for every stage.
- **Acquisition basis is per-source for Digital** (own camera→AP1, own as-shot/fixed WB,
  own lens correction). For Negative it stays shared (one scanning setup).
- **M1 accepts digital camera RAWs only.** JPEG (with an inverse ACES output transform to
  recover scene-referred) is an explicit *"maybe someday — honestly just use Photoshop"*
  roadmap item, not planned work.

## The seam: input stage vs grading stage

One boundary does the work. Everything below it is shared and blind to the input type.

```
source raster (camera RGB in fixed WB basis)   ← shared acquisition (decode, lens, flat-field)
        │
        ▼
  INPUT STAGE  ── interpret polarity/response ──►  ungraded scene-referred working positive
  (per-roll)                                       (signed/HDR AP1)   ← THE SEAM
        │
        ▼
  GRAADING STAGE ── Roll WB/Exposure, Photo Exp/WB/Contrast/Sat, ordered locals
  (blind to input)
        │
        ▼
  OUTPUT STAGE ── creative look → ACES → TIFF/EXR/scopes
```

- **Input stage:** "what kind of photograph is this, and how does it become a positive?"
  Negative film *inverts*; a digital photo *is already positive* (pass-through); slide film
  is positive *with film characteristics*.
- **Grading stage:** operates only on the working positive and never knows which input
  stage ran.
- The seam already exists as typed code — `RollSettings::inversion_only()`
  (`src/processing.rs:158`) splits input (response/gamma/balance) from grade (wb/exposure);
  `NeutralRecipe` (`src/image_recipe.rs`) and `ViewSourceSpec.neutral`
  (`src/gpu_working_view.rs:28`) are its homes. **The work names and generalizes this
  boundary into a strategy; it does not invent one.**

A key simplification: the **source-acquisition layer is already shared**. A film scan and a
digital capture both decode to linear camera RGB in a fixed WB basis, then a camera→AP1
matrix. Only the *polarity/response interpretation* differs. A no-inversion path already
exists (`process_camera_source`, `src/shaders/film_response.wgsl:157`) used today for
source inspection; **Digital is that path promoted to a first-class grading input.**

## Input stage abstraction

Settings split (today `RollSettings` mixes both):

```
InputSettings                                   RollGrade  (shared, all stages)
  Negative { calibration {dmin,dmax,input_floor},   { wb, exposure }
             response, gamma, balance }
  Digital  { /* empty at M1; future sim/grain/halation */ }
  Slide    { … }                              (future)
```

`InputStage` enum/trait with one job: `interpret(source samples + camera basis + stage
settings) → UngradedPositive`. The Negative stage owns its pick/measure UX and the
`Calibration::for_source` flat-field folding as internal mechanics. Variants:
- **Negative** — existing `Inversion` (Dmin/Dmax, response, gamma, balance).
- **Digital** — no inversion; `process_camera_source` conversion promoted to a grading path.
- **Slide** — future; positive-film response (clear-base anchor, no polarity flip).

Generalize `Pipeline::new` (`src/processing.rs:355`) to take an `InputStage` instead of
hard-coding `Inversion`. Everything after the seam (grade.apply, locals, looks, rendering)
is unchanged and shared. GPU: formalize the existing `process_film` / `process_camera_source`
branch into per-stage entry points.

**Persistence & compatibility** (`src/project.rs`): add an `input_stage` tag to
`Project`/`Development`. `Development::inversion_space` (currently the fixed
`"native-camera-RGB"`, `src/project.rs:58`) becomes a per-stage identity. Per
[projects.md](docs/projects.md#compatibility-is-about-the-processing-basis), the input
stage joins the camera basis as part of a roll's processing basis. `Development::fixed_wb`
becomes per-source for Digital (default = source's as-shot); it stays shared for Negative.

## ROLL tab UX

Reframe the tab from "negative inversion" to "input interpretation + shared roll grade."
The tab help string (`src/editor_ui_hierarchy.rs:34`) and the flat ordering change to teach
the mental model (interpretation at top, shared grade at bottom, middle adapts):

```
┌ ROLL ────────────────────────────────────────────┐
│ Input stage:  [ Digital ▾ ]        (top, always)  │
│   help: "How this roll's photos become a positive │
│          before grading."                         │
│ ── Input interpretation (varies by stage) ──      │
│   Negative: Flat-field · Calibration · Gamma &    │
│             balance · Toe & shoulder              │
│   Digital:  (none at M1) → later: Negative sim,   │
│             Grain, Halation                       │
│   Slide:    (future) film base · response         │
│ ── Roll grade (shared, always) ──                 │
│   White balance · Exposure                        │
└───────────────────────────────────────────────────┘
```

Each input stage declares an **applicability set**; the ROLL UI renders only those (no
scattered `if digital`):

| ROLL surface | Negative | Digital | Location |
|---|---|---|---|
| Orientation (Turn/Flip/Reset) | ✓ | ✓ | tool shelf (`editor_analysis.rs`) |
| Analyze (roll analysis) | ✓ | hide | tool shelf (`editor_analysis.rs:258`) |
| Auto-crop | ✓ | hide | `editor_auto_crop.rs`, rendered across tabs (`editor.rs:974`) |
| Flat-field | ✓ | hide | `editor_flat_field.rs` |
| Calibration (Dmin/Dmax) | ✓ | hide | `editor.rs:986` |
| Gamma & balance | ✓ | hide | `editor.rs:1023` |
| Toe & shoulder | ✓ | hide | `editor.rs:1063` |
| Roll White balance, Exposure | ✓ | ✓ | shared roll grade |

Auto-crop (film-gate) and Analyze (fits negative calibration/grade) render **outside** the
per-tab section body, so gating reaches their render sites, not just the ROLL panel. A
future digital auto-grade is a *different* feature, not Analyze repurposed (roadmap). The
Photo and Rendering tabs are untouched — they operate post-seam.

## Switching input stages

- **Where:** a per-roll **Input stage** selector at the top of the ROLL tab. Input stage is
  a *roll/project* property (like camera basis), not per-source.
- **Presentation:** a dropdown ("Input stage ▾"), not a binary toggle — must scale to N
  stages. Each option carries a one-line description.
- **Settings are non-destructive:** each stage's settings (and Negative's calibration)
  persist independently; switching changes only which is active and which controls show.
  Switching Negative→Digital→Negative restores measured Dmin/Dmax. Shared `RollGrade`
  carries across. No confirmation dialog (reversible, non-destructive).
- **Guard (blocks entering Negative):** the Negative option is enabled **only when all
  sources share one native-camera basis** (`CameraBasis` compatibility, `src/project.rs:113`).
  Otherwise disabled with a plain reason — *"these RAWs don't share one scanning setup;
  Digital handles mixed sources."* One gate at the mode level.

## Compatibility model

Reconciles "import first, settings later" with negative's real constraint:

- **Digital (default): no restriction at all.** Import anything; mix cameras / manufacturers
  freely; per-source acquisition basis. This is the "don't restrict" ideal, achieved for the
  common case.
- **Negative (strict): enforces one shared scanning setup, blocks non-aligning sources.**
  Both on switching *to* Negative (guard above) and on add/import while in Negative
  ("just like before" — `Project::add_source` / `CameraBasis::require_source`,
  `src/project.rs:378`). Negative genuinely needs one shared, basis-specific calibration.
- **Import is otherwise permissive.** The blocks bite only in the mode that needs them.

**Fresh-import default rationale:** negative cannot render without measured Dmin/Dmax
(`Project::calibration()` is `None` until both are set, `src/project.rs:421`; processing
requires it), so a fresh negative shows a setup/prompt state, not a wrong-looking image.
Digital (pass-through) renders immediately with zero setup — coherent first-import UX for
the milestone. Cost: a film-scan user's fresh import shows the un-inverted scan until they
click Negative (then "measure Dmin/Dmax"), remembered per project after.

## First milestone (M1) — the abstraction

**M1 is the abstraction itself** (user, this session): the seam, the strategy, and Digital
grading through it. The fuller ROLL-tab layout polish is a follow-on increment, not M1.

Core (the abstraction):
1. `InputStage` enum + settings split (`InputSettings` / `RollGrade`) + `Pipeline::new`
   generalization.
2. **Digital** stage = promote `process_camera_source` to a grading input (no inversion).
3. Persistence (`input_stage` tag) + compatibility identity.
4. Grade / UI / thumbnail / export paths consume the working positive unchanged (they
   already do).

Minimum coherent UX (enough to use it — not the full redesign):
5. An input-stage selector + per-stage hiding (Digital shows no negative-only controls).
6. Permissive import + per-source acquisition for Digital; a minimal Negative guard/block.

Follow-on increment (not M1): grouped ROLL-tab layout, help-copy overhaul, richer switching
affordances, and the full applicability table above.

**Acceptance:** digital RAWs (single- and mixed-camera) grade through the unchanged
scene-referred pipeline in Digital with no import friction; the abstraction cleanly separates
input interpretation from grading (a third stage can be added without touching grading);
negative output is unchanged. Full UX polish is the next increment.

## Roadmap (out of M1)

- **Slide film** (input stage #3): positive-film response, clear-base anchor, no polarity
  flip. Falls out of the `InputStage` strategy — the reason we generalize now.
- **Negative simulation** (digital path): digital positive → simulated negative capture +
  print. The `digital-input-poc` dye/CMY/AP1 studies are relevant modelling prior art here.
  Likely lives in the **input stage** (Digital variant), not the look.
- **Grain** and **Halation**: *spatial*, **scene-linear-light** effects on the working
  positive (capture-domain, not display creative-looks). Placement analysis (this session):
  halation/grain are linear and global, *revealed* by highlight compression but not created
  by it. Exposure (linear scalar) commutes → does not decide placement. The placements that
  actually differ are relative to **WB** (cross-channel matrix; WB before/after halation
  changes the halo colour) and the **nonlinear grade + ACES display** (what reveals the
  glow). Recommended home: a **spatial film-simulation pass on the working positive, at the
  seam, before grading** — physically faithful and cheap; accept that WB/contrast grade
  them. A "stable-look" variant (halo unaffected by WB) is a product knob, not physics.
  *Open product-intent choice* — see Open items. Spatial machinery gap remains (neighbourhood
  access → a distinct spatial pass, not per-pixel input interpretation).
- **JPEG import** (inverse ACES output transform to recover scene-referred): explicit
  *"maybe someday — honestly just use Photoshop."* Not planned.
- **Digital auto-grade** (a different feature from Analyze): auto white-balance/exposure for
  digital inputs.

## Remaining steps

- [x] **CPU seam (the abstraction) done + tested** (worktree `refactor/input-stage-seam`):
      `InputKind` (persisted) + `Interpretation` (runtime strategy) + `Pipeline`
      interpret→grade split (`to_graded_ap1` / `working_to_graded` / shared `grade`).
      `Pipeline::new` = negative (unchanged), `Pipeline::digital` = no inversion.
      Scalar tests: digital ≡ camera→AP1 identity + shared grade; negative bit-unchanged.
- [ ] **GPU Digital path** (careful, multi-variant). See “GPU notes” below.
- [x] **Persistence**: `InputKind` on `Project` (default **Digital**; round-trips).
      **No persistence compatibility** — break the format freely. Saved projects no one
      cares about yet; do not accumulate pre-release compat cruft (`skip_serializing_if`,
      legacy-defaults, old-file loading).
- [x] **Clean input-stage model (the “proper refactor”)** — `RollSettings` split so it reads
      as if multiple input stages were always the design:
      - `InputSettings` (enum, per stage): `Negative { calibration, response, gamma, balance }`
        | `Digital`. Owns calibration + interpretation; `NegativeInput` is its negative view.
      - `RollSettings` (shared grade): `{ wb, exposure }`.
      - `Pipeline::new(&InputSettings, RollSettings, PhotoSettings, DMat3)` dispatches on the
        stage (`Inversion::new(NegativeInput)` vs identity); `Pipeline::ungraded(&InputSettings,…)`.
      - **Core migrated + base lib compiles clean** (0 errors): `processing`, `sampling`,
        `roll_analysis`, `roll_analysis_gamma`, `local_adjustments`.
- [ ] **App-wiring cascade (remaining):** base + `gpu` features are **done + compiling**.
      The `editor` feature still needs the old-`RollSettings` interpretation usage migrated.
      Native LibRaw/OpenMP are seeded in the worktree `.cache/native-deps/` (offline build
      works; do **not** run `build-openmp.sh`/`build-libraw.sh` — they re-download). `cargo
      check --features editor --lib` shows the target sites (~70, 20 files). Plan:
      - `processing`: `Calibration` now derives serde (done). Add `InputSettings::Negative`
        → `Negative(NegativeInput)` tuple variant if the editor controls want a clean
        `&mut NegativeInput`; keep serde `tag="kind"`.
      - `editor`: replace fields `dmin: Option<DVec3>, dmax: Option<DVec3>, floor: f64` +
        `roll.{response,gamma,balance}` with `input: InputSettings`; keep `roll: RollSettings`
        (grade `wb`/`exposure`). `calibration()` → `input.negative_input().map(|n| n.calibration)`;
        `sync_document` → `project.input = self.input; project.roll = self.roll;` (drop the
        `dmin`/`dmax`/`input_floor`/`processing_model` field writes — now inside `InputSettings`).
        ROLL controls (editor.rs ~1018–1129) write `self.input`'s Negative fields (gamma,
        balance, response, calibration picks) + `self.roll.{wb,exposure}`.
      - `image_recipe`/`editor_processing`/`repair_source`/`scan`/`editor_dust`/
        `editor_analysis`/`editor_measurement`/`editor_editing`/`gpu_working_view`/
        `editor_thumbnail_worker`/`gpu_export_area`: thread `&InputSettings` where they passed
        `calibration` + call `Pipeline::interpret`/`ProcessingParameters::interpret`/
        `SpatialPipeline::new`/`GpuPhotoGrade::{new,validate,validate_repaired}`.
      - Persistence: `Project.input: InputSettings` (replaces `dmin`/`dmax`/`input_floor`/
        `input_stage`); default-Digital-on-import (editor_document register); drop the
        camera-basis lock for Digital.
      - Then tests + `cargo build --features editor --bin antitone`.
- [ ] UI selector (deferred — follow-on UX increment).
- [ ] When integrating: fetch latest `origin/main` (network was blocked this session),
      rebase the branch, required fresh-context review before integration.

## GPU notes (code shape + findings, this session)

**Code-shape principle (user):** the different stages and components must be clearly
visible in the shader — named functions per stage (`interpret_positive` →
`camera_positive_to_ap1` → `photographic_post`/grade), the input-stage `switch` confined to
`interpret_positive`, named stage constants (not `> 0.5`/`> 1.5` magic thresholds). The
shader compiler inlines/optimizes regardless, so structure is free — legibility over
micro-optimization. Mirror the CPU `Interpretation` shape.

**Findings that widen the GPU work:**

- Inversion lives in **multiple shader variants**: `film_response.wgsl` (interactive),
  `film_inversion_precise.wgsl` (working view), `film_inversion_export.wgsl` (export).
  Each needs an interpret stage (or at least those in the vibe path). WGSL function names
  are internal (`include_str!`-concatenated in `gpu.rs`); nothing references them from Rust,
  so renames are safe.
- **Packed uniforms are intricate:** the exposure gain is folded into `anchor.w` as
  `ln(gain)`; camera→AP1 is a *separate* `camera_matrix` uniform; all split-constant
  f64-emulation. The f32 preflight bounds (`floor_bound`/`negative_bound`) are
  inversion-specific; **Digital needs a different bound** (no inversion continuation —
  source magnitude through camera→AP1 · post · grade).

**Scope the GPU Digital work (preview-first):** trace how `gain`/`camera_matrix` are applied
in the shader → `ProcessingParameters::digital` + `interpret_positive` digital branch for
the **interactive** path first (the import-and-see-it vibe) → GPU parity tests (negative
unchanged, digital correct) → then precise/export variants + routing.

## Open items / decisions

- **Terminology pass: “Scan” is too specific** (film-scan-centric; reads wrong for a digital
  input). Rename `Scan` / `ConvertedScan` / “scan-illumination” / “scanning setup” to an
  input-agnostic word across code + shaders + docs. **Target word to be chosen later**
  (candidates: `Capture`, `Frame`, or source-aligned `Source`/`DevelopedSource`). **Keep
  “Roll”** — it correctly encodes settings shared across the roll vs per-photo (unlike
  Lightroom), so it is *not* part of the rename. Do it as a **separate behaviour-preserving
  pass** (trivial to verify/review in isolation), after the functional seam is verified and
  before integration. The **user-visible “scanning” wording** (ROLL tab help) lands with the
  ROLL-tab UX work. Record-only now — do not decide the word here.

- **Grain/halation product-intent choice** — physical-simulation (scene-linear, at the seam,
  before grading; WB/contrast grade the effect) vs grade-stable-look (after photographic
  grade; unaffected by WB). Physics places them in scene-linear light at the seam and makes
  exposure irrelevant to the choice; the free choice is only WB/tone interaction. Decide the
  product intent when that work starts.
- **Scan auto-detect** (content signal: orange base mask + inverted luminance) to default
  Negative for real film scans — only if the one-click cost for scan users proves annoying.
  Not built at M1.
- Prior negative-inversion-model studies (`experiment/digital-input-poc` and related
  worktrees) are **roadmap prior art** for negative simulation, not this milestone.

*(Archived: the concluded doc-cleanup task's plan → `.cache/plans/doc-cleanup-WORK_PLAN-archived.md`.)*
