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
    // Explorer and the taskbar take the executable's icon and version details from its resources.
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=native/antitone.rc");
        embed_resource::compile("native/antitone.rc", embed_resource::NONE)
            .manifest_required()
            .expect("Could not embed the Windows application resources");
    }
    println!("cargo:rerun-if-env-changed=LIBRAW_SOURCE");
    println!("cargo:rerun-if-changed=native/raw_bridge.cpp");
    println!("cargo:rerun-if-changed=native/raw_bridge.h");
    println!("cargo:rerun-if-changed=native/logging.h");
    println!("cargo:rerun-if-changed=native/highlights.h");
    #[cfg(feature = "raw")]
    build_raw();
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

#[cfg(feature = "raw")]
fn native_pin(path: &str, key: &str) -> String {
    println!("cargo:rerun-if-changed={path}");
    let text = std::fs::read_to_string(path).expect("native dependency pins");
    text.lines()
        .filter_map(|line| line.split_once('='))
        .find(|(name, _)| *name == key)
        .unwrap_or_else(|| panic!("Missing {key} in {path}"))
        .1
        .to_owned()
}

#[cfg(feature = "raw")]
fn build_raw() {
    use std::{fs, path::PathBuf};
    println!("cargo:rerun-if-changed=native/rawspeed");
    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let revision = native_pin("native/libraw/dht-hashes.txt", "patched");
    let speed_revision = native_pin("native/rawspeed/pins.txt", "revision");
    for (pin, length) in [(&revision, 64), (&speed_revision, 40)] {
        assert!(pin.len() == length && pin.bytes().all(|c| c.is_ascii_hexdigit()));
    }
    let omp_version = native_pin("native/openmp/pins.txt", "version");
    let cache = PathBuf::from(".cache/native-deps");
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86_64") => "x64",
        Ok("aarch64") => "arm64",
        _ => panic!("RAW native setup supports x64 and arm64 targets"),
    };
    let platform = if windows {
        format!("llvm-{arch}")
    } else {
        "openmp".to_owned()
    };
    // Clang include paths need ordinary Win32 paths, not canonicalize's verbatim prefixes.
    let root = dunce::canonicalize(
        std::env::var_os("LIBRAW_SOURCE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                cache.join(format!(
                    "LibRaw-0.22.3-{}-rawspeed-{platform}",
                    &revision[..12]
                ))
            }),
    )
    .expect("Run scripts/build-libraw.sh (macOS) or scripts/build-libraw.ps1 (Windows)");
    let speed_platform = if windows {
        format!("windows-{arch}")
    } else {
        "macos-arm64".to_owned()
    };
    let speed = dunce::canonicalize(cache.join(format!(
        "rawspeed-{}-{speed_platform}",
        &speed_revision[..12]
    )))
    .expect("Run the checkout's LibRaw build script to prepare RawSpeed");
    let omp_directory = if windows {
        format!("openmp-{omp_version}-windows-{arch}-install")
    } else {
        format!("openmp-{omp_version}-install")
    };
    let openmp = dunce::canonicalize(cache.join(omp_directory))
        .expect("Run the checkout's LibRaw build script to prepare LLVM OpenMP");
    let features = root.join("antitone-features.txt");
    assert_eq!(
        fs::read_to_string(&features).unwrap_or_default().trim(),
        format!("rawspeed3;llvm-openmp-{omp_version}"),
        "LibRaw lacks the pinned RAW build features; rerun its checkout build script"
    );
    let libraw = if windows {
        "libraw_static.lib"
    } else {
        "libraw_r.a"
    };
    let speed_lib = if windows {
        "rawspeed.lib"
    } else {
        "librawspeed.a"
    };
    let pugi_lib = if windows {
        "pugixml.lib"
    } else {
        "libpugixml.a"
    };
    let runtime = openmp.join(if windows {
        "bin/libomp.dll"
    } else {
        "lib/libomp.dylib"
    });
    for file in [
        features,
        root.join("lib").join(libraw),
        speed.join("lib").join(speed_lib),
        speed.join("lib").join(pugi_lib),
        speed.join("include/antitone_rawspeed_camera_db.h"),
        openmp.join("include/omp.h"),
        runtime.clone(),
    ] {
        assert!(
            file.exists(),
            "Incomplete native dependency: {}",
            file.display()
        );
        println!("cargo:rerun-if-changed={}", file.display());
    }
    let mut bridge = cpp_bridge();
    if windows {
        let compiler_file = speed.join("share/antitone/compiler.txt");
        println!("cargo:rerun-if-changed={}", compiler_file.display());
        let compiler = fs::read_to_string(compiler_file).expect("RawSpeed compiler identity");
        let clang = PathBuf::from(compiler.trim()).with_file_name("clang-cl.exe");
        assert!(
            clang.exists(),
            "RawSpeed's clang-cl is missing; rerun Windows setup"
        );
        bridge
            .compiler(clang)
            .define("WIN32", None)
            .define("LIBRAW_NODLL", None)
            .flag("/clang:-fopenmp");
        for (key, value) in developer_environment() {
            bridge.env(key, value);
        }
        // A plain PowerShell cargo test/run must find the same DLL as the release
        // package. Do not depend on a system-installed or VS-bundled OpenMP runtime.
        let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
        let profile = out
            .ancestors()
            .nth(3)
            .expect("Cargo profile output directory");
        let bytes = fs::read(&runtime).expect("Pinned LLVM OpenMP DLL");
        for directory in [profile.to_owned(), profile.join("deps")] {
            fs::create_dir_all(&directory).unwrap();
            let target = directory.join("libomp.dll");
            if fs::read(&target).ok().as_deref() != Some(bytes.as_slice()) {
                fs::copy(&runtime, &target).unwrap_or_else(|error| {
                    panic!(
                        "Cannot stage pinned {}: {error}; close any running Antitone first",
                        target.display()
                    )
                });
            }
        }
    } else {
        bridge
            .flag("-mmacosx-version-min=14.0")
            .flag("-Xclang")
            .flag("-fopenmp");
    }
    bridge.include(openmp.join("include"));
    let mut adapter = bridge.clone();
    bridge
        .include(&root)
        .file("native/raw_bridge.cpp")
        .compile("antitone_raw_bridge");
    adapter
        .std("c++20")
        .include(speed.join("include"))
        .include(speed.join("include/rawspeed"))
        .file("native/rawspeed/adapter.cpp")
        .compile("antitone_rawspeed_adapter");
    for directory in [root.join("lib"), speed.join("lib"), openmp.join("lib")] {
        println!("cargo:rustc-link-search=native={}", directory.display());
    }
    if windows {
        println!("cargo:rustc-link-lib=static=libraw_static");
        println!("cargo:rustc-link-lib=dylib=libomp");
    } else {
        println!("cargo:rustc-link-lib=static=raw_r");
        println!("cargo:rustc-link-lib=z");
        println!("cargo:rustc-link-lib=dylib=omp");
        println!(
            "cargo:rustc-link-arg=-Wl,-rpath,{}",
            openmp.join("lib").display()
        );
        println!("cargo:rustc-link-arg=-mmacosx-version-min=14.0");
    }
    println!("cargo:rustc-link-lib=static=rawspeed");
    println!("cargo:rustc-link-lib=static=pugixml");
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
#[cfg(any(feature = "raw", feature = "dng-sdk"))]
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
        "native/dng_highlights.h",
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
