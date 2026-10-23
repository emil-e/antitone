fn main() {
    println!("cargo:rerun-if-env-changed=LIBRAW_SOURCE");
    println!("cargo:rerun-if-changed=native/raw_bridge.cpp");
    println!("cargo:rerun-if-changed=native/raw_bridge.h");
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
            cc::Build::new()
                .cpp(true)
                .std("c++17")
                .define("WIN32", None)
                .define("LIBRAW_NODLL", None)
                .flag("/EHsc")
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
            cc::Build::new()
                .cpp(true)
                .std("c++17")
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
            let mut build = cc::Build::new();
            build
                .cpp(true)
                .std("c++17")
                .include("native")
                .file("native/scan_regions.cpp");
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
        cc::Build::new()
            .cpp(true)
            .std("c++17")
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
    let mut configure = std::process::Command::new("cmake");
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
            .expect("CMake is required; see README.md")
            .success()
    );
    assert!(
        std::process::Command::new("cmake")
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
    let mut build = cc::Build::new();
    build.cpp(true).std("c++17");
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
    }
}
