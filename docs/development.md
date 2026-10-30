# Making and checking a change

Read [architecture](architecture.md) before changing application code, then use the [topic guides](README.md) to understand the affected behavior. [Build and run](build-and-run.md) owns setup and commands; [build and delivery](build-and-delivery.md) owns packaging and CI.

## Engineering standards

### Code and design

Apply the architectural principles rather than copying neighboring code unquestioningly. Existing code, tests and documentation describe present choices; they can contain design defects. Identify the responsibilities, lifetimes, authority and contracts involved before choosing a structure.

Keep changes focused and preserve unrelated work. Explain consequential choices through their benefits, costs and affected callers or saved data. Distinguish product intent from implementation choices; an existing algorithm, numeric representation or API is not automatically a requirement. Surface conflicts instead of silently weakening a commitment or expanding the task into a rewrite.

Name values by role outside conventional mathematics, separate logical phases, and expand dense control flow. Prefer the simplest useful baseline; add machinery when an observed need justifies it. Keep tools with an ongoing contributor/build/diagnostic role; retire completed study scaffolding and record lasting knowledge in its owning guide.

### Comments and documentation

Explain current behavior, rationale and non-obvious constraints. Omit obvious restatements, patch narration and task history. Public documentation describes supported behavior rather than private call paths; comments preserve important ownership, numerical, compatibility and platform knowledge.

Read a guide end-to-end before editing it. Improve the existing explanation rather than adding a parallel account, and link to the owning topic. Length is a cost. A negative sentence should prevent a concrete reader mistake, not merely disclaim a claim nobody would act on.

Run `python3 scripts/check-docs.py --base HEAD`. Fix broken links and judge its word-growth, long-paragraph and hedge reports. Keep execution notes local and untracked; contributor guides explain the system, not the work session.

### Meaningful tests

For each case and each assertion, ask: **What plausible defect would this catch, and why would it matter?** Derive expectations from requirements, justified invariants or independent references. Agreement between two production paths can establish consistency without establishing correctness.

For photographic processing, GPU-rendered output is the implementation under assessment. An independent CPU renderer does not become an oracle through agreement. Prefer materially simpler analytic helpers, justified invariants or fixtures with established provenance; recording current output alone does not establish that provenance. Keep test-only implementations in helpers or fixtures.

Inspect fixture setup and control discovery as well as assertions. Positive controls and contrasting data should distinguish a working operation from a no-op or universally disabled UI. Avoid self-comparisons and expectations copied from implementation calculations. Test names should describe what the fixture actually exercises.

Distinguish meaningful interaction, accessibility, serialization and lifetime guarantees from incidental wording, dimensions, colours or layout. Exact assertions can be essential, but need that justification. Before removing or reshaping a test, identify the protection that must remain. A passing test is not a reason to freeze a poor design or accept an unjustified numerical tolerance.

## Verification

Choose checks for the changed contracts, including composed consumers—not only the edited function. Investigate failures before changing the test strategy; do not weaken assertions, suppress failures or force serial execution to obtain a pass. Confirm that filtered tests were discovered and report exclusions and uncertainty.

### Everyday checks

The numerical core needs no native libraries or GPU:

```sh
cargo test --locked --no-default-features
```

Use the committed lockfile; add `--offline` when dependencies are already available. For application work, run the owning library/integration cases and relevant production/all-feature checks. Check production separately so reference-feature unification cannot conceal a runtime dependency. `cargo xtask check` is the broader maintained workflow; [CI](build-and-delivery.md#continuous-integration) covers only its hardware-independent subset.

### Source formatting

Run `python3 scripts/check-format.py` before handoff. It checks authored sources; `--write` applies the configured formatters without weakening their rules. The checked-in toolchain and maintained setup pin the tools. Use [normal setup](build-and-run.md#build-and-run) or existing compatible tools through `RUFF`, `CLANG_FORMAT` and `WGSLFMT`, rather than an agent-only bootstrap.

### Feature-specific checks

Find relevant tests through the [topic/module map](README.md#module-map) and the owning code. `editor` checks the production application; `ocio-reference` supplies OCIO comparisons and test diagnostics, not a production fallback. Native/GPU/editor checks need the corresponding [prerequisites](build-and-run.md). [Authored RAW fixtures](../tests/fixtures/raw/README.md) exercise declared sensor data without private photographs.

Justify numerical expectations independently; test exact supplied-sample serialization separately. Verify capture/routing, persistence, physical lifetimes and user interaction at their own boundaries. Unit or offscreen evidence must not be reported as photographic, native-window or physical-input acceptance.

#### Export checks

Check captured recipes, source-specific output, cold acquisition, failure continuation, collision races, atomic publication and preservation of completed files. Distinguish GPU/scalar numerical correspondence from encoder serialization. [Output](colour-and-output.md#tiff-and-exr-serve-different-purposes) owns those contracts; [editor](editor.md#export-photographs) owns the composed interaction.

#### Roll analysis

Check source-footprint eligibility and forward sampling/fitting separately from review, confirmation and atomic acceptance. Exercise cancellation, failure and preservation of accepted state through the real interaction. [Roll analysis](roll-analysis.md) explains the model; [diagnostics](diagnostics.md#analyze-roll) covers supplied-photo exercises.

#### Adjustment layers and shared consumers

Check source/layer targeting and captured composition across consumers, alongside independent numerical expectations. A supported fixture alone does not establish complete-stack admission. [Processing](processing.md#adjustment-layers-change-controls-not-rendered-pixels) and [editor](editor.md#immutable-adjustment-layer-routing) own the contracts.

### Cancellation checks

Exercise before-first-poll, queued and running work at observed execution boundaries. Verify that owner retirement removes continuation rights while physical inputs survive cleanup and other consumers remain independent. Where Cancel retains feedback, verify acknowledgement. A stop after publication must preserve success; [architecture](architecture.md#cooperative-cancellation) explains the lifetime distinction.

### Shared editor UI checks

Use [shared presentation](editor.md#build-ui-from-shared-presentation) and check both component and composed feature. Fixtures must run production startup, bring controls into view before input, and execute admitted commands through the live application. Known chooser results are fixture inputs, not substitutes for controllers or workers. Supply explicit resource budgets/pressure rather than relying on the host's current state.

#### Framework UI regressions

Run affected `ui-tests` cases in both debug and release. The [harness](../src/editor/kittest_tests.rs) uses production UI and real GPU resources offscreen. Missing hardware is a verification failure/limit, never a reason to silently skip or pass. Device-dependent suites run locally before delivery because hosted CI cannot gate them.

Regenerate and inspect [guide screenshots](../assets/docs/README.md) for user-visible UI changes and [branding assets](../assets/branding/README.md) when their sources change. Offscreen captures do not verify native presentation or physical monitor appearance.

### Explicit native-host checks

The [native host](../tests/native_host.rs), [executor](../tests/native_executor.rs), [menus](../tests/native_menus.rs) and [termination](../tests/native_termination.rs) executables exercise real platform boundaries. Window launches require explicit `--run`, an unlocked desktop and authorization for application automation. Select the relevant scenario from the executable's usage/source; ordinary invocation does not launch it.

Require actual paints/completion and failure on premature closure or timeout. Fullscreen, focus, chooser, native input, accessibility and relaunch behavior need their corresponding real-use checks; kittest metadata is not a substitute.

### Specialist diagnostics

Ignored tests are opt-in for samples, hardware, windows or substantial resources. [Diagnostics](diagnostics.md) owns workloads, safety stops and interpretation. Authored resource observations live with [acquisition/execution](../src/gpu/view/tests/resource_observation.rs) and [native output](../src/gpu_export/native_resources.rs); sampled peaks and phase timings have limited coverage. Keep private inputs read-only and generated products ignored.

## Before handoff

Inspect the complete result, run proportionate checks and state what they establish and what remains unverified. Preserve meaningful tests and safety checks even when the design changes. A review supplements this evidence; neither green tests nor agreement with existing code establishes architectural soundness.
