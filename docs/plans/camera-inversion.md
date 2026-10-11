# Native-camera inversion replacement

Status: implementation integrated onto finalized main `c0b664a`; post-rebase numerical/source verification and release rebuild pass. Camera-model onscreen/photographic review remains separate. The user approved replacing AP0 inversion outright, with no old-grade compatibility/migration. AP0 stays at EXR interchange and inside unchanged ACES rendering. Post-inversion camera metadata matrix → AP1 is the approved pragmatic mapping, not a claim of recovered scene radiometry.

## Baseline and scope

Worktree `.worktrees/camera-inversion`, branch `impl/camera-inversion`, current base `c0b664a` (finalized flat-field main). The original baseline was `ace5628` plus forty-six copied uncommitted flat-field files; `.pi/` excluded. `.cache/flat-field-baseline/manifest.json` and copied files identify it. The user requested rebase/verification after flat-field landed. Only camera deltas were three-way transplanted; no camera commits existed to replay, so the branch base advanced without creating a commit. The user authorized a local camera commit on top of this main base. Main itself remains unchanged; no push authorized.

Replace CPU/GPU sampling, calibration, response, fitting and source bypass with one camera-RGB inversion path; map positives to AP1 after inversion. Keep the existing response controls/equations, signed/HDR data, compact RGB16 source, photographic Bradford WB and output rendering/encoding. Flat-field remains pre-inversion camera-RGB gain with the same physical neutral/centre-luminance target. Old project schema/contracts must reject explicitly, not reinterpret their RGB or matrix values. No PCA transform or legacy AP0 processing branch.

## Steps and acceptance

1. [x] Scalar camera→AP1 metadata matrix, native calibration/Scan/fitting and code floor; unchanged physical flat-field neutral target through AP1/XYZ.
2. [x] One precise GPU response/source path, matrix after inversion, no retained wide source or legacy AP0/f32 branch. Range bounds cover negative continuation and each real post-transform intermediate; tolerances unchanged.
3. [x] Editor/thumbnails/refinement/export and schema/model/development metadata. Explicit response; old AP0 projects reject. Retained roll camera-model/matrix basis rejects mismatched sources and transient references; exact persisted base floors and one shared corrected bound.
4. [x] Scalar/project/Metal/target/editor suite, real train and real flat-field Save/Open/corrected TIFF/AP0 EXR regressions, strict all-target Clippy, formatting and release editor build. Contracts/architecture/ADRs updated; historical AP0 tools are not current gates.
5. [x] Implementation-only diff checked against the snapshot; independent review performed once and both findings fixed with targeted regressions. [Compact verification snapshot/commands/limits](../research/evidence/camera-inversion.txt).

## Handoff

Camera implementation is packaged as a local commit on this branch, directly on the finalized main base. `.cache/rebase-main-c0b664a/` preserves the pre-transplant working files, verified hashes, three-way inputs and comparison manifest. `.cache/camera-implementation.patch` and file list now compare against current base `c0b664a`; they are disposable artifacts, not authored source/docs. The copied flat-field foundation is now tracked in main and no longer appears as camera work.

Main's only post-snapshot code delta was `src/editor_flat_field_smoke.rs` (settled native inspection and persisted support rectangle); it is preserved byte-for-byte. Four documentation overlaps were resolved while retaining main's accepted/completed flat-field plan and acceptance evidence. Its plan stays under `completed/`, not resurrected as active work. Post-rebase tests, strict Clippy, release real-RAW regressions, rebuild, format/whitespace and documentation links pass; [fresh snapshot](../research/evidence/camera-inversion.txt).

The review's basis/floor findings are resolved: `CameraBasis` survives source removal and prevents cross-camera calibration reuse; altered stored base floors reject. Runtime correction uses the exact shared bound, avoiding path-dependent floating-point rescaling and accidental repeat amplification. The pre-existing Original unit-response extra-field serde limitation is reported in the project contract, not expanded into unrelated cleanup.

Next concrete action: try the rebuilt `target/release/antitone` for camera-model photographic/onscreen feedback, then address concrete findings only if requested. No further commit/push or desktop automation is implied. The rebase did not rerun onscreen smoke or expensive full-size exports; headless corrected Save/Open/small exports and the real train GPU regression were freshly rerun. No calibrated stock/scene-colour or physical-monitor claim is made.
