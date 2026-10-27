use super::*;

fn event(logger: &SessionLogger, level: log::Level, target: &str, message: &str) {
    logger.log(
        &Record::builder()
            .level(level)
            .target(target)
            .args(format_args!("{message}"))
            .build(),
    );
}

fn rows(directory: &Path) -> Vec<Value> {
    fs::read_to_string(directory.join("current.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn shutdown_drains_worker_events_and_preserves_typed_fields_and_escaping() {
    let root = tempfile::tempdir().unwrap();
    let session = Session::open(root.path(), LevelFilter::Debug).unwrap();
    let directory = session.directory.clone();

    let fields = [
        ("request", kv::Value::from(42_u64)),
        ("elapsed_ms", kv::Value::from(1.25)),
        ("success", kv::Value::from(true)),
        ("detail", kv::Value::from("line\n\"quoted\"")),
        ("missing", kv::Value::null()),
        ("negative", kv::Value::from(-2_i64)),
    ];

    session.logger.log(
        &Record::builder()
            .level(log::Level::Info)
            .target("antitone::test")
            .args(format_args!("source_ready"))
            .key_values(&fields)
            .build(),
    );

    let logger = session.logger.clone();
    thread::spawn(move || {
        event(
            &logger,
            log::Level::Warn,
            "antitone::worker",
            "worker_failure",
        )
    })
    .join()
    .unwrap();

    drop(session);

    let rows = rows(&directory);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["fields"]["request"], 42);
    assert_eq!(rows[0]["fields"]["elapsed_ms"], 1.25);
    assert_eq!(rows[0]["fields"]["success"], true);
    assert_eq!(rows[0]["fields"]["missing"], Value::Null);
    assert_eq!(rows[0]["fields"]["negative"], -2);
    assert_eq!(rows[0]["fields"]["detail"], "line\n\"quoted\"");

    assert_eq!(rows[0]["event"], "source_ready");
    assert_eq!(rows[1]["event"], "worker_failure");
    assert_eq!(rows[1]["level"], "WARN");
    assert_ne!(rows[0]["thread"], rows[1]["thread"]);
    assert!(rows[0]["timestamp_ms"].as_u64().unwrap() > 0);
}

#[test]
fn app_filter_is_distinct_from_dependency_filter_and_flush_is_a_barrier() {
    let root = tempfile::tempdir().unwrap();
    let session = Session::open(root.path(), LevelFilter::Debug).unwrap();

    event(
        &session.logger,
        log::Level::Debug,
        "antitone::source",
        "app_debug",
    );
    event(
        &session.logger,
        log::Level::Trace,
        "antitone::source",
        "app_trace",
    );
    event(
        &session.logger,
        log::Level::Info,
        "wgpu_core",
        "dependency_info",
    );
    event(
        &session.logger,
        log::Level::Warn,
        "wgpu_core",
        "dependency_warn",
    );
    event(
        &session.logger,
        log::Level::Debug,
        "antitone_extra",
        "not_app",
    );
    session.logger.flush();

    let names: Vec<_> = rows(session.directory())
        .into_iter()
        .map(|row| row["event"].clone())
        .collect();
    assert_eq!(names, vec![json!("app_debug"), json!("dependency_warn")]);
}

#[test]
fn rolling_files_bound_storage_and_keep_the_latest_complete_records() {
    let root = tempfile::tempdir().unwrap();
    let mut sink = Sink::open(root.path(), 40, 3).unwrap();

    for n in 0..10 {
        sink.write(format!("{{\"sequence\":{n},\"value\":12345}}\n").as_bytes())
            .unwrap();
    }

    drop(sink);

    assert_eq!(rows(root.path())[0]["sequence"], 9);
    let previous: Value = serde_json::from_str(
        fs::read_to_string(root.path().join("1.jsonl"))
            .unwrap()
            .trim(),
    )
    .unwrap();
    let oldest: Value = serde_json::from_str(
        fs::read_to_string(root.path().join("2.jsonl"))
            .unwrap()
            .trim(),
    )
    .unwrap();
    assert_eq!(previous["sequence"], 8);
    assert_eq!(oldest["sequence"], 7);

    let files: Vec<_> = fs::read_dir(root.path())
        .unwrap()
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "jsonl"))
        .collect();
    assert_eq!(files.len(), 3);
    assert!(
        files
            .iter()
            .all(|entry| entry.metadata().unwrap().len() <= 40)
    );
}

#[test]
fn retention_protects_active_sessions_and_unrelated_directories() {
    let root = tempfile::tempdir().unwrap();
    let _retention = retention_lock(root.path()).unwrap();

    let active = root.path().join("session-000-active");
    fs::create_dir(&active).unwrap();
    let _active = Sink::open(&active, FILE_BYTES, SESSION_FILES).unwrap();

    for n in 1..8 {
        let path = root.path().join(format!("session-{n:03}-closed"));
        fs::create_dir(&path).unwrap();
        drop(Sink::open(&path, FILE_BYTES, SESSION_FILES).unwrap());
    }

    let unrelated = root.path().join("session-user-notes");
    fs::create_dir(&unrelated).unwrap();

    let current = root.path().join("session-999-current");
    fs::create_dir(&current).unwrap();
    let _current = Sink::open(&current, FILE_BYTES, SESSION_FILES).unwrap();

    prune(root.path(), &current).unwrap();

    let retained = fs::read_dir(root.path())
        .unwrap()
        .flatten()
        .filter(|entry| entry.path().join(".active").is_file())
        .count();
    assert_eq!(retained, RETAINED_SESSIONS);
    assert!(active.join("current.jsonl").exists());
    assert!(unrelated.exists());
    assert!(!root.path().join("session-001-closed").exists());
    assert!(
        root.path()
            .join("session-007-closed/current.jsonl")
            .exists()
    );
}

#[test]
fn saturated_producers_do_not_block_and_loss_is_reported() {
    let root = tempfile::tempdir().unwrap();
    let (send, receive) = mpsc::sync_channel(1);
    let dropped = Arc::new(AtomicU64::new(0));
    let logger = SessionLogger {
        level: LevelFilter::Info,
        send,
        dropped: dropped.clone(),
    };

    event(&logger, log::Level::Info, "antitone", "first");
    event(&logger, log::Level::Info, "antitone", "overflow");
    assert_eq!(dropped.load(Ordering::Relaxed), 1);

    let sink = Sink::open(root.path(), FILE_BYTES, SESSION_FILES).unwrap();
    let worker = thread::spawn(move || {
        let mut console = Vec::new();
        run_worker(sink, receive, dropped, &mut console);
        String::from_utf8(console).unwrap()
    });

    logger.send.send(Command::Stop).unwrap();
    let console = worker.join().unwrap();

    assert!(console.contains("records_dropped count=1"));
    assert!(
        rows(root.path())
            .iter()
            .any(|row| row["event"] == "records_dropped" && row["fields"]["count"] == 1)
    );
}

#[test]
fn oversized_records_remain_valid_json_and_output_is_private() {
    let root = tempfile::tempdir().unwrap();
    let session = Session::open(root.path(), LevelFilter::Info).unwrap();

    event(
        &session.logger,
        log::Level::Info,
        "antitone",
        &"x".repeat(RECORD_BYTES * 2),
    );
    session.logger.flush();

    assert_eq!(
        rows(session.directory())[0]["event"],
        "oversized_record_discarded"
    );
    assert!(
        fs::metadata(session.directory().join("current.jsonl"))
            .unwrap()
            .len()
            < RECORD_BYTES as u64
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(session.directory())
                .unwrap()
                .permissions()
                .mode()
                & 0o077,
            0
        );
        assert_eq!(
            fs::metadata(session.directory().join("current.jsonl"))
                .unwrap()
                .permissions()
                .mode()
                & 0o077,
            0
        );
    }
}

#[test]
fn failed_file_writes_leave_console_logging_and_flush_available() {
    let root = tempfile::tempdir().unwrap();
    let mut sink = Sink::open(root.path(), FILE_BYTES, SESSION_FILES).unwrap();
    sink.file = File::open(root.path().join("current.jsonl")).unwrap();

    let (send, receive) = mpsc::sync_channel(4);
    let dropped = Arc::new(AtomicU64::new(0));
    let logger = SessionLogger {
        level: LevelFilter::Info,
        send,
        dropped: dropped.clone(),
    };

    event(&logger, Level::Info, "antitone", "write_fails");
    event(&logger, Level::Info, "antitone", "after_file_failure");

    let (flushed, acknowledged) = mpsc::channel();
    logger.send.send(Command::Flush(flushed)).unwrap();
    logger.send.send(Command::Stop).unwrap();

    let mut console = Vec::new();
    run_worker(sink, receive, dropped, &mut console);
    acknowledged.try_recv().unwrap();

    let console = String::from_utf8(console).unwrap();
    assert!(console.contains("write_fails"));
    assert!(console.contains("after_file_failure"));
    assert_eq!(console.matches("session file logging stopped").count(), 1);
    assert!(rows(root.path()).is_empty());
}

#[test]
fn failed_console_writes_and_flush_do_not_interrupt_file_capture() {
    struct UnavailableConsole;

    impl Write for UnavailableConsole {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }
    }

    let root = tempfile::tempdir().unwrap();
    let sink = Sink::open(root.path(), FILE_BYTES, SESSION_FILES).unwrap();
    let (send, receive) = mpsc::sync_channel(4);
    let dropped = Arc::new(AtomicU64::new(0));
    let logger = SessionLogger {
        level: LevelFilter::Info,
        send,
        dropped: dropped.clone(),
    };

    event(&logger, Level::Info, "antitone", "first");
    event(&logger, Level::Info, "antitone", "after_console_failure");

    let (flushed, acknowledged) = mpsc::channel();
    logger.send.send(Command::Flush(flushed)).unwrap();
    logger.send.send(Command::Stop).unwrap();

    run_worker(sink, receive, dropped, UnavailableConsole);
    acknowledged.try_recv().unwrap();

    let rows = rows(root.path());
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["event"], "first");
    assert_eq!(rows[1]["event"], "after_console_failure");
}

#[test]
fn stalled_console_has_bounded_flush_and_cannot_hold_file_capture_or_shutdown() {
    struct StalledConsole {
        entered: Option<mpsc::Sender<()>>,
        release: mpsc::Receiver<()>,
        retired: mpsc::Sender<()>,
    }

    impl Write for StalledConsole {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if let Some(entered) = self.entered.take() {
                let _ = entered.send(());
                let _ = self.release.recv();
            }

            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Drop for StalledConsole {
        fn drop(&mut self) {
            let _ = self.retired.send(());
        }
    }

    let (entered, writing) = mpsc::channel();
    let (release, waiting) = mpsc::channel();
    let (retired, retirement) = mpsc::channel();
    let mut console = Console::new(
        StalledConsole {
            entered: Some(entered),
            release: waiting,
            retired,
        },
        1,
    )
    .unwrap();

    console.write_all(b"stalled\n").unwrap();
    writing.recv_timeout(Duration::from_secs(2)).unwrap();

    let (checked, result) = mpsc::channel();
    thread::spawn(move || {
        let timeout = console.flush().unwrap_err().kind();
        let full = console.write_all(b"overflow\n").unwrap_err().kind();
        let _ = checked.send((console, timeout, full));
    });

    let (console, timeout, full) = result
        .recv_timeout(Duration::from_secs(2))
        .expect("console flush/enqueue must not wait for the stalled writer");
    assert_eq!(timeout, io::ErrorKind::TimedOut);
    assert_eq!(full, io::ErrorKind::WouldBlock);

    let root = tempfile::tempdir().unwrap();
    let sink = Sink::open(root.path(), FILE_BYTES, SESSION_FILES).unwrap();
    let (send, receive) = mpsc::sync_channel(4);
    let dropped = Arc::new(AtomicU64::new(0));
    let logger = Arc::new(SessionLogger {
        level: LevelFilter::Info,
        send,
        dropped: dropped.clone(),
    });
    let worker = thread::spawn(move || run_worker(sink, receive, dropped, console));
    let session = Session {
        directory: root.path().into(),
        logger: logger.clone(),
        worker: Some(worker),
    };

    event(
        &logger,
        Level::Info,
        "antitone",
        "file_survives_stderr_backpressure",
    );

    let (flushed, flush_done) = mpsc::channel();
    let (stopped, stop_done) = mpsc::channel();
    let shutdown = thread::spawn(move || {
        logger.flush();
        let _ = flushed.send(());
        drop(session);
        let _ = stopped.send(());
    });

    flush_done
        .recv_timeout(Duration::from_secs(2))
        .expect("file flush was held by stderr");
    stop_done
        .recv_timeout(Duration::from_secs(2))
        .expect("session shutdown was held by stderr");
    shutdown.join().unwrap();
    assert_eq!(
        rows(root.path())[0]["event"],
        "file_survives_stderr_backpressure"
    );

    release.send(()).unwrap();
    retirement.recv_timeout(Duration::from_secs(2)).unwrap();
}

#[test]
fn console_format_keeps_levels_targets_and_fields_readable_on_one_line() {
    let fields = Map::from_iter([
        ("request".into(), json!(7)),
        ("success".into(), json!(true)),
        ("reason".into(), json!("line\n\"quoted\"")),
    ]);
    let entry = Entry::new(
        Level::Debug,
        "antitone::source",
        "source_ready\nnext",
        fields,
    );
    let console = String::from_utf8(entry.text).unwrap();

    assert!(console.contains("DEBUG"));
    assert!(console.contains("antitone::source"));
    assert!(console.contains("source_ready\\nnext"));
    assert!(console.contains("request=7"));
    assert!(console.contains("success=true"));
    assert!(console.contains("reason=\"line\\n\\\"quoted\\\"\""));
    assert_eq!(console.lines().count(), 1);
    assert!(console.ends_with('\n'));
}

#[test]
fn unwritable_log_location_is_a_reportable_error() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("file-not-directory");
    fs::write(&path, "do not overwrite").unwrap();

    assert!(Session::open(&path, LevelFilter::Info).is_err());
    assert_eq!(fs::read_to_string(path).unwrap(), "do not overwrite");
}
