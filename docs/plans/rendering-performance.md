# Responsive rendering and thumbnail caching

Status: user-authorized rebase and semantic integration onto main `0beee3e` (accepted-composition scopes) is complete and locally verified in `.worktrees/rendering-performance`, branch `impl/rendering-performance`. Prior geometry/native checks and logical local commits are recorded separately below. The user tested the earlier release build and reported improved responsiveness; broader hardware/release acceptance remains. Main delivery and push are not authorized.

## Goal and agreed scope

Prioritize responsiveness while scaling optional caches/prefetch/concurrency down under resource pressure. Typical rolls rarely exceed 36 photos. Preserve exact RGB16 source/development identity, pinned LibRaw/ACES, numerical gates, source-coordinate correction, cancellation and selected-image priority. No full-roll developed RAW cache, alternate development, approximate ACES, scopes/geometry implementation or schema changes.

Approved: tile-overlap-only rectangular averaging; shared immutable renderer resources; selected-Scan thumbnail seeding; protected visible/recent retention and obsolete-work cancellation; pressure-responsive byte budgets and bounded disposable disk RGB16 proxies; measured bounded submission overlap; higher-resolution switching-cache assessment. User added read-only initial-import/development phase investigation without development-policy changes.

## Concurrent work / integration boundaries

Inspected read-only; no other worktree's uncommitted files were copied, merged or rebased:

- Camera inversion is merged into main `5efb302`.
- Geometry is committed on main at `f8969c1` and is now the authorized integration base. Preserve affine presentation/averaging, `ViewRequest.frame`, source-coordinate correction and composition-on-paint thumbnails. Both averaging pipelines are shared; only rectangular views use restricted overlap dispatch. Existing standalone constructors remain.
- Accepted-composition scopes are now committed on main in `813d469`/`0beee3e`. Preserve parameter-keyed whole-proxy preparation, image-owned source/correction revision and accepted `FrameRaster` identity. Viewer navigation, thumbnail cache/scheduling and regional refinement must not substitute for scope coverage or lifecycle.
- Dark-red/roll-analysis: research/docs overlap, no identified production renderer overlap.
- Later roll-assistance worktree, base `5efb302`: inspected planning/research state; no implementation copied. Keep selected-image priority and existing Scan/source verification available to future analysis.

Dedicated `renderer`, memory/LRU/disk cache and probe modules reduce editor/viewer overlap. During native follow-up main advanced to committed geometry `f8969c1`; the user authorized integration. Tracked changes/status and committed geometry docs were backed up under `.cache/geometry-integration`; tracked/untracked performance work was stashed, the branch rebased onto `f8969c1`, and the stash reapplied (retained for recovery). Main's unrelated uncommitted documentation and other worktrees are untouched. Shared affine resources, presentation mapping reuse and crop-draft thumbnail suppression are reconciled and verified. Main subsequently advanced through documentation-only `a66e2ae`/`bbc9afd`, then accepted-composition scopes. The current authorized rebase incorporates those committed changes and plan closures; no other worktree's uncommitted implementation is imported.

## Steps and acceptance

- [x] Overlap bounds: fractional tile seams, conservative f32 boundaries, edge tiles and large outputs at unchanged tolerances.
- [x] Shared processing/both ACES targets/presentation/averaging; rendered-only images have no processing/source buffers; unchanged parameters/UVs reuse preparation.
- [x] Verified selected proxy seeding; byte-weighted caches; visible protection/recent retention; inactive-field eviction; obsolete-scroll/stale-generation regressions; complete warm 36-image forward/reverse scrolling.
- [x] Versioned/checksummed disk proxies; full-byte identity verification using resolved locators; atomic writes/pruning; corruption/unavailability as diagnostic misses, changed RAWs as errors; new-worker real-source disk reuse.
- [x] Phase/cache/development/cancellation timings; release import/prewarm and 1/2/3-depth contention probes. Retain depth 1 based on foreground latency, using specific-submission waits.
- [x] Focused editor/viewer checks, formatting, strict all-target editor Clippy, release build and diff inspection. One independent broad review performed; actionable findings fixed, no repeat review.
- [x] Contracts/architecture/runbook, compact authored evidence and higher-resolution/native-development assessment.

## Implementation and verified checkpoint

`GpuRenderer` owns immutable pipelines/tables behind one shared `RendererCache`, prewarmed in parallel with import. Sources/working/output/signatures remain image/job-owned. The selected verified Scan emits an exact uncorrected 256 proxy. Header verification hands its owned RAW handle into development; its fingerprint is taken before opening that handle so a pathname replacement cannot relabel older open data with newer bytes.

Normal optional thumbnail budgets follow machine RAM; warning quarters budgets/disables neighbors; critical zeroes optional budgets and cancels/suspends new thumbnail work. Visible GPU entries and the current selected proxy remain essential working sets; empty/failed views release source protection. One background pressure observer wakes idle UI only on transitions. Active gain resources are shared; inactive correction entries are discarded. Policy/exclusions are canonical in [the output contract](../contracts/output.md#responsiveness-and-optional-caches), not an aggregate OOM guarantee.

Persistent uncorrected RGB16 proxies are 128 MiB bounded, atomic/versioned/checksummed and independent of grade/target/correction/locator/document source ID. Full RAW bytes are verified before disk reuse; cold development also verifies after processing. View/grade edits reuse source work, whereas document/foreground/obsolete-scroll changes cancel phases. One outstanding thumbnail job avoids a cancellation backlog.

Independent review fixed the unresolved locator in post-development verification, unbudgeted retention of old correction maps and obsolete selected-source protection. Seed installation additionally rejects fingerprint/development mismatches (including temporary references). Final guards reject old failed-refinement revisions and react to pressure transitions without a one-second UI throttle.

Fresh final checks, local M5 Pro/48 GiB/macOS 26.6.2/Metal/Rust 1.99.0, offline Apple Clang:

- `./scripts/cargo.sh test --offline --features editor --lib --test viewer`: **96 library passed/4 ignored; all 5 viewer tests passed**, unchanged colour/reconstruction tolerances.
- `./scripts/cargo.sh clippy --offline --features editor --all-targets -- -D warnings`: passed.
- `./scripts/cargo.sh fmt --all -- --check`, `git diff --check`: passed.
- Release editor build: passed. Five camera-inversion and two full-f64 GPU film-response gates passed; the ignored release real-train RAW/scalar/GPU gate passed (174,592 samples, unchanged tolerance). Native probe compiled with strict warnings and confirmed the baseline full-RGB hash; changed Markdown local links passed.
- Ignored release import/depth, resolved relative-locator cold-thumbnail and prewarm/real-disk-restart probes: passed. Final restart measurement is after the guard/path fixes.
- Earlier release Save/Open onscreen smoke: passed; four sources completed, source/WB/model-4/target reproduction and worker isolation passed, 821 visible frames. This preceded final small guard/path polish; not a fresh physical-use acceptance.

Private full transcripts/products remain ignored under `.cache/`; selected timings, environment, commands and limitations are preserved in [evidence](../research/evidence/rendering-performance.txt). Baseline's 7 thumbnail tests and first checkpoint's 90 library tests are historical, not final counts. No sample edits or photographic outputs were promoted.

## Renderer/cache checkpoint outcomes and remaining choices

Initial registration costs about 0.13 s for four sources. Pinned full development dominates: approximately 0.5 s unpack plus 0.8–1.0 s processing. Renderer cold initialization was about 0.71 s; prewarming overlaps it, giving about 1.76–1.79 s registration-to-selected readiness in the measured trials rather than 2.26 s in the separate lazy trial. Selected seed reuse is approximately 0.2–1.2 ms. Verified real-source disk reuse after worker restart is about 37 ms, mostly the compulsory full-byte RAW hash, with no development. These are local trial snapshots, not a universal percent-speedup claim.

Depth 2/3 improves idle refinement only about 6%, while simulated foreground p95 rises from about 36 ms to 51–52 ms. Production stays depth 1; results were bitwise equal. Overlap bounds remove billions of rejected averaging threads at large outputs, not required per-source-pixel inversion/ACES.

**Higher-resolution switching cache:** a 1024×683 RGB16 CPU proxy is about 5.34 MiB; 36 cost about 192 MiB. Current source/working/linear GPU ownership costs about 26.7 MiB per target/proxy, approaching 1.9 GiB for both targets across 36. CPU-only recent 1024 proxies could be reasonable, with a few recent GPU images, without caching full RAWs. But cached pending preview must be distinct from a verified selected Scan/accepted selection: keep pick/refine/analysis unavailable until full identity/development succeeds, reject failures without changing state, and key geometry/correction/parameters correctly. Existing 1024 extraction/image creation is only a few ms; RAW development is the real switching bottleneck. No new switching ownership path is implemented; measure it together with the native follow-up before adding this tier.

**Historical native-development investigation (adopted source-built follow-up below):** pinned `Makefile.devel` builds reentrant LibRaw but leaves upstream OpenMP AHD disabled. A private build using the existing LLVM runtime and the same Apple Clang/LibRaw/bridge settings cut processing substantially. With four threads and a diagnostic bypass of the otherwise unused global sensor-statistics pass, four sources developed in roughly 0.73–0.79 s versus 1.36–1.41 s; hashes of every RGB16 code matched baseline. The statistics pass alone costs ~60–86 ms. [Durable probe/guide](../research/tools/raw-development.md); no runtime installs, shared dependency changes, diagnostic bypass or production build switch was made. Adoption needs an explicit packaging/dependency and bounded CPU/pressure choice, lazy diagnostics preserving immutable sensor/picker semantics, metadata/broader-source equality checks and foreground contention verification.

Actual OS pressure forcing, physical rapid-scroll/control use, macOS 14/broader hardware and physical-monitor calibration remain unverified. Headless cache/Metal and visible-window checks do not establish those gates.

## Additional potential bottleneck: import fingerprinting

User flagged full-byte RAW fingerprinting during import as another potential bottleneck. Current four-source registration is ~0.13 s and individual hashes cost ~26–36 ms; the selected source is also hashed before opening/development and verified again afterward. Larger imports, colder storage and repeated reads could make this more visible. Investigate hash/read counts, repeated import/selected preparation, cold versus warm filesystem cache and UI blocking separately from LibRaw processing. Preserve full-byte identity and replacement/development race protection; this note does not authorize timestamp-only identity, skipped verification or a fingerprint-cache implementation.

## Approved native follow-up checkpoint

User approved pinned local OpenMP, bounded CPU concurrency and lazy diagnostics, then permitted the source download. The installed Homebrew runtime requires macOS 26 and is not a production input. LLVM 22.1.8 release archive was downloaded into this worktree's private `.cache/native-deps`, SHA256-verified, and only OpenMP/common CMake/license source extracted. Source-built arm64 runtime targets macOS 14 with `@rpath` identity; pinned LibRaw is rebuilt separately there. Shared `.cache/deps` and other worktrees are unchanged.

- [x] Private source-built runtime/LibRaw; durable checksum-pinned scripts, no global installs.
- [x] Native per-call OpenMP ICV scopes/restore; at most available CPUs minus two (floor one), caps 8 foreground/2 background, warning 2/1, critical 1/1. Scoped zero blocktime makes idle workers sleep. No process environment mutation.
- [x] Decode once independently of optional global statistics; explicit `unpack()` retains full-statistics API/cache. Regional picks still read immutable original CFA/RAW/black/white/margins after development and do not invalidate camera pixels.
- [x] Complete-source comparisons on four sources at 1/2/4/8 plus a fifth train source/two WB references at 2/8: identical full RGB16, metadata/pre-multipliers, dimensions/maxima/warnings and full/region statistics. Native probe verifies ICV restoration; unchanged real-train numerical gate and new lazy/failed-pick/cached-pointer regressions pass.
- [x] Normal-pressure idle prefetch visits unshown photos after visible/nearby work; requires spare GPU budget and never evicts retained work to add optional entries. No-touch membership avoids LRU churn. Missing-visible or resumed-foreground work cancels offscreen prefetch; warning/critical disable it. Headless far-end/preemption/backlog and cache-admission regressions pass.

Fresh native checkpoint: **99 editor library passed/6 ignored + 5 viewer/Metal passed**, raw-only library **16 passed/1 ignored**; strict all-target editor Clippy, release build and formatting/diff checks passed. Explicit ignored lazy RAW, native contention, prewarm/disk restart and real-train numerical probes passed. Fresh release onscreen Save/Open smoke passed all-four worker isolation, model4/custom-WB/working+sRGB+P3 reproduction and RAW-error atomicity with **686 visible frames**. Runtime and editor Mach-O minimum 14.0, local rpath and runtime's sole libSystem dependency inspected; actual macOS14 execution remains unverified.

Native-only independent review found an inherited runtime override could select the macOS26 Homebrew library (override removed) and far prefetch could survive resumed foreground work (idle admission/cancellation predicate shared and regression added). This reviewed the new native/prefetch slice, not a repeat review of earlier renderer/cache code; no second pass after fixes.

Source-built native development is ~0.65–0.68 s at 8 threads vs ~1.34–1.44 s old eager serial snapshots. Four-source registration-to-selected readiness measured **0.903 s** vs earlier **1.758 s** prewarm snapshot; disk restart **32 ms**, dominated by full-byte verification. Background 2 threads kept simulated foreground p95 ~28 ms; 4/8 raised it to ~31 ms, supporting the bounded priority split. See [canonical evidence](../research/evidence/rendering-performance.txt) for all source/hash fields, exact commands, timing/physical limits. [Accepted native decision](../decisions/parallel-libraw.md), [probe/guide](../research/tools/raw-development.md).

## Combined geometry integration checkpoint

This combined checkpoint ran on geometry base `f8969c1` before local performance commits. Both rectangular and affine averaging pipelines are immutable shared renderer resources; sums/dimensions/affine parameters remain job-owned. Rectangular views alone use restricted dispatch. The affine shader, uniform layout and full-output coverage dispatch remain unchanged. Presentation caches key all six affine mapping components, preserving turns/flips/straightening and out-of-source discard. Thumbnail sources/textures stay geometry-independent with composition on paint; crop drafts cancel offscreen prefetch and suspend new idle jobs. Geometry's source-coordinate Scan/picker/export behavior remains intact.

Fresh local checks (same M5 Pro/48 GiB/macOS 26.6.2/Rust 1.99.0/Metal environment; offline Apple Clang):

- Editor library **144 passed/7 ignored**; viewer **7**, affine reconstruction **1**, geometry export/reopen **3**, camera inversion **5** and full-f64 GPU response **2** passed. This includes warm 36-image scrolling, pressure/stale-result/source-identity tests, orientation/crop/picker behavior and a new crop-draft prefetch regression. Rotated/source-corrected viewer tests now use shared images across targets.
- New explicit ignored release `corrected_affine_refinement_matches_scalar_coverage` passed: real developed RAW, two straightening angles, roll turn/flips, source-coordinate correction, padded tile seams, both targets, rendered-only ownership, cancellation and inconsistent-frame rejection. Worst absolute difference from independent scalar coverage/ACES was **2.894114e-7**, unchanged bound **2e-5**.
- Explicit ignored release prewarm/disk-restart, lazy full/region/cached-development diagnostics and real-train numerical gates passed. Train remains **174,592 samples**, worst unchanged-tolerance multiple **2.771745091e-4**, bound 1. Native complete-source comparisons above were not rerun: the native bridge/dependency inputs are unchanged by integration.
- Strict all-target editor Clippy, release editor build, formatting, shell syntax and diff checks passed. Final integration code diff inspected; local Markdown links checked.
- Fresh release onscreen Save/Open smoke passed four-source isolation/completion, model4/custom-WB/working+sRGB+P3 reproduction, changed/missing/incompatible RAW atomicity and live settled preview: **633 visible frames**. Rotated reconstruction is covered by numerical checks, not photographic or physical-gesture acceptance.

Private transcripts are under `.cache/geometry-integration/`; selected results/reproduction are in [evidence](../research/evidence/rendering-performance.txt). The first new crop regression needed the fixture's native floor initialized; strict Clippy caught two redundant conversions in the new probe. Both test-only defects were fixed, and checks passed afterward. No unresolved integration failure remains. Actual OS pressure, broader cameras/devices, macOS14 execution, physical input/monitor and bundled runtime/licensing/codesigning acceptance remain separate.

## Local commit checkpoint

The user authorized committing the current work without additional integration or features. Original logical implementation commits before the current main rebase (historical hashes, now rewritten):

- `8f6cfd2` — bounded OpenMP RAW development and lazy sensor diagnostics, including shared OS pressure sampling and native compatibility probe.
- `c9fd35c` — shared GPU resources, rectangular/affine integration, responsive/persistent thumbnail caches, idle prefetch and regressions.
- Contracts, accepted native rationale, build/probe guides and selected evidence are packaged in the following documentation commit.

Before committing, fresh raw-feature library checks passed (**29 passed/1 ignored**); combined editor/GPU/export checks again passed with the counts above, as did strict all-target editor Clippy and formatting/diff checks. Documentation-only packaging does not change the tested implementation. No additional onscreen smoke, complete native comparison or hardware acceptance was claimed from this commit step. Main's dirty docs and other worktrees remain untouched; no push.

## Accepted-composition scopes integration checkpoint

The user explicitly requested semantic integration, including cleanly merged code. The integration retains image-owned source revision/signature alongside the shared renderer, one whole-source preparation entry point for either callback order, scope-owned bins/readback/cache keys and canonical accepted whole-frame coverage. Thumbnail and refinement storage/parameters remain independent. Scope pipelines/bins are already one persistent editor resource, so no speculative extra sharing layer is added.

Main's whole-proxy viewport/composition/crop-suspension regression now uses shared images in both targets. The scope preparation/invalidation regression uses different images backed by the same renderer. A new both-target/callback-order regression verifies corrected foreground scopes survive thumbnail/tile processing with different source/matrix/parameters, invalid writes preserve identity, valid writes trigger exactly one new reduction, and viewer cropping does not narrow scope samples. The real-RAW prewarm/disk-restart probe now additionally checks whole-frame scope preparation/diagnostics during thumbnail work.

Fresh checks on the same local M5 Pro/48 GiB/macOS 26.6.2/Rust 1.99.0/Metal environment, offline Apple Clang:

- **158 library passed/7 ignored**, scope GPU **9 passed/1 ignored**, viewer **7**, affine reconstruction **1**, geometry export **3**, camera inversion **5** and full-f64 GPU response **2** passed. Existing accepted-composition, retained-trace, readback coalescing, crop/reference/source suspension and layout/Save tests remain intact. New shared-renderer identity/ownership tests pass without weakened tolerances.
- Explicit ignored release prewarm/scope/thumbnail restart passed: **699,392 valid scope samples, zero invalid, one reduction** before/after background work, with no selected-image reprocessing. Registration-to-selected readiness was **0.947191 s**; disk reuse **0.034504 s**, mostly compulsory full-byte verification. Single local trials, not statistical speedup gates.
- Explicit ignored release corrected affine RAW refinement and real-train numerical gates passed unchanged (**2.894114e-7** worst affine absolute error, bound **2e-5**; train **174,592** samples, tolerance multiple **2.771745091e-4**, bound 1). Native implementation/dependency inputs are unchanged by rebase; complete native-source comparisons were not rerun.
- Strict all-target editor Clippy, formatting/diff/shell checks, release build and changed-document file links passed. A single independent review of the new semantic integration delta found no actionable defect; original native/cache implementations were not reviewed again.
- Fresh release combined onscreen scopes/controls/Save/Open smoke passed, **1,592 visible frames**. Scope modes/amplification, normal/narrow Roll/Photo layout holds, full-proxy diagnostics (**699,392 samples, 8 reductions, zero invalid**), settled refinement, four-source thumbnail/selected isolation, model4/custom-WB/working+sRGB+P3 reproduction and failed RAW Open atomicity passed. This is scripted lifecycle/presentation, not photographic guide appearance, physical gestures or monitor acceptance.

Rebased code commits are `f3109a2` (native) and `8584163` (renderer/cache plus scope integration/tests); the documentation commit follows. Architecture/contracts preserve accepted scope semantics; links follow main's closed camera/geometry/scope plans. Durable selected results/reproduction are in [evidence](../research/evidence/rendering-performance.txt); private transcripts/range-diff/backups are under `.cache/scopes-integration`. Main's dirty-document patch was checked unchanged. No unresolved integration failure or push; broader resource/hardware/macOS14/distribution/photographic acceptance remains open.

## Exact next action

Continue user release-build feedback and await separate main-delivery authorization. The branch is based on main's committed accepted-composition scopes; preserve main's unrelated dirty docs at delivery. Import fingerprinting, larger switching caches and full-source scope analysis remain follow-ups, not implementation. Global installs, sample edits, pushes and other-worktree changes are not authorized.
