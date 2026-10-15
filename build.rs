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
    #[cfg(feature = "render")]
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
            .expect("Run scripts/build-ocio.sh before building with --features render");
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
