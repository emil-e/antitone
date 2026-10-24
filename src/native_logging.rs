use std::ffi::{CStr, c_char};

// Called only with bridge-owned, NUL-terminated literals. No panic may cross C ABI.
#[unsafe(no_mangle)]
unsafe extern "C" fn antitone_native_log(
    level: i32,
    component: *const c_char,
    operation: *const c_char,
    status: i32,
    elapsed_ms: f64,
) {
    let _ = std::panic::catch_unwind(|| {
        if component.is_null() || operation.is_null() {
            return;
        }

        let component = unsafe { CStr::from_ptr(component) }.to_string_lossy();
        let operation = unsafe { CStr::from_ptr(operation) }.to_string_lossy();

        let level = match level {
            1 => log::Level::Error,
            2 => log::Level::Warn,
            3 => log::Level::Info,
            4 => log::Level::Debug,
            _ => log::Level::Trace,
        };

        log::log!(
            target: "antitone::native",
            level,
            component = component.as_ref(),
            operation = operation.as_ref(),
            status,
            elapsed_ms;
            "native_operation"
        );
    });
}
