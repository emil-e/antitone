# Making and checking a change

Use [the README](../README.md) to build and run. If you are new to an area, read its [topic guide](README.md#understand-the-system), then follow the code links. The guides explain the model; types, implementation comments and tests define its mechanics.

## Everyday checks

The default build is the numerical core, without native libraries or the desktop stack:

```sh
cargo build --locked
cargo fmt --all -- --check
cargo test --locked --no-default-features
```

Cargo uses the committed lockfile. Add `--offline` when caches are populated and downloads are undesirable; omit it for a first build.

## Feature-specific checks

Choose checks for the boundary you changed rather than running every native/sample-dependent exercise.

```sh
# Pixel processing and photographic grading; no native libraries or Metal.
cargo test --locked --no-default-features \
  --test camera_inversion --test processing --test film_response --test photographic

# Persistence; no native libraries or Metal.
cargo test --locked --features project \
  --test camera_project --test project --test film_project --test flat_field_project

# Authored DNG plumbing; native libraries, no private photographs or desktop.
cargo test --locked --features raw,export,project --test raw_dng --test raw_flat_field --test raw_roll

# Authored import/correction/Analyze/JPEG/cache workers; native setup and Metal, no desktop window.
cargo test --locked --features editor --lib authored_ -- --test-threads=1

# Production GPU math; requires Metal, not native RAW/rendering libraries.
cargo test --locked --features gpu --test gpu_processing --test gpu_film_response

# Composed editor, scopes and viewer; native setup and Metal required.
cargo test --locked --features editor --lib --test scopes --test viewer
```

The [authored RAW fixtures](../tests/fixtures/raw/README.md) are repository data, not private camera samples. They cover RAW/Scan/export, measured inversion anchors, sensor-clipping rejection, LibRaw matrix/flooring gates, correction/map persistence, roll sampling and named Editor/JPEG worker flows. They do not establish native-dialog interaction, general composed-UI behavior or photographic quality. Tests read them directly and use temporary export/project outputs; they never generate or download their inputs. The fixture README explains the patch expectations and deterministic maintenance recipe.

The opt-in [ImageIO export check](diagnostics.md#export) adds independent TIFF storage/profile reopening without private files or a prebuilt helper. It compiles the native checker into temporary storage; ordinary tests leave it ignored.

The photographic tests use an independent scalar oracle in [test support](../tests/support/photographic_reference.rs). Numerical gates live in the tests; do not loosen them merely to make a new implementation agree. A missing Metal device is a verification limit, not a numerical pass.

After native setup, use broader integration/lints when the change warrants them:

```sh
cargo test --locked --all-features
cargo clippy --locked --all-features --all-targets -- -D warnings
```

## Roll analysis

[Analysis](roll-analysis.md) spans sampling, numerical estimation and editor installation. Check the layer you changed:

```sh
cargo test --locked --no-default-features --test roll_analysis --test roll_sampling
cargo test --locked --lib roll_analysis::
cargo test --locked --features editor --lib editor::analysis::tests -- --test-threads=1
```

The first two exercise synthetic populations and curve fitting. Editor checks additionally cover snapshots, confirmation, cancellation and atomic full-grade reset.

To check the actual picture-area and Analyze controls with authored RAWs, use the [framework prerequisites](#framework-ui-regressions) and run:

```sh
cargo test --locked --features ui-tests --lib editor::kittest_tests::analyze -- --test-threads=1
```

These tests cover Apply/Cancel, reset confirmation, sparse-area failure and settled sRGB/P3 rendering, with and without correction. Cancellation holds reply delivery until the Cancel widget is clicked, without replacing or stalling the worker. Optional captures are `analyze-{roll,corrected}-{srgb,p3}-offscreen.png`. [Real-RAW diagnostics](diagnostics.md#analyze-roll) remain separate.

## Shared GPU queue checks

```sh
cargo test --locked --features ui --lib platform::submission
cargo test --locked --features editor --lib refinement_worker_uses_the_submission_gate_supplied_to_the_host
```

These headless Metal checks verify submission exclusion during surface configuration and the editor worker's use of the host's gate. Use the [native presentation stress route](diagnostics.md#macos-presentation) for real surface reconfiguration alongside background GPU work.

## Shared editor UI checks

Read [shared presentation](editor.md#build-ui-from-shared-presentation) before adding a widget. Check the component **and** its composed consumer: paint equivalence alone does not verify state or input ownership.

App-level fixtures must run production startup before injecting events and bring controls into view before clicking; painted text bounds alone do not establish an interactive target. Resource-policy tests supply explicit budgets/pressure rather than depending on the host's current pressure.

```sh
cargo test --locked --features editor --lib editor::presentation
cargo test --locked --features editor --lib editor::activity::tests
cargo test --locked --features editor --lib editor::controls::tests
cargo test --locked --features editor --lib editor::tests::viewer
cargo test --locked --features editor --lib editor::tests::workspace
cargo test --locked --features editor --lib editor::thumbnails
cargo clippy --locked --features editor --lib --tests -- -D warnings
```

For wordmark/icon changes, also [regenerate the branding assets](../assets/branding/README.md#regenerate) and inspect the SVG/PNG and native icon results. Presentation tests cover the header’s fixed logical size and menu focus at native 1×/2× density; workspace tests cover its composed availability and unchanged processing state. A local bundle launch is separate from release/device acceptance.

Typography checks cover reading/instrument family separation, retained glyph fallbacks and equal-width digits, shared action measurement and tab label padding. Parameter tests exercise precision and disabled edits with the production reading font. The framework `typography_keeps_tabs_readouts_and_tool_actions_coherent_at_both_densities` case checks real startup, stage activation, tool actions and unchanged project/source state at normal/narrow 1×/2× sizes; its optional captures use the `typography-` prefix.

For feature-specific transactions, use the `editor::analysis::tests`, `editor::flat::tests`, `editor::files::tests`, `editor::navigation::tests`, `editor::preferences::tests` or `editor::scopes::tests` filter. Use [layout and GPU checks](diagnostics.md#editor-and-layout-checks) when painting/layout changes.

The presentation dialog suite checks neutral versus explicit warning semantics, text/action spacing, normal/narrow layout, safe initial focus, explicit choices, blocked backdrop clicks and popup-first Escape. Files and analysis tests cover decision cancellation/acceptance, staged destinations/continuations, unchanged accepted state and stale confirmation rejection. `editor::feedback::tests` covers typed worker/send failures, persistent acknowledgement, matching resolution and pending save decisions. Component feedback tests cover all severity cues, selectable/bounded technical details, disclosure focus and isolation from inspector preferences. Working-component tests check title/bar-only normal/narrow framing, percentage-free indeterminate animation, supplied file counts, centred viewer-spinner geometry, explicit cancellation and blocked modal input. Document tests use authored RAWs to check completed-file callbacks, duplicate reads and failed-file boundaries. Activity tests check typed request scopes (including reused numeric IDs and distinct measurement kinds), stale/repeated completion, counted/indeterminate/cancelling transitions, unsupported capabilities, superseded Analyze results/actions, immediate send failures, failure priority, unchanged viewer geometry and Analyze cancellation from every stage.

The thumbnail suite includes composed cold-load/error selection, JPEG placeholder semantics, disabled focus/input and ordered-roll picker return checks. [Wheel accessibility](plans/inspection-and-interaction.md#focused-fixes) remains unsettled. Physical pointer/trackpad behavior, VoiceOver and preference relaunch also need real-use checks; headless events cannot establish them.

### Framework UI regressions

The optional `ui-tests` feature pins egui_kittest to the application’s egui version. No eframe host or snapshot baselines are enabled. Run the interaction tests in both profiles:

```sh
cargo test --locked --features ui-tests --lib editor::kittest_tests -- --test-threads=1
cargo test --locked --release --features ui-tests --lib editor::kittest_tests -- --test-threads=1
cargo clippy --locked --features ui-tests --lib --tests -- -D warnings
```

`parameter_reset_click_respects_disabled_state` finds Reset Contrast by its accessibility label, clicks it and checks disabled input. `dmin_picker_cancels_or_applies_measurement_and_paints_source` requires the documented native setup and a **Metal device**, but no desktop/window. It imports the committed patch DNG, scrolls to the real Dmin picker, checks disabled Apply and unchanged picker cancellation before dispatch, then selects a source-coordinate patch and applies through the real worker. Worker waits have a 60-second deadline; GPU readback waits are bounded. Missing Metal fails rather than silently skipping.

A test-only renderer shares the Editor’s device/queue and submission gate, and paints the production selected-image callback into RGBA16Float. Finite readback and two neutral patches are checked against the CPU source-rendering reference at `0.001`; this is implementation agreement, not a new independent ACES oracle. For changes to rendering or presentation, use the independent OCIO/Apple checks for the affected boundary; matching implementations alone cannot validate that boundary. Preferences are never restored. Most scenarios disable thumbnail scheduling; project/correction tests run the production thumbnail worker with disk storage disabled.

`stage_tabs_preserve_project_and_fit_normal_and_narrow_layouts` clicks all three stage tabs at 1100×800/315-point sidebar and 800×600/270-point sidebar sizes. It checks selected-tab semantics, tab/header bounds, stage-specific contents, unchanged project/source selection, settled preview and completed scope diagnostics before rendering all six states. `unfinished_editor_operation_fails_at_wait_deadline` checks that unfinished work fails at the wait deadline rather than passing.

`decision_dialogs_expose_safe_actions_and_fit_normal_and_narrow_viewports` uses the authored DNG and the same float renderer to inspect all four prompts at normal/narrow sizes. It checks each controller's neutral/warning classification, enabled semantic actions, popup-first Escape in the composed viewer and unchanged project/selection after Cancel. Use its filter with `ANTITONE_UI_CAPTURE` to write `dialog-{save,project-replace,output-replace,analyze}-{normal,narrow}-offscreen.png`.

`operation_failures_are_acknowledged_without_promoting_failed_candidates` uses the authored DNG to exercise missing-reference and rejected correction workers, support validation, failed save and export-destination rejection. It checks unchanged project, selection, dirty state and saved bytes after acknowledgement. With the capture environment it writes `feedback-{reference,support,correction,save,export}-{normal,narrow}-offscreen.png` and expanded save-detail captures.

The `editor::kittest_tests::workflow` filter covers measured Dmin/Dmax, numeric gamma/exposure/WB edits, positive and +2 EV source rendering in both targets, native-pixel settled refinement and explicit picker-source switching/Cancel return. It reuses the patch and 512² roll fixtures with temporary copies; no larger fixture or private photograph is needed. Scope widgets exercise RGB/Luma/vectorscope, 1×/2×/4×/8× amplification and both targets, requiring zero invalid samples, the exact whole-frame sample count and at least three completed GPU reductions. Source brightness, picker cancellation and scope controls preserve the project except for intentional target edits.

The `editor::kittest_tests::project` filter covers actual Save/New menu actions, Save/Discard/Cancel protection, high-precision Film/classic response and Photo settings, stable IDs/order/unvisited defaults, saved versus automatic WB, exact working/sRGB/P3 pixel reopening and rejected ordered-source Open with unchanged document/display. A failed Save blocks New. The `editor::kittest_tests::correction` filter covers real support move/resize/Apply/Cancel, corrected Dmin/Dmax/Gamma/WB exactly matching full-resolution CPU measurements, both settled targets and thumbnail readiness, +2 EV native-pixel inspection, Enabled/Clear, exact map reopening and missing-reference preservation. Known Save destinations are fixture setup; Open and initial reference selection supply chooser results to the real guards/workers. Native file dialogs are not automated.

`working_feedback_gallery_preserves_available_interactions_and_exposes_every_style` imports the authored DNG, then holds explicit UI working states for repeatable normal/narrow inspection. It checks disabled document input, status-bar-only export with retained stage/canvas interaction, pending versus completed Analyze cancellation while another stage is selected and unchanged accepted project state. These are composed presentation fixtures, not duration measurements or proof of backend cancellation. Their supplied counts are layout fixtures; completed-file reporting is checked separately through real authored RAW reads. With `ANTITONE_UI_CAPTURE` and the `editor::kittest_tests::activity` filter it writes `working-{case}-{normal,narrow}-offscreen.png` for counted/final-preparation document work, export, RAW/reference/measurement, Analyze sampling/fitting/pending and completed cancellation, and quiet background presentation.

For optional named inspection artifacts:

```sh
ANTITONE_UI_CAPTURE="$PWD/.cache/ui-captures" \
  cargo test --locked --features ui-tests --lib dmin_picker_cancels_or_applies \
  -- --test-threads=1
```

Use the `stage_tabs_preserve_project` filter with the same capture environment to write `{roll,photo,rendering}-{normal,narrow}-offscreen.png`. The `editor::kittest_tests::tools` filter adds `{crop,dmin,wb,flat}-{normal,narrow}-offscreen.png`: real crop/angle and patch input, disabled Apply, unchanged Cancel and accepted Apply through the actual workers. Flat-field reference-path selection is fixture setup, not native file-dialog automation; the test checks proposed support/Fit bounds, settled reference previews and widget Apply/Cancel. These and `authored-dmin-offscreen.png` are clipped/quantized from float captures for inspection only. The workflow filter also writes `workflow-{positive,source}-{srgb,p3}-offscreen.png`, `workflow-native-p3-offscreen.png` and `scope-{srgb,p3}-{rgb,luma,vector}-offscreen.png`. Successful generation does not approve appearance or establish native surface/ColorSync presentation, physical input, dialogs, or other platforms. Project tests add `project-film-{srgb,p3}-offscreen.png`; correction adds `correction-positive-{srgb,p3}-offscreen.png`, `correction-source-native-offscreen.png` and `correction-edit-narrow-offscreen.png`. The 600×600 correction inspection viewport exists to exercise a cropped native-pixel region in the small fixture, not to claim native-shell layout acceptance. Broader native/physical acceptance remains separate.

The `editor::kittest_tests::fullscreen` filter checks photo-only composition at normal/narrow native 1×/2× density, source-EV precision, suspended drafts, workspace restoration and text/popover shortcut precedence. It also checks zoom-group placement, auto-hide/reveal, unchanged canvas input, editing and keyboard-focus retention. `editor::presentation::viewer` checks the shared overlay's hover/focus/disabled/hitbox contract, including taller activity content. `editor::fullscreen::tests` covers host restoration acknowledgement, OS-adjusted bounds, pre-existing native fullscreen, stale source identity and blocking transactions. Supplied viewport metadata and host acknowledgement are fixtures, not a real window round trip; [remaining acceptance](plans/inspection-and-interaction.md#layout-and-controls) is separate.

### Explicit native-host checks

The separate [native-host test executable](../tests/native_host.rs) shares the production host, not Editor state or kittest. Lifecycle cases need macOS/Metal and an available desktop, but no RAWs/native RAW libraries. The `editor` case additionally needs normal native setup and the committed patch DNG; it opens a temporary authored project through the public Editor constructor, with temporary HOME/cache storage and no restored personal preferences. Ordinary test invocation runs software result-contract checks without opening a window, including under `--all-features`. Only explicit `--run` launches it:

```sh
cargo test --locked --features ui --test native_host -- --run lifecycle
cargo test --locked --release --features ui --test native_host -- --run lifecycle
# Current-Space full-display entry/paint/return/paint; requires an unlocked desktop.
cargo test --locked --features ui --test native_host -- --run fullscreen
# Actual Editor composition and selected-image surface passes; authored inputs only.
cargo test --locked --features editor --test native_host -- --run editor
cargo test --locked --release --features editor --test native_host -- --run editor
# Negative cases: all must exit nonzero with the corresponding failure.
cargo test --locked --features ui --test native_host -- --run early-close
cargo test --locked --features ui --test native_host -- --run timeout
cargo test --locked --features ui --test native_host -- --run process-timeout
```

Lifecycle requests focus and four native sizes, waits for each real surface paint, requires submitted/presented frames and explicit completion, then closes through the host. Premature closure and a 10-second UI deadline fail; a 20-second process watchdog also bounds initialization/event-loop hangs. The timeout case uses a 200-ms UI deadline; process-timeout shortens the process watchdog to 200 ms while leaving UI completion pending. Zero-frame/incomplete-paint rejection is also exercised by software result checks. Hardware errors are failures, not skips after a requested launch.

The fullscreen case requests the host's photo-only presentation rather than native Space fullscreen. It requires a focused borderless window covering the current display's complete frame, remaining on its active Space with the native fullscreen flag off, auto-hide menu/Dock options and actual surface painting. Return must restore original outer bounds and presentation options and paint again before completion. These gates require an unlocked desktop; do not substitute kittest metadata or relax them. Native focus-loss/active-close, pre-existing native fullscreen and broader display/input acceptance remain in the [interaction follow-up](plans/inspection-and-interaction.md#layout-and-controls).

The authored Editor case additionally waits for a selected-image callback in the canvas at each native size and places a paint acknowledgment in that same viewport, requiring the real surface pass and presented frames before advancing. This covers basic Editor/host composition, not per-tool appearance, pointer/keyboard event forwarding, native dialogs, physical gestures or monitor matching. Native input automation is deferred; no event-injection adapter is selected. Existing [presentation references and resize/submission stress](diagnostics.md#macos-presentation) and [physical interaction follow-up](plans/inspection-and-interaction.md) remain separate.

## Specialist diagnostics

Most ordinary tests need no private RAWs. Tests marked ignored are opt-in because they need samples, hardware, visible windows or substantial resources. Their presence does not make every one a required gate; choose a check for its asserted boundary, not a name or successful report. [Diagnostics](diagnostics.md) explains how to run them safely and interpret their results.

Keep local photographs and generated images/logs in ignored storage. Automated agreement establishes a tested software boundary, not photographic quality, physical monitor matching or broad device support. [The roadmap](../PLAN.md) tracks remaining acceptance.
