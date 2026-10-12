#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MemoryPressure {
    Normal,
    Warning,
    Critical,
}
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
#[cfg(target_os = "macos")]
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
#[cfg(not(target_os = "macos"))]
fn sysctl<T: Default>(_name: &std::ffi::CStr) -> Option<T> {
    None
}
