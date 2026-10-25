//! A fixture file that native readers and tests can open, read and append freely.
//!
//! `tempfile::NamedTempFile` is unsuitable here on Windows: it creates the file with an
//! exclusive share mode, so LibRaw and the DNG SDK cannot open the path while the handle lives,
//! and its closed-handle form keeps a read-only handle that blocks appends. These fixtures live
//! in a temporary directory as ordinary files instead.

pub struct FixturePath {
    #[allow(dead_code)]
    directory: tempfile::TempDir,
    path: std::path::PathBuf,
}

impl FixturePath {
    /// Writes `bytes` to `name` inside a fresh temporary directory.
    pub fn write(name: &str, bytes: &[u8]) -> Self {
        let directory = tempfile::TempDir::new().unwrap();
        let path = directory.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        Self { directory, path }
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Appends bytes as tests do to change a fixture's revision.
    #[allow(dead_code)]
    pub fn append(&self, bytes: &[u8]) {
        use std::io::Write;
        std::fs::OpenOptions::new()
            .append(true)
            .open(self.path())
            .unwrap()
            .write_all(bytes)
            .unwrap();
    }
}
