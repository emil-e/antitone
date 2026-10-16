//! Optional exact inference scratch, scoped to one preparation. Never a persistent source cache.
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
};

const LIMIT: u64 = 512 << 20;

pub(super) struct Measurement {
    size: [u32; 2],
    offset: u64,
}
#[derive(Default)]
pub(super) struct Measurements {
    file: Option<File>,
    bytes: u64,
    failed: bool,
}
impl Measurements {
    fn bytes(size: [u32; 2]) -> Option<u64> {
        size.iter()
            .all(|&n| (1..=3072).contains(&n))
            .then(|| u64::from(size[0]) * u64::from(size[1]) * 6)
    }
    pub fn can_store(&self, size: [u32; 2]) -> bool {
        !self.failed && Self::bytes(size).is_some_and(|n| n <= LIMIT - self.bytes)
    }
    pub fn store(&mut self, size: [u32; 2], rgb: &[[u16; 3]]) -> Option<Measurement> {
        if !self.can_store(size) || rgb.len() as u64 * 6 != Self::bytes(size)? {
            return None;
        }
        // tempfile() is anonymous on the supported OS: no path remains after creation,
        // and the descriptor owns the scratch lifetime, including cancellation/unwind.
        let result = (|| {
            if self.file.is_none() {
                self.file = Some(tempfile::tempfile()?);
            }
            let file = self.file.as_mut().unwrap();
            file.seek(SeekFrom::Start(self.bytes))?;
            file.write_all(bytemuck::cast_slice(rgb))
        })();
        if result.is_err() {
            self.discard();
            return None;
        }
        let measurement = Measurement {
            size,
            offset: self.bytes,
        };
        self.bytes += Self::bytes(size)?;
        Some(measurement)
    }
    pub fn load(&mut self, measurement: Measurement) -> Option<([u32; 2], Vec<[u16; 3]>)> {
        let file = self.file.as_mut()?;
        let mut rgb = vec![[0; 3]; (Self::bytes(measurement.size)? / 6) as usize];
        let result = file
            .seek(SeekFrom::Start(measurement.offset))
            .and_then(|_| file.read_exact(bytemuck::cast_slice_mut(&mut rgb)));
        if result.is_err() {
            self.discard();
            return None;
        }
        Some((measurement.size, rgb))
    }
    #[cfg(test)]
    pub(super) fn unavailable() -> Self {
        Self {
            failed: true,
            ..Default::default()
        }
    }
    #[cfg(test)]
    pub(super) fn full() -> Self {
        Self {
            bytes: LIMIT,
            ..Default::default()
        }
    }
    fn discard(&mut self) {
        self.file = None;
        self.bytes = 0;
        self.failed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scratch_round_trips_exact_codes_and_distinct_sizes_without_native_source_retention() {
        let mut scratch = Measurements::default();
        let a = [[0, 1, 65535], [32767, 32768, 65534]];
        let b = [[17, 3000, 40000]; 6];
        let first = scratch.store([2, 1], &a).unwrap();
        let second = scratch.store([2, 3], &b).unwrap();
        assert_eq!(scratch.bytes, 48);
        assert_eq!(scratch.load(second).unwrap(), ([2, 3], b.to_vec()));
        assert_eq!(scratch.load(first).unwrap(), ([2, 1], a.to_vec()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(
                scratch.file.as_ref().unwrap().metadata().unwrap().nlink(),
                0
            );
        }
    }
    #[test]
    fn scratch_admission_is_bounded_and_malformed_inputs_do_not_allocate_a_file() {
        let mut scratch = Measurements::default();
        for size in [[0, 1], [1, 0], [3073, 1], [u32::MAX, u32::MAX]] {
            assert!(!scratch.can_store(size));
            assert!(scratch.store(size, &[]).is_none());
        }
        assert!(scratch.store([2, 2], &[[0; 3]; 3]).is_none());
        assert!(scratch.file.is_none());
        scratch.bytes = LIMIT - 6;
        assert!(scratch.can_store([1, 1]));
        assert!(!scratch.can_store([2, 1]));
        scratch.bytes = LIMIT;
        assert!(!scratch.can_store([1, 1]));
    }
    #[test]
    fn truncated_scratch_abstains_and_retires_all_previously_saved_handles() {
        let mut scratch = Measurements::default();
        let first = scratch.store([1, 1], &[[1, 2, 3]]).unwrap();
        let second = scratch.store([1, 1], &[[4, 5, 6]]).unwrap();
        scratch.file.as_ref().unwrap().set_len(1).unwrap();
        assert!(scratch.load(second).is_none());
        assert!(scratch.load(first).is_none());
        assert!(!scratch.can_store([1, 1]));
        assert!(scratch.file.is_none());
    }
    #[test]
    fn write_failure_disables_optional_scratch_instead_of_exposing_partial_measurements() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("read-only");
        std::fs::write(&path, []).unwrap();
        let mut scratch = Measurements {
            file: Some(File::open(path).unwrap()),
            ..Default::default()
        };
        assert!(scratch.store([1, 1], &[[1, 2, 3]]).is_none());
        assert!(scratch.failed);
        assert!(scratch.file.is_none());
        assert_eq!(scratch.bytes, 0);
    }
}
