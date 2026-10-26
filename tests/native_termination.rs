//! native termination check: macOS runs the real check, other platforms report it.

#[cfg(target_os = "macos")]
#[path = "native_termination/macos.rs"]
mod macos;

fn main() -> antitone::platform::host::Result<()> {
    #[cfg(target_os = "macos")]
    {
        macos::main()
    }

    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("this check requires macOS");
        Ok(())
    }
}
