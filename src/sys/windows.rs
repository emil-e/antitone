#[cfg(any(feature = "project", feature = "export", feature = "raw"))]
use std::path::Path;

#[cfg(feature = "raw")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MemoryPressure {
    Normal,
    Warning,
    Critical,
}

#[cfg(feature = "raw")]
fn memory_status() -> Option<windows_sys::Win32::System::SystemInformation::MEMORYSTATUSEX> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    (unsafe { GlobalMemoryStatusEx(&mut status) } != 0).then_some(status)
}

#[cfg(feature = "raw")]
pub(crate) fn memory_pressure() -> MemoryPressure {
    // Windows reports system memory load, not macOS's pressure aggregation.
    match memory_status().map(|status| status.dwMemoryLoad) {
        Some(load) if load >= 90 => MemoryPressure::Critical,
        Some(load) if load >= 75 => MemoryPressure::Warning,
        _ => MemoryPressure::Normal,
    }
}

#[cfg(feature = "editor")]
pub(crate) fn physical_memory() -> u64 {
    memory_status().map_or(0, |status| status.ullTotalPhys)
}

#[cfg(any(feature = "project", feature = "raw"))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FileIdentity {
    pub(crate) device: u64,
    pub(crate) inode: u64,
    pub(crate) changed: (i64, i64),
}

#[cfg(any(feature = "project", feature = "raw"))]
fn handle_identity(path: &Path) -> std::io::Result<FileIdentity> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };
    let file = std::fs::File::open(path)?;
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(FileIdentity {
        device: info.dwVolumeSerialNumber as u64,
        inode: ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
        // Windows exposes no ctime equivalent here; modification time remains in FileRevision.
        changed: (0, 0),
    })
}

#[cfg(any(feature = "project", feature = "raw"))]
pub(crate) fn file_identity(path: &Path) -> std::io::Result<FileIdentity> {
    handle_identity(path)
}

#[cfg(feature = "editor")]
pub(crate) fn same_file(a: &Path, b: &Path) -> bool {
    match (handle_identity(a), handle_identity(b)) {
        (Ok(a), Ok(b)) => a.device == b.device && a.inode == b.inode,
        _ => false,
    }
}

#[cfg(feature = "export")]
pub(crate) fn exclusive_rename(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::MoveFileExW;

    let wide = |path: &Path| -> std::io::Result<Vec<u16>> {
        let mut units: Vec<_> = path.as_os_str().encode_wide().collect();
        if units.contains(&0) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "path contains NUL",
            ));
        }
        units.push(0);
        Ok(units)
    };
    let (from, to) = (wide(from)?, wide(to)?);
    // No REPLACE_EXISTING or COPY_ALLOWED: publish by a same-volume rename or fail, including on
    // filesystems without hard links. Copying into the final name would expose incomplete output.
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(all(test, feature = "export"))]
mod file_tests;

/// Vulkan avoids the DX12 runtime HLSL compiler and is the Windows production backend.
#[cfg(feature = "gpu")]
pub(crate) fn backends() -> wgpu::Backends {
    wgpu::Backends::VULKAN
}

/// DX12 compiles the translated shaders at runtime. FXC is the built-in fallback and is
/// pathologically slow on the photographic kernels, so the shipped DXC next to the
/// executable is required rather than optional.
#[cfg(feature = "gpu")]
pub(crate) fn backend_options() -> wgpu::BackendOptions {
    let mut options = wgpu::BackendOptions::default();
    options.dx12.shader_compiler = wgpu::Dx12Compiler::default_dynamic_dxc();
    options
}

#[cfg(feature = "gpu")]
mod gpu;

#[cfg(all(feature = "gpu", any(test, feature = "ui")))]
pub(crate) use gpu::allocated_resource_bytes;

#[cfg(feature = "editor")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GpuMemory {
    Unified,
    Discrete { dedicated_bytes: Option<u64> },
    Unknown,
}

#[cfg(feature = "editor")]
pub(crate) fn gpu_memory(device: Option<&wgpu::Device>) -> GpuMemory {
    device.map_or(GpuMemory::Unknown, gpu::memory_capacity)
}

#[cfg(feature = "editor")]
fn roaming_appdata() -> Option<std::path::PathBuf> {
    std::env::var_os("APPDATA")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(|home| std::path::PathBuf::from(home).join("AppData/Roaming"))
        })
}

#[cfg(feature = "editor")]
pub(crate) fn config_dir(app: &str) -> Option<std::path::PathBuf> {
    roaming_appdata().map(|directory| directory.join(app))
}

#[cfg(feature = "project")]
pub(crate) fn cache_dir(app: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(|home| std::path::PathBuf::from(home).join("AppData/Local"))
        })
        .map(|directory| directory.join(app))
}
