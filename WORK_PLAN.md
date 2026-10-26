# Digital source brightness normalization

## Agreed scope

New Digital photographs should have roughly camera-preview brightness with manual Roll/Photo Exposure at zero. Retain Antitone colour rendering and capture-exposure differences. Share preview extraction/decode/in-flight work; prepare selected sources, navigation proxies and cold exports lazily, without paint-time IO or eager batch development.

Latest user direction: fixed Digital normalization belongs in the AP1 source received by tools, before dust, not in photographic grading. Roll exposure/WB placement and visibility remain unchanged. Broader calibration disentanglement is outside this revision.

Integration now authorized: checkpoint/rebase, proportionate fixes/verification/review, push task branch, create PR and watch CI. No merge or direct main push. No new downloads/installations or saved-project compatibility/migrations. Private originals remain read-only; preserve unrelated main-checkout work.

## Current implementation and acceptance

Implemented in .worktrees/digital-preview-exposure, branch feat/digital-preview-exposure, base 9c63d1e:

- Fixed gain is fused into camera-to-AP1 production matrices, before repairs and photographic grading.
- Native buffers, physical camera matrices, estimator sampling, Negative processing and camera inspection retain their own scale.
- Removed PreparedInput and starting-exposure handling from grading APIs and dust. Shared processing owns source signatures and repaired-range preflight; normalized source matrices participate in identity/bounds.
- CPU repair sampling, GPU working/navigation/pending views, original-resolution reference/precision reconstruction and TIFF/EXR use the normalized source transform.
- Shared lazy preparation, canonical restored defaults, optional derived persistence and cache-fork/in-flight reuse remain.
- Manual Roll/Photo controls, dirty state and no-re-metering behavior unchanged.

## Verification

Current receipts: .cache/initializer-state.md; detailed commands in .cache/ap1-*.log.

- Affected library scope: 1098 passed / 29 ignored in debug and release; excludes scan_regions and composed UI.
- 50 ordinary integration targets: 317 passed / 9 ignored.
- Composed UI debug and release: 105 passed; same pre-existing correction Save/Open failure remains.
- Core: 56 passed. Minimal feature builds, production release editor, all-target/all-feature Clippy, pinned source formatting and documentation checks pass.
- Dust quality: smooth/repeating texture, signed/HDR values, gains 1/256–256, automatic/manual donors; unchanged outside-mask bits and clean-content expectations.
- Actual sensor-defect fixture verifies normalized AP1 repair inputs, unchanged native pixels and identical Negative repair results. Cold repaired TIFF/AP0 EXR match independent expected brightness.
- Nonzero-reference selected/navigation pixels, original-reference toggle and WB precision reconstruction pass against independent OCIO expectations. Small-gain CPU/GPU and unequal-channel headroom controls pass.
- Six private reference photographs still match; production-transform comparison panels are byte-identical to pre-revision panels. Preparation 1.89–2.44ms; repeat 0.83–1.42µs. No extra full-image pass.

Independent normalization review completed. Its two findings (original reconstruction matrix and small-gain matrix validation) were fixed and covered. Review scope/limits: .cache/ap1-independent-review.md. No repeat review solely for those fixes.

## Remaining / next actions

Implementation is ready for user assessment; no further routine implementation is planned in this revision.

Integration: rebased onto fetched main 6cd71b3 (ordered adjustment layers, PR65). Ported normalized source transforms, neutral identity, preflight, original reconstruction and tests to the new layer APIs. Schema12 / processing model5; no migration. Focused integrated exposure/normalization controls and all-feature test-target compilation pass. Remaining: refresh full integrated checks, fresh-context semantic/test-quality review, remove this plan after transferring lasting knowledge, push/open PR and watch CI.

Existing limits: whole-library debug run previously hit 900s in framing-search tests; do not claim a complete unfiltered run. The correction UI failure was reproduced on unchanged base in prior source-scale work. Photographic donor quality beyond synthetic/declared defect fixtures, physical monitors, native windows and Windows runtime remain unverified. Prior broad feature review failed with a provider error; the completed independent review here specifically examined the normalization revision and relevant combined source/cache boundaries.

Preserve main's ISSUE_TRIAGE_DRAFTS.md and all other worktrees/artifacts. Pre-rebase checkpoint c3f3bba is retained as checkpoint/digital-normalization-pre-layers. Private/verification caches stay in this worktree until no longer needed; do not remove before merge.
