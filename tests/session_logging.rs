#![cfg(feature = "logging")]
use serde_json::Value;
use std::{fs, process::Command};

#[test]
fn captures_a_process_session_including_workers_panics_and_native_failures() {
    let root = tempfile::tempdir().unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "session_child", "--nocapture"])
        .env("ANTITONE_LOG_DIR", root.path())
        .env("ANTITONE_LOG", "debug")
        .env("ANTITONE_LOG_TEST_CHILD", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let directories: Vec<_> = fs::read_dir(root.path())
        .unwrap()
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .collect();
    assert_eq!(directories.len(), 1);

    let text = fs::read_to_string(directories[0].path().join("current.jsonl")).unwrap();
    let rows: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let event = |name: &str| {
        rows.iter()
            .find(|row| row["event"] == name)
            .unwrap()
            .clone()
    };

    assert_eq!(
        event("session_started")["fields"]["version"],
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(event("worker_done")["fields"]["request"], 7);
    assert_eq!(event("panic")["level"], "ERROR");
    assert!(
        event("panic")["fields"]["details"]
            .as_str()
            .unwrap()
            .contains("diagnostic panic")
    );
    assert_eq!(rows.last().unwrap()["event"], "session_finished");
    assert_eq!(event("debug_detail")["level"], "DEBUG");
    assert_eq!(event("dependency_warning")["level"], "WARN");
    assert!(
        rows.iter()
            .all(|row| row["event"] != "hidden_trace" && row["event"] != "dependency_info")
    );

    let console = String::from_utf8_lossy(&output.stderr);
    let worker_line = console
        .lines()
        .find(|line| line.contains("worker_done"))
        .unwrap();

    assert!(worker_line.contains("INFO"));
    assert!(worker_line.contains("antitone::test"));
    assert!(worker_line.contains("request=7"));
    assert!(console.contains("debug_detail"));
    assert!(console.contains("dependency_warning"));
    assert!(!console.contains("hidden_trace"));
    assert!(!console.contains("dependency_info"));

    #[cfg(feature = "raw")]
    assert!(rows.iter().any(|row| {
        row["target"] == "antitone::native"
            && row["fields"]["component"] == "raw"
            && row["fields"]["operation"] == "open"
            && row["fields"]["status"]
                .as_i64()
                .is_some_and(|status| status != 0)
            && row["level"] == "WARN"
            && row["fields"]["elapsed_ms"]
                .as_f64()
                .is_some_and(|ms| ms >= 0.)
    }));
    #[cfg(feature = "raw")]
    assert!(rows.iter().any(|row| row["target"] == "antitone::native"
        && row["fields"]["component"] == "raw"
        && row["fields"]["operation"] == "open"
        && row["fields"]["status"] == 0
        && row["level"] == "DEBUG"));
    #[cfg(feature = "ocio-reference")]
    assert!(rows.iter().any(|row| row["target"] == "antitone::native"
        && row["fields"]["component"] == "ocio"
        && row["fields"]["operation"] == "create"
        && row["fields"]["status"] == 0));
    #[cfg(feature = "dng-sdk")]
    assert!(rows.iter().any(|row| row["target"] == "antitone::native"
        && row["fields"]["component"] == "dng"
        && row["fields"]["operation"] == "inspect"
        && row["level"] == "WARN"));
}

#[test]
fn unavailable_log_directory_falls_back_without_failing_the_process() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("not-a-directory");
    fs::write(&path, "preserve").unwrap();

    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "session_child", "--nocapture"])
        .env("ANTITONE_LOG_DIR", &path)
        .env("ANTITONE_LOG", "info")
        .env("ANTITONE_LOG_TEST_CHILD", "fallback")
        .output()
        .unwrap();
    assert!(output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("session log unavailable"));
    assert!(stderr.contains("fallback_failure"));
    assert!(stderr.contains("diagnostic reason"));
    assert!(stderr.contains("fallback_info"));
    assert!(!stderr.contains("fallback_debug_hidden"));
    assert_eq!(fs::read_to_string(path).unwrap(), "preserve");
}

#[test]
fn concurrent_process_startups_do_not_over_prune_history() {
    use std::time::{Duration, Instant};

    let root = tempfile::tempdir().unwrap();

    for n in 0..5 {
        let old = root.path().join(format!("session-000{n}-closed"));
        fs::create_dir(&old).unwrap();
        fs::write(old.join(".active"), "").unwrap();
        fs::write(old.join("current.jsonl"), "").unwrap();
    }

    let mut children: Vec<_> = (0..8)
        .map(|_| {
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "session_child", "--nocapture"])
                .env("ANTITONE_LOG_DIR", root.path())
                .env("ANTITONE_LOG", "info")
                .env("ANTITONE_LOG_TEST_CHILD", "retention")
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap()
        })
        .collect();

    let start = Instant::now();
    while fs::read_dir(root.path())
        .unwrap()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("ready-"))
        .count()
        < children.len()
    {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "startup barrier timed out"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    fs::write(root.path().join("go"), "").unwrap();

    for child in &mut children {
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            if start.elapsed() > Duration::from_secs(15) {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("logging child hung");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    let count = fs::read_dir(root.path())
        .unwrap()
        .flatten()
        .filter(|entry| entry.path().join(".active").is_file())
        .count();
    // Sessions still locked at the last startup can legitimately leave more than five.
    assert!(
        count >= 5,
        "competing pruners must not each delete a stale quota"
    );
}

// Global log/panic hooks are exercised in a separate process, not shared with other tests.
#[test]
fn session_child() {
    if std::env::var_os("ANTITONE_LOG_TEST_CHILD").is_none() {
        return;
    }

    if std::env::var("ANTITONE_LOG_TEST_CHILD").unwrap() == "fallback" {
        assert!(antitone::logging::init().is_none());
        log::error!(error = "diagnostic reason"; "fallback_failure");
        log::info!(target: "antitone::test", "fallback_info");
        log::debug!(target: "antitone::test", "fallback_debug_hidden");

        return;
    }

    if std::env::var("ANTITONE_LOG_TEST_CHILD").unwrap() == "retention" {
        let root = std::path::PathBuf::from(std::env::var_os("ANTITONE_LOG_DIR").unwrap());
        fs::write(root.join(format!("ready-{}", std::process::id())), "").unwrap();

        let start = std::time::Instant::now();
        while !root.join("go").exists() {
            assert!(start.elapsed() < std::time::Duration::from_secs(10));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let _session = antitone::logging::Session::from_env().unwrap();

        return;
    }

    let _session = antitone::logging::Session::from_env().unwrap();

    log::debug!(target: "antitone::test", "debug_detail");
    log::trace!(target: "antitone::test", "hidden_trace");
    log::info!(target: "dependency", "dependency_info");
    log::warn!(target: "dependency", "dependency_warning");

    std::thread::spawn(|| {
        log::info!(
            target: "antitone::test",
            request = 7;
            "worker_done"
        )
    })
    .join()
    .unwrap();

    #[cfg(feature = "raw")]
    assert!(
        antitone::raw::RawFile::open(std::path::Path::new("/nonexistent/antitone-log-test.ARW"))
            .is_err()
    );
    #[cfg(feature = "dng-sdk")]
    assert!(
        antitone::dng::Header::read(
            std::path::Path::new("/nonexistent/antitone-log-test.dng"),
            antitone::dng::Options {
                max_pixels: 1024,
                fixed_wb: None
            },
            &antitone::dng::Cancellation::default()
        )
        .is_err()
    );
    #[cfg(feature = "raw")]
    {
        let fixture =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng");
        let mut raw = antitone::raw::RawFile::open(&fixture).unwrap();
        let wb = raw.metadata.as_shot_wb();
        raw.develop_camera(wb).unwrap();
    }

    #[cfg(feature = "ocio-reference")]
    {
        let _renderer =
            antitone::rendering::CpuRenderer::new(antitone::rendering::OutputTarget::Srgb).unwrap();
    }
    assert!(std::panic::catch_unwind(|| panic!("diagnostic panic")).is_err());

    log::info!(target: "antitone::test", "session_finished");
}
