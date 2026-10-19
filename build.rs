fn main() {
    println!("cargo:rerun-if-env-changed=LIBRAW_SOURCE");
    println!("cargo:rerun-if-changed=native/raw_bridge.cpp");
    println!("cargo:rerun-if-changed=native/raw_bridge.h");
    #[cfg(feature = "raw")]
    {
        let root = std::env::var_os("LIBRAW_SOURCE")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(".cache/native-deps/LibRaw-0.22.0-openmp"));
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
        #[cfg(feature = "project")]
        {
            for file in ["native/scan_regions.mm", "native/scan_region_lines.h"] {
                println!("cargo:rerun-if-changed={file}");
            }
            cc::Build::new()
                .cpp(true)
                .std("c++17")
                .flag("-fobjc-arc")
                .flag("-mmacosx-version-min=14.0")
                .file("native/scan_regions.mm")
                .compile("antitone_scan_regions");
            println!("cargo:rustc-link-lib=framework=Foundation");
        }
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

    assert_eq!(std::env::var("CARGO_CFG_TARGET_OS").unwrap(), "macos");
    assert_eq!(std::env::var("CARGO_CFG_TARGET_ARCH").unwrap(), "aarch64");
    println!("cargo:rerun-if-env-changed=DNG_SDK_PREFIX");
    let prefix = std::env::var_os("DNG_SDK_PREFIX")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".cache/dng-sdk"));
    let source_file = prefix.join("source-path.txt");
    let defines_file = prefix.join("defines.txt");
    let source = PathBuf::from(
        fs::read_to_string(&source_file)
            .expect("Prepare the opt-in SDK with scripts/build-dng-sdk.py; see README.md")
            .trim(),
    );
    let defines = fs::read_to_string(&defines_file).expect("Incomplete SDK build metadata");
    for path in [
        source_file,
        defines_file,
        prefix.join("lib/libantitone_dng_sdk.a"),
    ] {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    for path in [
        "native/dng_bridge.cpp",
        "native/dng_bridge.h",
        "native/dng_execution.h",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .flag("-mmacosx-version-min=14.0");
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
    println!(
        "cargo:rustc-link-search=native={}",
        prefix.join("lib").display()
    );
    println!("cargo:rustc-link-lib=static=antitone_dng_sdk");
    println!("cargo:rustc-link-lib=framework=Carbon");
    println!("cargo:rustc-link-arg=-mmacosx-version-min=14.0");
}
