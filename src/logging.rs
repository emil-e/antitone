//! Session files and readable stderr share a bounded background writer.

use log::{Level, LevelFilter, Log, Metadata, Record, kv};
use serde_json::{Map, Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const FILE_BYTES: u64 = 8 * 1024 * 1024;
const SESSION_FILES: usize = 3;
const RETAINED_SESSIONS: usize = 5;
const QUEUE_RECORDS: usize = 1024;
const RECORD_BYTES: usize = 16 * 1024;
const CONSOLE_FLUSH_WAIT: Duration = Duration::from_millis(250);

/// Start session-file and stderr logging, falling back to stderr alone on failure.
/// Hold the returned guard until application shutdown.
pub fn init() -> Option<Session> {
    match Session::from_env() {
        Ok(session) => Some(session),
        Err(error) => {
            let _ = writeln!(
                io::stderr(),
                "Antitone session log unavailable; using stderr: {error}"
            );

            let level = level_from_env().unwrap_or(LevelFilter::Info);
            let logger = ConsoleLogger { level };

            if log::set_boxed_logger(Box::new(logger)).is_ok() {
                log::set_max_level(level);
            }

            None
        }
    }
}

fn level_from_env() -> Result<LevelFilter, log::ParseLevelError> {
    std::env::var("ANTITONE_LOG")
        .unwrap_or_else(|_| "info".into())
        .parse()
}

fn enabled(level: LevelFilter, metadata: &Metadata<'_>) -> bool {
    let level = if metadata.target() == "antitone" || metadata.target().starts_with("antitone::") {
        level
    } else {
        level.min(LevelFilter::Warn)
    };

    metadata.level() <= level
}

struct ConsoleLogger {
    level: LevelFilter,
}

impl Log for ConsoleLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        enabled(self.level, metadata)
    }

    fn log(&self, record: &Record<'_>) {
        if self.enabled(record.metadata()) {
            let entry = Entry::from_record(record);
            let _ = io::stderr().write_all(&entry.text);
        }
    }

    fn flush(&self) {
        let _ = io::stderr().flush();
    }
}

/// Keep alive until application workers and native owners have stopped.
pub struct Session {
    directory: PathBuf,
    logger: Arc<SessionLogger>,
    worker: Option<JoinHandle<()>>,
}

impl Session {
    /// `ANTITONE_LOG_DIR` overrides the parent directory; `ANTITONE_LOG` sets the
    /// app level (error/warn/info/debug/trace/off). Dependency logs stop at warn.
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        let level = level_from_env()?;

        let parent = match std::env::var_os("ANTITONE_LOG_DIR") {
            Some(path) => PathBuf::from(path),
            None => {
                let home =
                    std::env::var_os("HOME").ok_or("HOME is unavailable for session logs")?;
                PathBuf::from(home).join("Library/Logs/Antitone")
            }
        };

        let session = Self::open(&parent, level)?;
        log::set_boxed_logger(Box::new(session.logger.clone()))?;
        log::set_max_level(level);

        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            log::error!(
                target: "antitone::panic",
                details = info.to_string().as_str();
                "panic"
            );
            // Drain before the default hook, including when panic=abort is selected.
            log::logger().flush();

            previous(info);
        }));

        log::info!(
            version = env!("CARGO_PKG_VERSION"),
            os = std::env::consts::OS,
            arch = std::env::consts::ARCH,
            level = level.as_str();
            "session_started"
        );

        Ok(session)
    }

    fn open(parent: &Path, level: LevelFilter) -> io::Result<Self> {
        fs::create_dir_all(parent)?;
        let _retention = retention_lock(parent)?;

        // Atomic directory creation avoids collisions and prevents replacing an existing log.
        let prefix = format!("session-{}-{}-", now_ms(), std::process::id());
        let mut builder = tempfile::Builder::new();
        builder.prefix(&prefix);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(fs::Permissions::from_mode(0o700));
        }

        let directory = builder.tempdir_in(parent)?.keep();
        let sink = Sink::open(&directory, FILE_BYTES, SESSION_FILES)?;

        if let Err(error) = prune(parent, &directory) {
            let _ = writeln!(
                io::stderr(),
                "Antitone could not prune old session logs: {error}"
            );
        }

        let (send, receive) = mpsc::sync_channel(QUEUE_RECORDS);
        let dropped = Arc::new(AtomicU64::new(0));
        let logger = Arc::new(SessionLogger {
            level,
            send,
            dropped: dropped.clone(),
        });

        let console = Console::new(io::stderr(), QUEUE_RECORDS)?;
        let worker = thread::Builder::new()
            .name("session-log".into())
            .spawn(move || run_worker(sink, receive, dropped, console))?;

        Ok(Self {
            directory,
            logger,
            worker: Some(worker),
        })
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Stop is queued behind accepted events, including on error exits.
        let _ = self.logger.send.send(Command::Stop);

        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct SessionLogger {
    level: LevelFilter,
    send: mpsc::SyncSender<Command>,
    dropped: Arc<AtomicU64>,
}

enum Command {
    Record(Entry),
    Flush(mpsc::Sender<()>),
    Stop,
}

impl Log for SessionLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        enabled(self.level, metadata)
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let entry = Entry::from_record(record);

        match self.send.try_send(Command::Record(entry)) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
            Err(mpsc::TrySendError::Disconnected(Command::Record(entry))) => {
                let _ = io::stderr().write_all(&entry.text);
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {}
        }
    }

    fn flush(&self) {
        let (send, receive) = mpsc::channel();

        if self.send.send(Command::Flush(send)).is_ok() {
            let _ = receive.recv();
        }
    }
}

struct Entry {
    json: Vec<u8>,
    text: Vec<u8>,
}

impl Entry {
    fn from_record(record: &Record<'_>) -> Self {
        let mut fields = Fields(Map::new());
        let _ = record.key_values().visit(&mut fields);
        let message = record.args().to_string();

        Self::new(record.level(), record.target(), &message, fields.0)
    }

    fn new(level: Level, target: &str, message: &str, fields: Map<String, Value>) -> Self {
        let timestamp_ms = now_ms();
        let thread = format!("{:?}", thread::current().id());
        let mut json = serde_json::to_vec(&json!({
            "timestamp_ms": timestamp_ms,
            "level": level.as_str(),
            "target": target,
            "thread": thread,
            "event": message,
            "fields": fields,
        }))
        .expect("JSON values serialize to a byte vector");

        if json.len() > RECORD_BYTES {
            return Self::new(
                Level::Warn,
                "antitone::logging",
                "oversized_record_discarded",
                Map::new(),
            );
        }

        let seconds_since_epoch = timestamp_ms / 1000;
        let hours = seconds_since_epoch / 3600 % 24;
        let minutes = seconds_since_epoch / 60 % 60;
        let seconds = seconds_since_epoch % 60;
        let millis = timestamp_ms % 1000;
        let mut text = format!(
            "{hours:02}:{minutes:02}:{seconds:02}.{millis:03}Z {level:<5} {target} [{thread}]: {}",
            message.escape_debug(),
        );

        for (key, value) in fields {
            text.push(' ');
            text.push_str(&key);
            text.push('=');
            text.push_str(&value.to_string());
        }

        json.push(b'\n');
        text.push('\n');

        Self {
            json,
            text: text.into_bytes(),
        }
    }
}

struct Fields(Map<String, Value>);

impl<'kvs> kv::VisitSource<'kvs> for Fields {
    fn visit_pair(&mut self, key: kv::Key<'kvs>, value: kv::Value<'kvs>) -> Result<(), kv::Error> {
        let mut field = Field(Value::Null);
        value.visit(&mut field)?;
        self.0.insert(key.to_string(), field.0);

        Ok(())
    }
}

struct Field(Value);

impl<'v> kv::VisitValue<'v> for Field {
    fn visit_any(&mut self, value: kv::Value<'_>) -> Result<(), kv::Error> {
        self.0 = if let Some(value) = value.to_bool() {
            json!(value)
        } else if let Some(value) = value.to_i64() {
            json!(value)
        } else if let Some(value) = value.to_u64() {
            json!(value)
        } else if let Some(value) = value.to_f64() {
            json!(value)
        } else {
            json!(value.to_string())
        };

        Ok(())
    }

    fn visit_null(&mut self) -> Result<(), kv::Error> {
        self.0 = Value::Null;
        Ok(())
    }
}

fn run_worker(
    sink: Sink,
    receive: mpsc::Receiver<Command>,
    dropped: Arc<AtomicU64>,
    console: impl Write,
) {
    let mut outputs = Outputs {
        file: Some(sink),
        console,
    };

    while let Ok(command) = receive.recv() {
        let count = dropped.swap(0, Ordering::Relaxed);

        if count != 0 {
            let mut fields = Map::new();
            fields.insert("count".into(), json!(count));

            let entry = Entry::new(Level::Warn, "antitone::logging", "records_dropped", fields);
            outputs.write(&entry);
        }

        match command {
            Command::Record(entry) => outputs.write(&entry),
            Command::Flush(done) => {
                outputs.flush();
                let _ = done.send(());
            }
            Command::Stop => break,
        }
    }

    outputs.flush();
}

struct Console {
    send: mpsc::SyncSender<ConsoleCommand>,
}

enum ConsoleCommand {
    Record(Vec<u8>),
    Flush(mpsc::Sender<io::Result<()>>),
}

impl Console {
    fn new(mut output: impl Write + Send + 'static, capacity: usize) -> io::Result<Self> {
        let (send, receive) = mpsc::sync_channel(capacity);

        // Dropping the senders retires a healthy worker. Do not join this thread:
        // a descriptor stalled inside write/flush must not hold file capture or Quit.
        thread::Builder::new()
            .name("session-stderr".into())
            .spawn(move || {
                while let Ok(command) = receive.recv() {
                    match command {
                        ConsoleCommand::Record(bytes) => {
                            if output.write_all(&bytes).is_err() {
                                break;
                            }
                        }
                        ConsoleCommand::Flush(done) => {
                            let result = output.flush();
                            let failed = result.is_err();
                            let _ = done.send(result);

                            if failed {
                                break;
                            }
                        }
                    }
                }
            })?;

        Ok(Self { send })
    }

    fn send(&self, command: ConsoleCommand) -> io::Result<()> {
        self.send.try_send(command).map_err(|error| {
            let kind = match error {
                mpsc::TrySendError::Full(_) => io::ErrorKind::WouldBlock,
                mpsc::TrySendError::Disconnected(_) => io::ErrorKind::BrokenPipe,
            };

            io::Error::from(kind)
        })
    }
}

impl Write for Console {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.send(ConsoleCommand::Record(bytes.to_vec()))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let (done, receive) = mpsc::channel();
        self.send(ConsoleCommand::Flush(done))?;

        receive.recv_timeout(CONSOLE_FLUSH_WAIT).map_err(|error| {
            let kind = match error {
                mpsc::RecvTimeoutError::Timeout => io::ErrorKind::TimedOut,
                mpsc::RecvTimeoutError::Disconnected => io::ErrorKind::BrokenPipe,
            };

            io::Error::from(kind)
        })?
    }
}

struct Outputs<W> {
    file: Option<Sink>,
    console: W,
}

impl<W: Write> Outputs<W> {
    fn write(&mut self, entry: &Entry) {
        // An unavailable console must not stop file capture, or vice versa.
        let _ = self.console.write_all(&entry.text);

        let Some(file) = &mut self.file else {
            return;
        };

        if let Err(error) = file.write(&entry.json) {
            self.disable_file(error);
        }
    }

    fn flush(&mut self) {
        let _ = self.console.flush();

        let Some(file) = &mut self.file else {
            return;
        };

        if let Err(error) = file.file.flush() {
            self.disable_file(error);
        }
    }

    fn disable_file(&mut self, error: io::Error) {
        self.file = None;
        let _ = writeln!(
            self.console,
            "Antitone session file logging stopped: {error}"
        );
    }
}

struct Sink {
    directory: PathBuf,
    file: File,
    _lock: File,
    bytes: u64,
    limit: u64,
    files: usize,
}

impl Sink {
    fn open(directory: &Path, limit: u64, files: usize) -> io::Result<Self> {
        let lock = private_file(&directory.join(".active"))?;
        lock.lock()?;

        Ok(Self {
            directory: directory.into(),
            file: private_file(&directory.join("current.jsonl"))?,
            _lock: lock,
            bytes: 0,
            limit,
            files,
        })
    }

    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self.bytes != 0 && self.bytes + bytes.len() as u64 > self.limit {
            // Rotate only our session. Other running processes retain their own files.
            for index in (1..self.files).rev() {
                let to = self.directory.join(format!("{index}.jsonl"));
                let from = self.directory.join(if index == 1 {
                    "current.jsonl".into()
                } else {
                    format!("{}.jsonl", index - 1)
                });

                if to.exists() {
                    fs::remove_file(&to)?;
                }
                if from.exists() {
                    fs::rename(from, to)?;
                }
            }
            self.file = private_file(&self.directory.join("current.jsonl"))?;
            self.bytes = 0;
        }

        self.file.write_all(bytes)?;
        self.bytes += bytes.len() as u64;
        Ok(())
    }
}

fn private_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    options.open(path)
}

fn retention_lock(parent: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let lock = options.open(parent.join(".retention.lock"))?;
    lock.lock()?;

    Ok(lock)
}

// Caller holds the parent lock from before session creation through this scan.
// Otherwise competing startups can over-prune or delete a not-yet-locked session.
fn prune(parent: &Path, current: &Path) -> io::Result<()> {
    let mut sessions: Vec<_> = fs::read_dir(parent)?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_dir())
                && entry.file_name().to_string_lossy().starts_with("session-")
                && entry.path().join(".active").is_file()
                && entry.path() != current
        })
        .collect();

    sessions.sort_by_key(|entry| entry.file_name());
    let mut remove = sessions.len().saturating_sub(RETAINED_SESSIONS - 1);

    for entry in sessions {
        if remove == 0 {
            break;
        }

        // A held OS lock makes an active session ineligible for pruning.
        if let Ok(lock) = OpenOptions::new()
            .read(true)
            .write(true)
            .open(entry.path().join(".active"))
            && lock.try_lock().is_ok()
        {
            fs::remove_dir_all(entry.path())?;
            remove -= 1;
        }
    }

    Ok(())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests;
