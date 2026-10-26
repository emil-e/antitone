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
pub(crate) fn memory_pressure() -> MemoryPressure {
    match sysctl::<u32>(c"kern.memorystatus_vm_pressure_level") {
        Some(level) if level >= 4 => MemoryPressure::Critical,
        Some(level) if level >= 2 => MemoryPressure::Warning,
        _ => MemoryPressure::Normal,
    }
}

#[cfg(feature = "editor")]
pub(crate) fn physical_memory() -> u64 {
    sysctl::<u64>(c"hw.memsize").unwrap_or(0)
}

#[cfg(feature = "raw")]
fn sysctl<T: Default>(name: &std::ffi::CStr) -> Option<T> {
    let mut value = T::default();
    let mut size = std::mem::size_of::<T>();
    // Fixed-size scalar sysctls write into matching owned storage.
    let status = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            (&mut value as *mut T).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    (status == 0 && size == std::mem::size_of::<T>()).then_some(value)
}

#[cfg(feature = "project")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FileIdentity {
    pub(crate) device: u64,
    pub(crate) inode: u64,
    pub(crate) changed: (i64, i64),
}

#[cfg(feature = "project")]
pub(crate) fn file_identity(path: &Path) -> std::io::Result<FileIdentity> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::metadata(path)?;
    Ok(FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        changed: (metadata.ctime(), metadata.ctime_nsec()),
    })
}

#[cfg(feature = "editor")]
pub(crate) fn same_file(a: &Path, b: &Path) -> bool {
    match (file_identity(a), file_identity(b)) {
        (Ok(a), Ok(b)) => a.device == b.device && a.inode == b.inode,
        _ => false,
    }
}

#[cfg(feature = "export")]
pub(crate) fn exclusive_rename(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let from = CString::new(from.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "path contains NUL"))?;
    let to = CString::new(to.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "path contains NUL"))?;
    // Same-directory temporary output; RENAME_EXCL atomically refuses a competing destination.
    if unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// The graphics API this platform presents and computes with.
#[cfg(feature = "gpu")]
pub(crate) fn backends() -> wgpu::Backends {
    wgpu::Backends::METAL
}

/// Native drivers compile the translated shaders; no runtime compiler choice applies.
#[cfg(feature = "gpu")]
pub(crate) fn backend_options() -> wgpu::BackendOptions {
    wgpu::BackendOptions::default()
}

#[cfg(all(feature = "gpu", any(test, feature = "ui")))]
pub(crate) fn allocated_resource_bytes(device: &wgpu::Device) -> Option<usize> {
    use objc2_metal::MTLDevice;
    // The HAL guard keeps the device alive; this query neither mutates nor destroys resources.
    let native = unsafe { device.as_hal::<wgpu::hal::api::Metal>() }?;
    Some(native.raw_device().currentAllocatedSize())
}

#[cfg(feature = "editor")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GpuMemory {
    Unified,
    Discrete { dedicated_bytes: Option<u64> },
    Unknown,
}

#[cfg(feature = "editor")]
pub(crate) fn gpu_memory(device: Option<&wgpu::Device>) -> GpuMemory {
    device
        .and_then(|device| {
            use objc2_metal::MTLDevice;
            // Read only a static topology fact under the HAL guard; no resource/ownership changes.
            let native = unsafe { device.as_hal::<wgpu::hal::api::Metal>() }?;
            Some(if native.raw_device().hasUnifiedMemory() {
                GpuMemory::Unified
            } else {
                // Metal's recommended working set is not a dedicated VRAM capacity.
                GpuMemory::Discrete {
                    dedicated_bytes: None,
                }
            })
        })
        .unwrap_or(GpuMemory::Unknown)
}

#[cfg(feature = "editor")]
pub(crate) fn config_dir(app: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|home| {
        std::path::PathBuf::from(home)
            .join("Library/Application Support")
            .join(app)
    })
}

#[cfg(feature = "project")]
pub(crate) fn cache_dir(app: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".cache").join(app))
}
