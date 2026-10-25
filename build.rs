/// Bridges throw C++ exceptions inside their own frames, so unwind cleanup must run there.
///
/// `cc` selects no exception model for C++ targets, and MSVC then skips destructors while
/// unwinding, which leaked a locked `std::mutex` out of an aborted DNG development.
// `cc` is an optional build dependency: only the native bridges need it.
#[cfg(any(feature = "raw", feature = "dng-sdk", feature = "ocio-reference"))]
fn cpp_bridge() -> cc::Build {
    let mut build = cc::Build::new();
    build.cpp(true).std("c++17");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        build.flag("/EHsc");
    }
    build
}

fn main() {
    println!("cargo:rerun-if-env-changed=LIBRAW_SOURCE");
    println!("cargo:rerun-if-changed=native/raw_bridge.cpp");
    println!("cargo:rerun-if-changed=native/raw_bridge.h");
    println!("cargo:rerun-if-changed=native/logging.h");
    #[cfg(feature = "raw")]
    {
        let target_os = std::env::var("CARGO_CFG_TARGET_OS").expect("target OS");
        let root = std::env::var_os("LIBRAW_SOURCE")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                let name = if target_os == "windows" {
                    "LibRaw-0.22.0-msvc"
                } else {
                    "LibRaw-0.22.0-openmp"
                };
                std::path::PathBuf::from(".cache/native-deps").join(name)
            });
        if target_os == "windows" {
            assert!(
                root.join("lib/libraw_static.lib").exists(),
                "Run scripts/build-libraw.ps1 before building with --features raw"
            );
            println!(
                "cargo:rerun-if-changed={}",
                root.join("lib/libraw_static.lib").display()
            );
            cpp_bridge()
                .define("WIN32", None)
                .define("LIBRAW_NODLL", None)
                .flag("/openmp")
                .include(&root)
                .file("native/raw_bridge.cpp")
                .compile("antitone_raw_bridge");
            println!(
                "cargo:rustc-link-search=native={}",
                root.join("lib").display()
            );
            println!("cargo:rustc-link-lib=static=libraw_static");
            println!("cargo:rustc-link-lib=vcomp");
        } else {
            assert!(
                root.join("lib/libraw_r.a").exists(),
                "Run scripts/build-libraw.sh before building with --features raw"
            );
            let openmp = std::path::PathBuf::from(".cache/native-deps/openmp-22.1.8-install")
                .canonicalize()
                .expect("Run scripts/build-openmp.sh before building with --features raw");
            assert!(
                openmp.join("include/omp.h").exists() && openmp.join("lib/libomp.dylib").exists(),
                "Incomplete checkout-local OpenMP runtime"
            );
            println!(
                "cargo:rerun-if-changed={}",
                root.join("lib/libraw_r.a").display()
            );
            println!(
                "cargo:rerun-if-changed={}",
                openmp.join("lib/libomp.dylib").display()
            );
            cpp_bridge()
                .flag("-mmacosx-version-min=14.0")
                .flag("-Xclang")
                .flag("-fopenmp")
                .include(openmp.join("include"))
                .include(&root)
                .file("native/raw_bridge.cpp")
                .compile("antitone_raw_bridge");
            println!(
                "cargo:rustc-link-search=native={}",
                root.join("lib").display()
            );
            println!("cargo:rustc-link-lib=static=raw_r");
            println!("cargo:rustc-link-lib=z");
            println!(
                "cargo:rustc-link-search=native={}",
                openmp.join("lib").display()
            );
            println!("cargo:rustc-link-lib=omp");
            println!(
                "cargo:rustc-link-arg=-Wl,-rpath,{}",
                openmp.join("lib").display()
            );
            println!("cargo:rustc-link-arg=-mmacosx-version-min=14.0");
        }
        #[cfg(feature = "project")]
        {
            for file in [
                "native/scan_regions.cpp",
                "native/scan_region_lines.h",
                "native/scan_region_json.h",
            ] {
                println!("cargo:rerun-if-changed={file}");
            }
            let mut build = cpp_bridge();
            build.include("native").file("native/scan_regions.cpp");
            if target_os == "macos" {
                build.flag("-mmacosx-version-min=14.0");
            }
            build.compile("antitone_scan_regions");
        }
    }
    #[cfg(feature = "dng-sdk")]
    build_dng();
    #[cfg(feature = "ocio-reference")]
    {
        println!("cargo:rerun-if-env-changed=OCIO_PREFIX");
        for file in [
            "native/ocio_bridge.cpp",
            "native/ocio_bridge.h",
            "native/rendering_transform.h",
        ] {
            println!("cargo:rerun-if-changed={file}");
        }
        let root = std::env::var_os("OCIO_PREFIX")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(".cache/deps/ocio-install"));
        let root = root
            .canonicalize()
            .expect("Run scripts/build-ocio.sh before building with --features ocio-reference");
        cpp_bridge()
            .include(root.join("include"))
            .file("native/ocio_bridge.cpp")
            .compile("antitone_ocio_bridge");
        println!(
            "cargo:rustc-link-search=native={}",
            root.join("lib").display()
        );
        println!("cargo:rustc-link-lib=OpenColorIO");
        println!(
            "cargo:rustc-link-arg=-Wl,-rpath,{}",
            root.join("lib").display()
        );
    }
}

/// CMake and Ninja wherever this checkout keeps them, and the MSVC/SDK environment that a
/// plain PowerShell session lacks. `cc` finds the compiler itself, but CMake's resource step
/// needs the Windows SDK's `rc.exe`, which only the developer prompt normally provides.
#[cfg(feature = "dng-sdk")]
fn cmake_tools() -> (std::path::PathBuf, Vec<(String, String)>) {
    let root = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let local = root.join(".cache/tools");

    let on_path = |name: &str| {
        std::process::Command::new(name)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .ok()
            .filter(|status| status.success())
            .map(|_| std::path::PathBuf::from(name))
    };
    let program = std::env::var_os("CMAKE")
        .map(std::path::PathBuf::from)
        .or_else(|| on_path("cmake.exe").or_else(|| on_path("cmake")))
        .or_else(|| {
            std::fs::read_dir(local.join("cmake"))
                .ok()
                .into_iter()
                .flatten()
                .flatten()
                .map(|entry| entry.path().join("bin/cmake.exe"))
                .find(|path| path.exists())
        })
        .unwrap_or_else(|| {
            panic!(
                "CMake was not found. Install it, set CMAKE, or run scripts/setup-windows.ps1 to                  place portable CMake and Ninja in .cache/tools."
            )
        });

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return (program, Vec::new());
    }

    // Ninja must be findable whichever PATH the developer prompt exports.
    let mut environment = developer_environment();
    let mut ninja = Vec::new();
    if on_path("ninja.exe").is_none() {
        ninja.push(local.join("ninja"));
    }
    let base = environment
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("PATH"))
        .map_or_else(
            || std::env::var("PATH").unwrap_or_default(),
            |(_, value)| value.clone(),
        );
    let extra: Vec<String> = ninja
        .into_iter()
        .filter(|directory| directory.exists())
        .map(|directory| directory.display().to_string())
        .collect();
    if !extra.is_empty() {
        environment.retain(|(key, _)| !key.eq_ignore_ascii_case("PATH"));
        environment.push(("PATH".to_owned(), format!("{};{base}", extra.join(";"))));
    }
    (program, environment)
}

/// Variables the Visual Studio developer prompt exports, captured from `VsDevCmd.bat` itself.
#[cfg(feature = "dng-sdk")]
fn developer_environment() -> Vec<(String, String)> {
    let vswhere = std::path::Path::new(&std::env::var("ProgramFiles(x86)").unwrap_or_default())
        .join("Microsoft Visual Studio/Installer/vswhere.exe");
    if !vswhere.exists() {
        return Vec::new();
    }
    let Ok(output) = std::process::Command::new(&vswhere)
        .args([
            "-latest",
            "-products",
            "*",
            "-requires",
            "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
            "-property",
            "installationPath",
        ])
        .output()
    else {
        return Vec::new();
    };
    let installation = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if installation.is_empty() {
        return Vec::new();
    }
    let devcmd = std::path::Path::new(&installation).join("Common7/Tools/VsDevCmd.bat");
    if !devcmd.exists() {
        return Vec::new();
    }
    // A batch file avoids the argument quoting that `cmd /c "<command>"` needs here.
    let script = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("devcmd.bat");
    let contents = format!(
        "@call \"{}\" -arch=x64 -host_arch=x64 >nul\r\n@set\r\n",
        devcmd.display()
    );
    if std::fs::write(&script, contents).is_err() {
        return Vec::new();
    }
    let Ok(captured) = std::process::Command::new("cmd")
        .arg("/c")
        .arg(&script)
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&captured.stdout)
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}

#[cfg(feature = "dng-sdk")]
fn build_dng() {
    use std::{fs, path::PathBuf};

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let source = root.join("vendor/dng-sdk");
    let prefix = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("dng");
    println!("cargo:rerun-if-changed=vendor/dng-sdk");
    println!("cargo:rerun-if-changed=native/dng");
    let defines = fs::read_to_string(root.join("native/dng/defines.txt")).unwrap();
    let compiler = cc::Build::new().cpp(true).get_compiler();
    let c_compiler = cc::Build::new().get_compiler();
    let jobs = std::env::var("NUM_JOBS")
        .ok()
        .and_then(|jobs| jobs.parse::<usize>().ok())
        .unwrap_or(1)
        .min(4);
    let (cmake, environment) = cmake_tools();
    let mut configure = std::process::Command::new(&cmake);
    configure.envs(environment.clone());
    configure.args(["-S", "native/dng", "-B"]).arg(&prefix);
    configure.args(["-G", "Ninja", "-DCMAKE_BUILD_TYPE=Release"]);
    configure.arg(format!(
        "-DCMAKE_C_COMPILER={}",
        c_compiler.path().display()
    ));
    configure.arg(format!(
        "-DCMAKE_CXX_COMPILER={}",
        compiler.path().display()
    ));
    if target_os == "windows" {
        configure.arg("-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreadedDLL");
    } else {
        configure.args([
            "-DCMAKE_OSX_ARCHITECTURES=arm64",
            "-DCMAKE_OSX_DEPLOYMENT_TARGET=14.0",
        ]);
    }
    assert!(
        configure
            .status()
            .expect("CMake could not be started")
            .success()
    );
    assert!(
        std::process::Command::new(&cmake)
            .envs(environment)
            .arg("--build")
            .arg(&prefix)
            .args([
                "--target",
                "antitone_dng_sdk",
                "--parallel",
                &jobs.to_string()
            ])
            .status()
            .expect("Could not build the vendored DNG SDK")
            .success()
    );
    for path in [
        "native/dng_bridge.cpp",
        "native/dng_bridge.h",
        "native/dng_execution.h",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    let mut build = cpp_bridge();
    if target_os == "macos" {
        build.flag("-mmacosx-version-min=14.0");
    } else if target_os == "windows" {
        build.define("NOMINMAX", None);
    }
    let platform = if target_os == "windows" {
        [
            ("WIN_ENV", Some("1")),
            ("Macintosh", Some("0")),
            ("qMacOS", Some("0")),
            ("qWinOS", Some("1")),
        ]
    } else {
        [
            ("MAC_ENV", Some("1")),
            ("Macintosh", Some("1")),
            ("qMacOS", Some("1")),
            ("qWinOS", Some("0")),
        ]
    };
    for (name, value) in platform {
        build.define(name, value);
    }
    for flag in defines.lines() {
        let (name, value) = flag
            .split_once('=')
            .map_or((flag, None), |(n, v)| (n, Some(v)));
        build.define(name, value);
    }
    for directory in [
        "dng_sdk/source",
        "libjpeg",
        "libjxl/libjxl/lib/include",
        "libjxl/client_projects/include",
        "xmp/toolkit/public/include",
        "xmp/toolkit/third-party/zlib",
    ] {
        build.include(source.join(directory));
    }
    build
        .file("native/dng_bridge.cpp")
        .compile("antitone_dng_bridge");
    println!("cargo:rustc-link-search=native={}", prefix.display());
    println!("cargo:rustc-link-lib=static=antitone_dng_sdk");
    if target_os == "macos" {
        println!("cargo:rustc-link-lib=framework=Carbon");
        println!("cargo:rustc-link-arg=-mmacosx-version-min=14.0");
    } else if target_os == "windows" {
        println!("cargo:rustc-link-lib=ws2_32");
        println!("cargo:rustc-link-lib=ole32");
    }
}
