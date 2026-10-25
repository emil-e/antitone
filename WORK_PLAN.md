# Windows port

Worktree: `.worktrees/windows-port`; delivery branch: `port/windows-supported` (preserves the older
published `port/windows` history). User authorized rebase, commit/push, PR creation and watching CI
through green; macOS hardware verification is delegated to the user. No release tag is authorized.

## Goal and agreed scope

Deliver Windows 11 as a first-class supported desktop platform without regressing macOS,
behind a platform seam reusable for Linux.

- Windows 11 x64 is the user-facing acceptance target. ARM64 remains build/functional-only
  scope; its status must not be represented as x64 hardware acceptance. Windows 10 is out of scope.
- Full editor behaviour: LibRaw, direct DNG SDK, projects, analysis, dust, looks and export.
- Native menus; Preferences and Quit in File; no duplicated in-window menu.
- SDR presentation initially, with linear-float internal rendering. HDR presentation/output
  remains later work, not a reason to retain DX12.
- Portable ZIP first; no MSIX, signing, file associations or public release in this task.
- Linux is not delivered here. Platform/system details belong behind the platform/system seam.
- The user explicitly requested that docs describe Windows as supported when this branch merges.
- Integration/pushing/PR creation needs an explicit request; merging is the user's action.

## Current implementation and verification

Acceptance hardware: Windows 11 x64, RTX 2080 Ti, NVIDIA driver 32.0.16.1714. Private samples
are under `G:/Lightroom`, read-only and never committed.

- The editor has opened, developed, analysed, displayed and exported real scans on this machine.
- Vulkan is the Windows backend. Internal RGBA16Float rendering is copied to an SDR
  `Rgb10a2Unorm` surface, with `Bgra8Unorm` fallback. Authored UI RGB `(34,36,38)` presented
  unchanged on the tested display.
- Native Win32 menus, light caption matching the light menu strip, window icon, executable
  icon/version resources and fullscreen taskbar coverage are implemented. egui's native-theme
  synchronization is disabled on Windows so the dark editor does not recolour the light caption.
- MSVC bridges use `/EHsc`; source opening uses wide paths; native-reader fixtures use
  disposable files. LibRaw and the DNG/XMP/libjxl build run from a plain PowerShell session.
- Maintained setup/package scripts and a Windows CI workflow exist. The ZIP bundles the
  app-local VC++/OpenMP runtime and smoke-tests the staged executable with `--version`.
- Main's shader simplification and stage/viewer changes are incorporated. The photographic
  processing has one plain-f32 implementation per stage; the measured A/B difference was at
  most one RGB16 code across 149,299,200 channel samples. Evidence remains in `.cache/ab/`.
- The scan-region observer is Rust; its native ABI, JSON writer and translation unit are gone.
  Temporary parity checks compared 2,106 numeric line fields over 12 scans from the January 16
  directory, plus 513 fields over three October 3 scans, with exact equality. Line/segment counts
  and line booleans were also checked; segment field values were not compared. The recorded
  twelve-scan timings were 1.147 s native versus 0.584 s Rust. Comparison scaffolding was removed.
- Most recent release editor-library suite: **1,153 passed, 0 failed, 19 ignored**. The
  no-default-features suite also passed. `cargo check --all-targets`, release editor build and
  Rust formatting passed. Clippy reports existing warnings; do not describe it as warning-free.
- The rebuilt editor was launched after the caption change and displayed a real scan with its
  thumbnail and `Project ready`; no errors were logged. This is not a complete post-merge
  acceptance pass over analysis, project round trips and export.

## Outstanding work and limits

1. **GPU memory queries implemented:** the active Vulkan device supplies physical local heap
   capacity for discrete cache defaults, or Unified for integrated hardware. wgpu allocation
   counters supply live buffer/texture bytes, excluding spare allocator reservation and driver
   residency. Missing device/backend remains Unknown/None. Dynamic driver budgets are not
   queried because neither existing consumer uses them. Real RTX 2080 Ti capacity was
   11,787,042,816 bytes; allocated resource bytes rose and retired back to baseline in the test.
   Persisted user cache limits remain authoritative; hardware defaults do not overwrite them.
2. **macOS verification:** this Windows machine cannot compile/run the Apple-specific path or
   Metal suites. Hosted CI can check macOS compilation; local Metal checks are still required
   before integration. Independent review of this follow-up and selected broader port boundaries
   is complete; its four concrete findings were assessed and fixed (see below).
3. **Delivery checks:** ZIP rebuild and relocated smoke verification pass. Still exercise
   native menu accelerators/file dialogs, project Save/Open and analysis/export after the main sync.
   Hosted workflows have not run for this branch; fresh-runner setup remains unverified.
4. **Documentation and release wiring implemented:** README and contributor guides describe
   Windows as supported. `release.yml` validates the tag, waits for both platform packages,
   verifies downloaded checksums and ZIP integrity, and publishes both ZIPs with one manifest.
   Windows packaging is reused from its CI workflow and includes relocated smoke/hash checks.
   No public release/tag has been created. Workflow parsing and seven release-assembly unit
   tests pass locally; hosted execution and macOS packaging remain unverified here.
5. **ARM64:** no current ARM64 acceptance hardware here; the package script and CI target x64.

## HDR capability: established versus unimplemented

The later native surface probe supersedes the early SDR-only observation. With the window on the
HDR display, Vulkan reported `R16G16B16A16_SFLOAT` + `EXTENDED_SRGB_LINEAR_EXT` (scRGB) and
`A2B10G10R10_UNORM_PACK32` + `HDR10_ST2084_EXT`, through the v1 query used by wgpu. wgpu maps
both pairs. On the SDR display the reported pairs were sRGB. Vulkan HDR needs no demonstrated
upstream fix on this hardware: remaining application work is monitor-aware format/colour-space
selection, reconfiguration when moving displays and matching scRGB/PQ output encoding. Current
presentation stays SDR; HDR output rendering remains deferred too.

## Deferred observations, not Windows blockers

- Same-key `document_prepared` currently clears and reacquires the pending visit; source-key
  retention could avoid duplicate development. The correction test deterministically pauses at
  `UnpackBegin` and checks visit retirement/product installation.
- Roll analysis can redevelop nonselected originals after import. Evidence is in
  `.cache/issues/roll-analysis-redevelopment.md`; this is shared caching work, not another port task.
- Vulkan remains selected after shader simplification: measured cold compile was 2.6 s versus
  8.2 s with DXC; FXC failed. DX12/WARP coverage in VMs/remote sessions is a possible follow-up,
  not a shipped fallback. Final main-composed shader cold timing still needs remeasurement if
  reporting a current startup-performance claim.

## Active follow-up: GPU telemetry and tagged releases

The user explicitly requested both implementations. Acceptance:

- Windows GPU classification/capacity uses the active device, not a guessed first adapter;
  discrete local heap capacity selects cache defaults and UMA remains distinguishable.
- Resource diagnostics report live wgpu allocation bytes, not total VRAM, allocator reservation
  or process/driver residency. Missing/unsupported data remains distinguishable from zero.
- Tagged releases build and verify both macOS ARM64 and Windows x64 ZIPs before publishing
  one release with both packages and a combined SHA-256 manifest.
- Verify capacity/retirement on the real Vulkan device, release packaging/checksum integrity,
  workflow structure, feature builds and documentation. No tag or public release is authorized.

Fetched main is now `d382d8d` (#59, source-record retirement); its changes do not touch the GPU
query or release boundaries. Task history remains on `820cfb3`; refresh/rebase before delivery.

Verification complete for the available Windows boundaries:

- Release editor-library suite: 1,153 passed, 0 failed, 19 ignored. Five targeted system tests
  include capacity, encoded-command retention/retirement, competing no-overwrite publication,
  Unicode paths and embedded-NUL rejection. The GPU test flushes device-initialization staging
  before taking its baseline; allocator reports explained the initial 208-byte baseline drift.
- Core suite, GPU all-target feature check, UI library check, editor all-target clippy and the
  UI native-host target compile/run pass. The Windows native-host target explicitly reports
  macOS-only status, not native acceptance. Clippy retains existing warnings.
- `--features ui --all-targets` still fails in `tests/affine_resample.rs`, which enables itself
  with `ui` alone but imports a `ui,render` module. This file is unchanged from fetched main;
  the production editor includes `render`. Reported rather than broadening this follow-up.
- Seven release-assembly tests, real Windows archive digest/ZIP verification, PowerShell parsing,
  workflow YAML/concurrency assertions, Ruff, Rust formatting and both modified native files'
  pinned clang-format checks pass. The full authored-source checker still needs WGSL/Swift tools.
- Rebuilt ZIP passes staged and relocated `--version`, executable/runtime byte hashes and SHA-256.
  Local package: `.cache/dist/Antitone-windows-x64.zip`; checksum: `.cache/dist/SHA256SUMS`.

Independent review findings resolved:

1. Namespaced the reusable Windows workflow's concurrency group so it cannot wait on the parent
   release's case-insensitively identical group.
2. Guarded Windows-only UTF-8/wide-path helpers in both native bridges and restored LibRaw's
   narrow-path call on non-Windows builds.
3. Apple backend selection returns Metal directly rather than recursing through its own re-export.
4. Windows exclusive publication uses same-volume `MoveFileExW` without replace/copy flags, not
   a hard-link fallback that copied into the final name. Three tests cover its safety contract.

Feature verification additionally exposed an editor-only gate around the host's general window
configuration hook; the hook now exists for UI-only builds, with editor icon decoding still gated.
No macOS compiler/Metal checks or hosted workflows have run here. Delivery is authorized; the
user will handle macOS hardware verification. Git fetch works but the GitHub API connection currently
reports `authenticated: false`, blocking PR creation and CI inspection until access is restored.

## Delivery in progress

- Rebased all seven task commits onto latest fetched main `bfcc7bf` (#59 source-record retirement;
  #60 source-view white balance), preserving both changes. Range-diff shows no dropped port changes.
- Main's new SDK viewing-white fixture wrote through NamedTempFile::reopen; its setup now uses
  std::fs::write on the port's ordinary fixture path, before opening native readers. Assertions
  and intended input data are unchanged.
- Rebased release editor library: 1,161 passed, 0 failed, 19 ignored. Editor all-target clippy
  completes with existing warnings. Release editor rebuilt and ZIP smoke/hash verification passed
  (13.8 MB). Seven release-assembly tests, Rust formatting, whitespace and docs links pass.
- Keep this plan until GitHub access/delivery is unblocked; remove it before the final PR,
  retaining a disposable copy under `.cache/`.
- Preserve the old remote `port/windows`; publish `port/windows-supported` without force-pushing.
- Remaining remote prerequisite: restore the pinned GitHub API authentication, then create PR and
  watch Windows/macOS hosted CI. Do not create a release tag or merge the PR.
