//! native host check: macOS runs the real check, other platforms report it.

#[cfg(target_os = "macos")]
#[path = "native_host/macos.rs"]
mod macos;

fn main() {
    #[cfg(target_os = "macos")]
    macos::main();

    #[cfg(not(target_os = "macos"))]
    eprintln!("this check requires macOS");
}
