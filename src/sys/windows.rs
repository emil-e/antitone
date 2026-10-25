#[cfg(any(feature = "project", feature = "export"))]
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

#[cfg(feature = "project")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FileIdentity {
    pub(crate) device: u64,
    pub(crate) inode: u64,
    pub(crate) changed: (i64, i64),
}

#[cfg(feature = "project")]
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

#[cfg(feature = "project")]
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
    // A same-volume hard link preserves the no-overwrite guarantee; remove the temporary name
    // after publication. Filesystems without hard links fall back to exclusive creation.
    match std::fs::hard_link(from, to) {
        Ok(()) => std::fs::remove_file(from),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Err(error),
        Err(_) => {
            let mut source = std::fs::File::open(from)?;
            let mut destination = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(to)?;
            let result =
                std::io::copy(&mut source, &mut destination).and_then(|_| destination.sync_all());
            drop(destination);
            if result.is_ok() {
                std::fs::remove_file(from)?;
            } else {
                let _ = std::fs::remove_file(to);
            }
            result.map(|_| ())
        }
    }
}

/// Vulkan compiles the photographic kernel in well under a second; DX12 hands the same
/// kernel to FXC or DXC, which need minutes for it.
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

#[cfg(all(feature = "gpu", any(test, feature = "ui")))]
pub(crate) fn allocated_resource_bytes(_device: &wgpu::Device) -> Option<usize> {
    None
}

#[cfg(feature = "editor")]
#[allow(dead_code)] // DXGI capacity query is not implemented; Windows returns Unknown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GpuMemory {
    Unified,
    Discrete { dedicated_bytes: Option<u64> },
    Unknown,
}

#[cfg(feature = "editor")]
pub(crate) fn gpu_memory(_device: Option<&wgpu::Device>) -> GpuMemory {
    GpuMemory::Unknown
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

#[cfg(feature = "editor")]
pub(crate) fn cache_dir(app: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(|home| std::path::PathBuf::from(home).join("AppData/Local"))
        })
        .map(|directory| directory.join(app))
}
