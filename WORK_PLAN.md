# Windows port

Worktree: `.worktrees/windows-port`, branch `port/windows`, based on `origin/main` (`6f2f4fd`).

## Goal

Bring the Antitone editor to Windows 11 without regressing macOS, behind a capability-based platform seam that also serves Linux.

## Agreed scope

- Targets: Windows 11 x64 and Windows 11 ARM64. x64 is the user-facing and acceptance target; ARM64 is built and functionally exercised in a VM.
- Windows 10 is out of scope. A contributor with Windows 10 hardware may propose it separately.
- Presentation: SDR only initially, at parity with macOS (EDR is disabled today). Keep the internal pipeline linear float and presentation behind a platform adapter; do not bake in an 8-bit-only path. HDR remains a later addition.
- Delivery: portable ZIP first. No MSIX, signing, file associations or public release in this task.
- Full editor behaviour: LibRaw, direct DNG SDK, export, dust, looks and project files at parity.
- Linux: not delivered here, but the seam must be capability-based, with no OS conditionals outside `src/platform/`, the build layer and Cargo target tables. Linux is the third implementation, not a future rewrite.
- Documentation: do not claim Windows support in README/docs until the 2080 Ti acceptance run is recorded.

## Environment

- Host: Apple Silicon macOS with VMware Fusion; Windows 11 ARM64 guest.
- Guest toolchain: VS 2022 Build Tools (MSVC v143 ARM64 and x64, Windows 11 SDK, CMake, Ninja), Git, Python 3, rustup with `aarch64-pc-windows-msvc` and `x86_64-pc-windows-msvc`.
- Acceptance: x64 Windows 11 machine with an RTX 2080 Ti and current driver, plus a real display for SDR presentation checks. ARM64 is VM-only: functional, not performance, acceptance.
- Agent access: SSH into the guest (see "VM access").
- Guest: Windows 11 Pro ARM64 26200, 24 GB RAM, 8 vCPU. C: extended to 107 GB (54 GB free); WinRE re-enabled on C:.
- Toolchain verified: VS Build Tools 17.14.42 (MSVC 14.44.35207, Windows SDK 10.0.26100, CMake, Ninja) and rustup 1.99.0 for `aarch64-pc-windows-msvc` with the `x86_64-pc-windows-msvc` target. Guest build wrappers `C:\Users\pi\build-arm64.cmd` and `build-x64.cmd` call `VsDevCmd.bat`.
- Host free space recovered to ~78 GiB by clearing inactive worktree targets; active targets and the main checkout were left alone.

## Phases

1. **Portability seam**
   - One selection point in `src/platform/mod.rs`; capability modules for backend, presentation, quit, menu, memory, filesystem and icon.
   - Name modules/capabilities `apple`, `unix` or `windows`; no `cfg(target_os)` outside `platform/`, the build layer and per-capability files.
   - Move OS dependencies into Cargo target tables; keep features capability-based.
   - Make `host.rs` portable; keep AppKit termination, colour tagging and fullscreen behaviour in the Apple adapter.
   - Acceptance: macOS checks unchanged; cross-target `cargo check` for both Windows targets and `x86_64-unknown-linux-gnu` where toolchains are available.
2. **Native MSVC build**
   - Port `native/dng/CMakeLists.txt` (no `libtool`, Windows defines, libjxl/libjpeg under MSVC), `build.rs` and `scripts/setup-native.sh`.
   - Build LibRaw and OpenMP for Windows; choose between MSVC `/openmp` and LLVM libomp, and settle the redistributable policy.
   - Verify lcms2's static MSVC build through Cargo.
   - Acceptance: `dng-sdk`, `raw` and `ocio-reference` build; probe binaries run.
3. **Host integration**
   - Windows host on the shared event loop: native menu, quit/close handshake, borderless fullscreen, `.ico`, preferences path, file dialogs.
   - Acceptance: editor runs; menus, dialogs and the unsaved-close flow match macOS behaviour.
4. **Presentation**
   - Resolve the DXGI colour space for the `Rgba16Float` swapchain; SDR parity with the macOS extended-sRGB path.
   - Validate encode math against presentation probes in the VM; validate display output on hardware.
5. **Packaging and CI**
   - Portable ZIP: DLL dependency audit, VC++ runtime and OpenMP policy, notices, `--version` smoke test.
   - Hosted Windows CI: build plus core/offscreen WARP tests; no presentation or performance gate.
6. **Acceptance**
   - 2080 Ti: functional editor run, SDR presentation comparison, export, and performance/memory budgets at 8704×6144.
   - ARM64 VM: functional run.
   - Docs: supported-platform statement only after acceptance.

## Baseline inventory (Windows 11 ARM64)

- Pass: core (`--no-default-features`), `gpu`, `ui`, `export`, `project`.
- Fail: `dng-sdk` at the `build.rs` macOS assertion; `raw` at the missing LibRaw build (expected); `editor` stops at `raw` before reaching the host or DNG.
- Consequence: the Rust, GPU-compute, UI and export layers already compile on Windows. The real work is the native build layer, a Windows host implementation, runtime backend/presentation selection, and OS capabilities (memory, filesystem, menu, quit).

## Progress

- Phase 1 compile seam: macOS `cargo check --features ui` and `--features editor` pass; Windows ARM64 `cargo check --features ui` passes, including the portable host, DX12 backend selection, Windows capability module and in-window menu fallback.
- `platform/mod.rs` is the single OS selection point. `host.rs`, `fullscreen.rs` and `latency.rs` are portable; macOS capabilities live in `platform/macos.rs`; Windows capabilities are stubs in `platform/windows.rs`.
- Windows stubs still no-op: presentation tagging, quit negotiation, fullscreen policy, GPU memory and icon. The native menu is disabled (`SUPPORTED = false`), so the in-window menu is used.
- `sys/` now owns memory pressure, physical memory, file identity, no-overwrite publish, GPU memory kind and resource accounting behind one target selection; `platform/` is desktop-only. The macOS feature matrix (core, raw, project, export, gpu, ui, editor) passes.
- `FileRevision` now carries device/inode/change fields on every platform with serde defaults, so projects stay compatible across platforms.
- Native build: LibRaw 0.22.0 builds with MSVC and `/openmp`, and the Windows RAW probe links. `scan_regions` is ported to portable C++ with no Foundation dependency; the macOS native boundary test passes.
- Runtime host and DXGI presentation remain unverified; `tests/native_host.rs` is still macOS-only.
- Next: DNG SDK CMake for MSVC, then a Windows host smoke test.

## Open risks

- wgpu 30 DX12 support and feature levels on Windows 11 ARM64; `Rgba16Float` swapchain availability.
- DNG SDK/libjxl MSVC build effort and archive/link integration.
- DXGI colour-space semantics versus macOS extended-sRGB tagging; what this wgpu version exposes for surface colour space.
- OpenMP runtime choice and redistributable obligations on Windows.
- Export publishing still uses macOS `renamex_np`; Windows needs a no-overwrite publish tested on NTFS. Project save already falls back to `create_new` when no-replace rename is unsupported (#30); verify that path on NTFS.
- Memory-pressure equivalents on Windows/Linux; implement only for the consumers that matter (issue #37 removes the RAW thread throttle).
- GPU memory query for cache budgets: wgpu exposes no DXGI video-memory information, so a native query is needed or budgets fall back to *unknown*.
- No GitHub push credentials from this environment; remote actions need working auth or the user.

## VM access

- Verified: `pi@192.168.166.131` (administrator), key at `.cache/ssh/antitone-vm`; NAT network reachable from the host.
- Do not expose SSH to the internet.

## Next actions

1. Commit this plan as the first checkpoint.
2. Create the Windows 11 ARM64 VM, install the toolchain, and run the baseline failure inventory.
3. Start phase 1 in this worktree.
