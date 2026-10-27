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
fn meminfo_kib(name: &str) -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    text.lines()
        .find_map(|line| line.strip_prefix(name))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|value| value.parse().ok())
}

#[cfg(feature = "raw")]
pub(crate) fn memory_pressure() -> MemoryPressure {
    if let Ok(psi) = std::fs::read_to_string("/proc/pressure/memory")
        && let Some(avg10) = psi
            .split_whitespace()
            .find_map(|part| part.strip_prefix("avg10="))
            .and_then(|value| value.parse::<f64>().ok())
    {
        return match avg10 {
            value if value >= 20.0 => MemoryPressure::Critical,
            value if value >= 5.0 => MemoryPressure::Warning,
            _ => MemoryPressure::Normal,
        };
    }
    match (meminfo_kib("MemAvailable:"), meminfo_kib("MemTotal:")) {
        (Some(available), Some(total)) if total != 0 => {
            let remaining = available as f64 / total as f64;
            if remaining < 0.10 {
                MemoryPressure::Critical
            } else if remaining < 0.20 {
                MemoryPressure::Warning
            } else {
                MemoryPressure::Normal
            }
        }
        _ => MemoryPressure::Normal,
    }
}

#[cfg(feature = "editor")]
pub(crate) fn physical_memory() -> u64 {
    meminfo_kib("MemTotal:").map_or(0, |kib| kib.saturating_mul(1024))
}

#[cfg(any(feature = "project", feature = "raw"))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FileIdentity {
    pub(crate) device: u64,
    pub(crate) inode: u64,
    pub(crate) changed: (i64, i64),
}

#[cfg(any(feature = "project", feature = "raw"))]
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

/// The graphics API this platform presents and computes with.
#[cfg(feature = "gpu")]
pub(crate) fn backends() -> wgpu::Backends {
    wgpu::Backends::VULKAN
}

/// Native drivers compile the translated shaders; no runtime compiler choice applies.
#[cfg(feature = "gpu")]
pub(crate) fn backend_options() -> wgpu::BackendOptions {
    wgpu::BackendOptions::default()
}

#[cfg(all(feature = "gpu", any(test, feature = "ui")))]
pub(crate) fn allocated_resource_bytes(_device: &wgpu::Device) -> Option<usize> {
    None
}

#[cfg(feature = "editor")]
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
pub(crate) fn config_dir(app: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".config"))
        })
        .map(|directory| directory.join(app.to_ascii_lowercase()))
}

#[cfg(feature = "project")]
pub(crate) fn cache_dir(app: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".cache"))
        })
        .map(|directory| directory.join(app.to_ascii_lowercase()))
}
