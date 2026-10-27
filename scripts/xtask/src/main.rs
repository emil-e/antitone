use std::{env, error::Error, fs, path::Path, process::Command};

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

fn run(program: &str, args: &[&str]) -> Result {
    println!("+ {program} {}", args.join(" "));
    if !Command::new(program).args(args).status()?.success() {
        return Err(format!("{program} failed").into());
    }
    Ok(())
}

fn cargo(args: &[&str]) -> Result {
    run(&env::var("CARGO").unwrap_or_else(|_| "cargo".into()), args)
}

fn prerequisites() -> Result {
    if env::consts::OS != "macos" || env::consts::ARCH != "aarch64" {
        return Err("Local editor delivery currently supports Apple Silicon macOS only".into());
    }
    for (tool, args) in [
        ("git", vec!["--version"]),
        ("rustc", vec!["--version"]),
        ("python3", vec!["--version"]),
        ("cmake", vec!["--version"]),
        ("ninja", vec!["--version"]),
        ("curl", vec!["--version"]),
        ("xcrun", vec!["--find", "clang"]),
        ("xcrun", vec!["--show-sdk-path"]),
        ("xcrun", vec!["--find", "swift-format"]),
    ] {
        run(tool, &args).map_err(|error| format!("Prerequisite check: {error}. See README.md"))?;
    }
    Ok(())
}

fn install_tools() -> Result {
    let tools = Path::new(".cache/format-tools");
    let venv = tools.join("venv");
    fs::create_dir_all(tools)?;
    // A restored cache can carry a virtual environment whose interpreter no longer
    // exists on this runner image; recreate it rather than fail when the formatter runs.
    let venv_usable = Command::new(venv.join("bin/python"))
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    if !venv_usable {
        fs::remove_dir_all(&venv).ok();
        run("python3", &["-m", "venv", ".cache/format-tools/venv"])?;
    }
    if !(tools.join("venv/bin/ruff").exists() && tools.join("venv/bin/clang-format").exists()) {
        run(
            ".cache/format-tools/venv/bin/python",
            &[
                "-m",
                "pip",
                "install",
                "ruff==0.17.1",
                "clang-format==22.1.8",
            ],
        )?;
    }
    if !tools.join("bin/wgslfmt").exists() {
        cargo(&[
            "install",
            "--git",
            "https://github.com/wgsl-analyzer/wgsl-analyzer",
            "--rev",
            "58a9aba79a0e0f9d3fef12a5ef0e577b3c4a2ae4",
            "--locked",
            "--root",
            ".cache/format-tools",
            "wgslfmt",
        ])?;
    }
    cargo(&["fetch", "--locked"])
}

fn tools() -> Result {
    prerequisites()?;
    install_tools()
}

fn setup_native() -> Result {
    prerequisites()?;
    run("sh", &["scripts/setup-native.sh"])
}

fn setup() -> Result {
    setup_native()?;
    install_tools()
}

fn check() -> Result {
    run("python3", &["scripts/check-format.py"])?;
    run(
        "python3",
        &[
            "-m",
            "unittest",
            "discover",
            "-s",
            "scripts",
            "-p",
            "test_*.py",
        ],
    )?;
    cargo(&["test", "--locked", "--package", "xtask"])?;
    cargo(&["test", "--locked", "--no-default-features"])?;
    // Separate production from reference-feature unification: OCIO must not
    // accidentally become a production link/runtime prerequisite.
    cargo(&["test", "--locked", "--features", "editor"])?;
    cargo(&[
        "clippy",
        "--locked",
        "--features",
        "editor",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ])?;
    cargo(&["test", "--locked", "--all-features"])?;
    cargo(&[
        "clippy",
        "--locked",
        "--all-features",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ])?;
    cargo(&[
        "test",
        "--locked",
        "--release",
        "--features",
        "ui-tests",
        "--lib",
        "editor::kittest_tests",
    ])
}

fn build() -> Result {
    cargo(&[
        "build",
        "--locked",
        "--release",
        "--target",
        "aarch64-apple-darwin",
        "--features",
        "editor",
        "--bin",
        "antitone",
    ])
}

fn package() -> Result {
    build()?;
    run("python3", &["scripts/package-macos.py"])
}

fn verify() -> Result {
    run("python3", &["scripts/package-macos.py", "--verify"])
}

fn main() -> Result {
    env::set_current_dir(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .unwrap(),
    )?;
    let args: Vec<_> = env::args().skip(1).collect();
    let action = args.first().map(String::as_str).unwrap_or("help");
    if args.len() > 1 {
        return Err("Expected one command; run cargo xtask help".into());
    }
    match action {
        "setup" => setup(),
        "setup-native" => setup_native(),
        "tools" => tools(),
        "doctor" => prerequisites(),
        "check" => check(),
        "build" => build(),
        "package" => package(),
        "verify-package" => verify(),
        "all" => {
            setup()?;
            check()?;
            package()?;
            verify()
        }
        "help" | "--help" | "-h" => {
            println!(
                "cargo xtask <doctor|setup|setup-native|tools|check|build|package|verify-package|all>\n\nsetup downloads/builds pinned dependencies and installs formatters locally.\nsetup-native downloads/builds only the pinned native dependencies, without the formatters.\ntools installs only the pinned formatting tools and warms Cargo downloads; no native build.\ncheck requires native setup and Metal, but opens no desktop windows.\npackage builds the production editor and creates a local .app and ZIP.\nverify-package extracts the ZIP, audits its runtime paths/signature and runs it.\nall performs setup, check, package and verification. No system packages are installed."
            );
            Ok(())
        }
        _ => Err(format!("Unknown command {action:?}; run cargo xtask help").into()),
    }
}
