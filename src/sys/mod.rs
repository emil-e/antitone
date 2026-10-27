//! Process, filesystem and hardware facts shared by modules that do not depend on the desktop UI.
#[cfg(target_os = "macos")]
mod apple;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "macos")]
use self::apple as imp;
#[cfg(target_os = "linux")]
use self::linux as imp;
#[cfg(target_os = "windows")]
use self::windows as imp;
#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
compile_error!("sys support is implemented for macOS, Windows and Linux");

#[cfg(all(feature = "gpu", any(test, feature = "ui")))]
pub(crate) use imp::allocated_resource_bytes;
#[cfg(feature = "project")]
pub(crate) use imp::cache_dir;
#[cfg(feature = "export")]
pub(crate) use imp::exclusive_rename;
#[cfg(any(feature = "project", feature = "raw"))]
pub(crate) use imp::file_identity;
#[cfg(feature = "editor")]
pub(crate) use imp::physical_memory;
#[cfg(feature = "editor")]
pub(crate) use imp::same_file;
#[cfg(feature = "editor")]
pub(crate) use imp::{GpuMemory, config_dir, gpu_memory};
#[cfg(feature = "raw")]
pub(crate) use imp::{MemoryPressure, memory_pressure};
#[cfg(feature = "gpu")]
pub(crate) use imp::{backend_options, backends};
