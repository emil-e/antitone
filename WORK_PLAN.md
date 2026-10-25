# Windows port

Worktree: `.worktrees/windows-port`, branch `port/windows` (rebased onto `origin/main` @ `432c3a8` via merge `7c02bcf`).

## Status: working port on the acceptance hardware (2026-10-25)

The editor builds, opens, develops, analyses, displays and exports 61 MP scans on the RTX 2080 Ti
through Vulkan, and the library test suite passes on Windows (1123 passed, 0 failed, 20 ignored).

Delivered in this session: presentation made correct for the Vulkan swapchain; DX12 retired after
measurement; the photographic arithmetic reduced to one f32 implementation per stage with the
kernel compile falling from 242 s to 2.6 s; composed shaders validated by a test; a plain-shell
Windows build with checkout-local pinned tools; wide-path source opening; fixtures that native
readers can open; a portable ZIP with an app-local runtime and smoke test; a Windows CI workflow;
and Windows build documentation.

Evidence still owed to the maintainer: the visual A/B page lives in `.cache/ab/index.html`
(served locally, or opened directly), built from `before.tif` and `after.tif` exported by the
ignored `editor::ab_probe_tests::ab_export` probe: maximum difference 1 code in 65 535 across
149 299 200 channel samples, with the difference overlay marking the isolated pixels.

Former flake, now deterministic: `editor::source_workflow::tests::a_correction_preparation_retires_the_pending_visit_and_installs_its_product`
holds the acquisition at `UnpackBegin` with the paused scheduler, so the correction always starts
while a visit is pending. It asserts that the visit is retired rather than left dangling and that
the document's own product is installed. Scope survival follows the prepared product's key, not
the preparation kind: a correction without a reference frame leaves the development unchanged and
the visit continues into the installed document; one that changes the gains changes the key and
replaces the scope. `document_prepared` currently clears the pending visit unconditionally, so a
same-key preparation still cancels and re-acquires; retaining the visit there is the change to
make if that duplicated development is worth removing.

Open judgement call: `photographic_precise.wgsl`, `film_inversion_precise.wgsl` and the compensated
arithmetic library are composed by nothing, but the colour oracle tests still compare the shipped
implementation against them. They are a second implementation kept only as a test reference;
delete them together with those oracle comparisons if one implementation is preferred.

## Status: resumed on acceptance hardware (2026-10-24)

The parked branch now runs on the acceptance target: Windows 11 x64 (26200), RTX 2080 Ti with driver 32.0.16.1714, DX12 adapter, `Rgba16Float` surface, 2560×1440 SDR displays. The VMware ARM64 guest is no longer the working environment; ARM64 remains built/functional-only scope.

## Goal

Bring the Antitone editor to Windows 11 without regressing macOS, behind a capability-based platform seam that also serves Linux.

## Agreed scope

- Targets: Windows 11 x64 and Windows 11 ARM64. x64 is the user-facing and acceptance target.
- Windows 10 is out of scope. A contributor with Windows 10 hardware may propose it separately.
- Presentation: SDR only initially, at parity with macOS (EDR is disabled today). Keep the internal pipeline linear float and presentation behind a platform adapter; do not bake in an 8-bit-only path. HDR remains a later addition.
- Delivery: portable ZIP first. No MSIX, signing, file associations or public release in this task.
- Full editor behaviour: LibRaw, direct DNG SDK, export, dust, looks and project files at parity.
- Linux: not delivered here, but the seam must be capability-based, with no OS conditionals outside `src/platform/`, the build layer and Cargo target tables. Linux is the third implementation, not a future rewrite.
- Documentation: do not claim Windows support in README/docs until the acceptance run is recorded.

## Building on Windows (contributed, not yet in docs/build-and-run.md)

A plain PowerShell session builds the editor; no developer prompt is needed.

```
powershell -ExecutionPolicy Bypass -File scripts/setup-windows.ps1   # CMake, Ninja, DXC, LibRaw
cargo build-app
cargo app -- G:/path/to/photo.ARW
```

- `scripts/setup-windows.ps1` fetches pinned, hash-verified CMake 4.4.4, Ninja 1.13.2 and DXC 1.9.2609 into ignored `.cache/tools/`, copies `dxcompiler.dll` beside a built executable for the DX12 backend, and runs `build-libraw.ps1`.
- `build.rs` finds CMake in `CMAKE`, `PATH` or `.cache/tools/cmake/*/bin`, and adds `.cache/tools/ninja` when `ninja` is not on `PATH`.
- The CMake configure needs the Windows SDK's `rc.exe`, which only a developer prompt exports, so `build.rs` captures `VsDevCmd.bat`'s environment through a generated batch file and applies it to the CMake steps. `cc` finds MSVC itself.
- Public documentation waits for the acceptance run, per the scope above.

## Environment

- Host: Windows 11 x64, RTX 2080 Ti, MSVC 14.36 (VS 2022 Community 17.6.5), Windows SDK 10.0.22000, Rust 1.99.0 `x86_64-pc-windows-msvc`.
- Checkout-local tools: CMake 4.4.4 and Ninja 1.13.2 unpacked under `.cache/tools`; `.cache/devbuild.cmd` enters `VsDevCmd.bat -arch=x64` (the CMake resource-compiler step needs the SDK on `PATH`, which `build.rs` does not arrange itself).
- LibRaw 0.22.0 built by `scripts/build-libraw.ps1 -Arch x64 -HostArch x64`. The script needs the Windows `tar.exe`; a Git-for-Windows `tar` first on `PATH` breaks extraction ("Cannot connect to C: resolve failed").
- Disk is a live constraint: ~15 GB free, `.cache` 167 MB, `target` 3.7 GB.

## What changed on resume

- **Presentation is now correct on DX12.** wgpu 30 advertises `ExtendedSrgbLinear` (DXGI `RGB_FULL_G10_NONE_P709`, scRGB) as the only colour space for an `Rgba16Float` surface, so the swapchain interpreted the shared extended-sRGB-encoded output as linear. Measured on hardware: authored `panel_fill` `(34,36,38)` presented as `(102,105,108)`, i.e. each channel re-encoded by 2.2.
- The seam now states both facts a platform decides: `platform::presentation()` returns the surface `color_space` and an `OutputTransfer` (`Srgb` on macOS, `Linear` on Windows). When the surface is linear, `host.rs` renders EGUI and paint callbacks into a staging `Rgba16Float` texture and converts once while presenting (`src/shaders/presentation_linear.wgsl`, `textureLoad` for an exact copy orientation).
- Verified after the change: `(34,36,38)` presents as `(34,36,38)`, so DWM's scRGB→SDR path round-trips the authored value.
- macOS keeps its behaviour: `color_space: Auto` (unchanged default) and `Srgb` transfer, so the staging path is not built and the Metal layer tagging is still the encoder.

## Backend decision: Vulkan (measured)

DX12's runtime shader compilers cannot handle the photographic kernel, so the Windows backend is now Vulkan, which lowers WGSL to SPIR-V and leaves optimisation to the display driver.

- Standalone measurement of the same module (`.cache/hlsl-probe --bin pipeline-time`, Vulkan): module 240 ms, compute pipeline 60 ms, total ~0.3 s. DX12 in the same program: 151 s (`-O3`), 253 s (`-Od`).
- Vulkan on Win32 offers no float swapchain. `Rgb10a2Unorm` presents correctly (verified on both the 10-bit Rec.2020/PQ display and the 8-bit SDR display, with no acquisition failure; an earlier "10-bit fails" observation was an artefact of attaching cdb to the live swapchain). `Bgra8UnormSrgb` remains the fallback. The scRGB staging conversion is gone with the DX12 backend.
- Colour spaces: wgpu's Vulkan surface reports `SurfaceColorSpaces(SRGB)` for every format even when the display is in HDR mode, so wgpu cannot present PQ/extended-range content on Vulkan today. HDR10/extended range remains a DX12 (or upstream Vulkan) path.
- `sys::backend_options()` and `gpu::instance()` remain the single place that names the API and the DX12 compiler policy, should DX12 be revisited.

### Resolved: driver compile cost, not our device setup

Instrumentation of every pipeline constructor settled it:

- A tiny compute shader builds in 0.2 ms, and the kernel builds in 27 ms on a device created inside the editor with `DeviceDescriptor::default()` — so the device configuration, surface, prewarm thread and error scopes were never the problem.
- The cost is the driver compiling each *distinct* kernel variant the first time it is seen: 97.9 s for the export variant on its first ever build, then 13–27 ms for every variant afterwards in the same and later processes. The interactive variant had already been cached by earlier runs, which is why the slow build kept appearing at different points in different logs.
- Consequence: the first launch after a driver or shader change pays ~98 s per kernel variant; warm launches reach an interactive editor with a scan in about 12 s. Options for later: persist pipelines (wgpu `PipelineCache`), or slim the kernel, or accept the one-time cost.

### Fixed: application pipelines assumed a float surface

With a 10-bit or 8-bit swapchain the editor failed at the first image draw:

```
Render pipeline targets are incompatible with render pass
  RenderPass uses [Rgb10a2Unorm] but 'image render pipeline' uses [Rgba16Float]
```

Every application pipeline renders in `Rgba16Float` because macOS always presents float. The host now renders into a float staging image and bridges to the platform's swapchain format with one copy pass (`shaders/presentation_copy.wgsl`); both presentation formats (`Rgb10a2Unorm`, then `Bgra8Unorm`) store the pipeline's sRGB-encoded values unchanged, so the pass needs no transfer conversion and no application pipeline changed. This was also the cause of the earlier "surface acquisition validation failed" exits, not the debugger attach or the colour space.

Verified after the fix: the editor shows the scan, authored `(34,36,38)` presents as `(34,36,38)`, and no surface errors.

## DX12 after the arithmetic simplification (measured)

Same kernel, same machine, driver caches cleared before each run, single `main` entry point:

| backend | shader compiler | cold compile |
| --- | --- | --- |
| Vulkan | SPIR-V, driver | 2.6 s |
| DX12 | DXC 1.9 (as shipped) | 8.2 s |
| DX12 | FXC fallback | fails: `D3D11 Internal Compiler Error` (negate modifier on an operand) |

- The earlier DX12 figures (97.8 s for the export formulation, 151–253 s for the precise one) were taken before the simplification; the same reduction that fixed Vulkan fixed DX12 within a factor of three.
- DX12 therefore needs the shipped DXC — FXC cannot compile this shader at all — so `dxcompiler.dll`/`dxil.dll` become a packaging requirement rather than a nicety.
- DX12 offered the only route to extended presentation as far as the earlier session could tell. That was wrong: with the window on the 10-bit Rec.2020/PQ display, the Vulkan surface reports seven pairs, including `R16G16B16A16_SFLOAT` with `EXTENDED_SRGB_LINEAR_EXT` (scRGB) and `A2B10G10R10_UNORM_PACK32` with `HDR10_ST2084_EXT`, and it reports them through the v1 query wgpu already uses. With the window on the SDR display it reports five sRGB pairs. `map_vk_surface_formats` maps both extended pairs, so wgpu exposes them; the earlier "Vulkan reports sRGB only" observation was a window on the SDR display.
- Consequence: extended presentation is available on Vulkan per monitor, with no upstream change. It needs app-side work instead: choose `Rgba16Float` + `ExtendedSrgbLinear` when the current monitor offers it, feed scRGB linear output (the presentation transfer pass that was removed), re-query and reconfigure when the window moves between displays, and fall back to 10-bit sRGB on SDR monitors.
- DX12's remaining advantage is coverage rather than presentation: WARP keeps the editor running in VMs and remote sessions where Vulkan is absent.
- Choosing DX12 for extended range means restoring the presentation transfer (sRGB-encoded pipeline output → linear scRGB), i.e. the staging copy that was removed with it, and re-testing presentation colour on hardware.

## Simplifying the photographic arithmetic (in progress)

Measured cold compiles of the composed kernel, Vulkan, same machine:

| step | compile |
|---|---|
| viewport kernel as found | 242 s |
| viewer composes the export arithmetic (one grade implementation) | 97.8 s |
| guarded compensated fallbacks deleted from `colour_post`/`colour_balance` | 26.7 s |
| f32 mask/coordinate evaluation in `colour_locals` | 22.5 s |
| response fallback removed (single f32 response) | **2.6 s** |

- With the driver's `GLCache` cleared before each run (otherwise every variant is a cache hit and the numbers are meaningless), the costs were: `colour_apply` ≈ 2 s, `colour_dot`/`colour_ratio`/`colour_tone` ≈ 1 s together, and `invert_film` ≈ 22 s.
- `invert_film` carried the same pattern as the colour operators: a cheap f32 path plus a `precise_invert_film` fallback for "sensitive" values and for `|anchor| > 32`. It is now one f32 path; the domain escape went with it, so extreme grade parameters use the same formula and are the first thing the A/B should look at.

- The viewport no longer has its own grade: `apply_creative_grade`/`grade_*`/`ratio_gain`/`cct_*` are neither composed nor needed. `export_shader()` delegates to `shader()`, so viewer, export and analysis share one photographic implementation.
- The removed code was a second implementation inside each colour operator: a fast f32 result plus a "sensitive" branch that recomputed everything in compensated arithmetic. It was always compiled and almost never executed.
- Pending: visual A/B of the removed guards (same scan, same crop, before/after with an amplified difference image) so any photographic change is judged by eye rather than by claim; and re-basing `gpu_working_view_colour_tests` / `gpu_export_arithmetic_research`, which still treat the precise variants as the oracle.

## Shader compilation blocker (Windows x64)

DX12 compiles the translated HLSL at runtime. The editor cannot show a scan until that finishes, and it currently does not finish in usable time:

- The photographic kernel is a ~4 k-line WGSL/HLSL module with 46 loops and 111 functions.
- Every helper takes and returns the `Triple` struct by value. DXIL cannot pass aggregates by value, so DXC inlines all of them: the module becomes ~1.6 M IR instructions unoptimised and ~314 k after `-O3` (measured with `dxc -dumpbin`; 70 k of those are `dx.op.unary.f32`).
- Measured on the RTX 2080 Ti, DXC 1.9.2609, `-HV 2018` (which wgpu always passes): precise kernel 151 s with `-O3`, 253 s with `-Od`, and `-Zi` (what debug builds request) is slower again. Export kernel 69 s. FXC behaves the same way.
- Consequences: `editor::…` tests appear to hang (~15 min for one test) and the editor sits on "Importing RAWs…" indefinitely; it is compilation, not deadlock — sampled stacks sit in `d3dcompiler_47!CCompiler::Emit*` / `dxcompiler!DxcCreateInstance2+…` with the instruction pointer moving.
- Reproduction: `.cache/hlsl-probe` composes the exact shader text and emits naga's HLSL; `.cache/tools/dxccl/bin/x64/dxc.exe -E main -T cs_6_2 -O3 -HV 2018 .cache/shaders.hlsl -Fo out.dxil`.
- Not yet ruled out or tried: pipeline caching (does not fix first run), splitting the kernel into passes, reducing the inlined code volume, naga/wgpu upstream (HLSL 2021 could keep aggregate calls; naga says it targets HLSL 2018).

## Roll analysis cost (measured)

Three 61 MP ARWs, release build, this machine:

| phase | time |
|---|---|
| `add_raws` (3 files, developed in parallel) | 1.9–3.7 s |
| analysis `acquire` frame 0 (already loaded) | 0.7 ms |
| analysis `acquire` frames 1–2 | 2.16 s, 2.12 s |
| analysis `sample` per frame | 51–84 ms |
| analysis `finish` (fit) | 1.07 s |
| analysis total (3 frames) | 5.8–6.1 s |

The cost is almost entirely **re-developing every frame that is not the selected one**: analysis asks for developed originals through the source cache, but the imported originals are not retained (`retain: false`), so each frame is developed again — serially, at about three times the per-frame cost of the parallel import.

Only 8192 sample pixels per frame are needed (`MAX_SAMPLES_PER_FRAME`), so the fix is to sample during import (the scan is 51–84 ms per frame) and let analysis consume the stored samples instead of developing again; keeping whole originals is not an option (61 MP × 3 planes × 4 B ≈ 840 MB per frame).

## Test-suite state (Windows x64)

- The lib test suite now compiles: `cargo test --locked --features editor --lib --no-run` is clean.
- The graphics API is selected once, in `sys`; `gpu::backends()` is the public accessor for probes. Test contexts no longer hardcode `BACKENDS::METAL` (78 sites).
- `Scheduler::close_and_wait`, `working_precision`, `raw_test_support`, the GPU export context and the shared arithmetic research diagnostics are no longer macOS-only.
- Fixed: `dng::tests` deadlocked because the C++ bridges were compiled without `/EHsc`; MSVC then skips destructor cleanup while unwinding, so the development lock stayed held by an exited thread. `build.rs` now compiles every C++ bridge through `cpp_bridge()`, which adds `/EHsc` for MSVC. All 23 `dng::tests` pass (5.6 s).
- GPU-backed tests are not hanging: they are waiting for DX12 shader compilation (see the blocker above). One `editor::…` test takes over 15 minutes in a single thread.

## Progress

- Platform seam complete: one selection point in `platform/mod.rs`; `host.rs`, `fullscreen.rs` and `latency.rs` are portable; macOS capabilities live in `platform/macos.rs`; Windows capabilities are in `platform/windows.rs`.
- `sys/` owns memory pressure, physical memory, file identity, no-overwrite publish, GPU facts, config and cache directories behind one target selection; `platform/` is desktop-only.
- Native build: LibRaw and the DNG/XMP/libjxl merge library build with MSVC; the editor binary builds and runs (`--version`, DX12 adapter, in-window menu, empty state).
- Windows test suite: compiles and the CPU-side groups pass; GPU groups are gated by shader compilation time.
- Native Windows menu: implemented with muda from the macOS structure minus the application menu; Preferences and Quit live in File, accelerators are Control-based, and the in-window menu bar is gone (the header keeps the project identity). Enablement and check states come from the same menu::State.
- Window icon: set from the embedded 256 px branding PNG when the window is created (`platform::configure_window`). The Explorer `.exe` icon still needs a resource file compiled at build time.
- Photo fullscreen: the Windows presentation raises the window to `AlwaysOnTop` while it owns fullscreen and restores the level when it ends, so the taskbar no longer overlays the photo.
- Not yet: GPU memory readback. `sys::windows` returns `GpuMemory::Unknown` and no allocated-resource bytes, so cache budgets fall back to conservative limits. The DXGI `IDXGIAdapter3::QueryVideoMemoryInfo` route calls into the `windows-sys` DXGI bindings once a COM adapter is resolved from the wgpu adapter; that needs the adapter handle exposed through the seam.

## Open risks

- Windows test-suite portability: compile-time gates are fixed; the DNG hang and the editor-group access violation remain, both needing a native stack trace.
- Many editor tests exceed 60 s per test on Windows; whether that is the same defect or genuine cost is unresolved.
- Native menu, quit negotiation, fullscreen policy and icon remain stubs or in-window fallbacks.
- GPU memory query for cache budgets currently falls back to *Unknown* on Windows.
- Packaging: VC++ runtime/OpenMP DLL policy, dependency audit and portable ZIP are not implemented.
- Acceptance needs private scans and expected outputs that are not on this machine; performance/memory budgets at 8704×6144 are unmeasured.

## Resume

1. `bash .cache/devbuild.cmd` equivalents: `.cache\devbuild.cmd cargo build --locked --features editor --bin antitone`.
2. Launch from the interactive desktop session; `presentation adapter=… color_space=… transfer=…` confirms the DX12 path.
3. Next: Windows test suite, then native menu/quit/fullscreen/icon, GPU memory, packaging/CI, then the 2080 Ti acceptance run.
