use serde::{Deserialize, Serialize};

use crate::platform::GpuMemory;

pub(super) const MIB: u64 = 1024 * 1024;
pub(super) const MAX_MIB: u64 = usize::MAX as u64 / MIB;

/// Optional retained storage, not total process memory or an allocation permission.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CacheLimits {
    pub cpu_bytes: u64,
    pub gpu_bytes: u64,
}

impl CacheLimits {
    pub fn valid(self) -> bool {
        [self.cpu_bytes, self.gpu_bytes]
            .into_iter()
            .all(|bytes| bytes.is_multiple_of(MIB) && bytes / MIB <= MAX_MIB)
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Hardware {
    pub ram_bytes: Option<u64>,
    pub gpu: GpuMemory,
}

impl Hardware {
    /// Hardware fractions select starting retention defaults, not live allocation headroom.
    pub fn defaults(self) -> CacheLimits {
        let ram = self.ram_bytes.filter(|&bytes| bytes != 0);
        let cpu = ram.map_or(1024 * MIB, |bytes| bytes / 4);
        let gpu = match self.gpu {
            GpuMemory::Unified => ram.map_or(256 * MIB, |bytes| bytes / 16),
            GpuMemory::Discrete { dedicated_bytes } => dedicated_bytes
                .filter(|&bytes| bytes != 0)
                .map_or(256 * MIB, |bytes| bytes / 4),
            GpuMemory::Unknown => 256 * MIB,
        };
        let round = |bytes: u64| bytes.min(MAX_MIB * MIB) / MIB * MIB;
        CacheLimits {
            cpu_bytes: round(cpu),
            gpu_bytes: round(gpu),
        }
    }

    pub fn current(device: Option<&wgpu::Device>) -> Self {
        let gpu = crate::platform::gpu_memory(device);
        let bytes = crate::resources::physical_memory();
        Self {
            ram_bytes: (bytes != 0).then_some(bytes),
            gpu,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::GpuMemory;

    #[test]
    fn starting_defaults_distinguish_shared_ram_dedicated_vram_and_unknown_capacity() {
        let limits = Hardware {
            ram_bytes: Some(48 << 30),
            gpu: GpuMemory::Unified,
        }
        .defaults();
        assert_eq!(
            limits,
            CacheLimits {
                cpu_bytes: 12 << 30,
                gpu_bytes: 3 << 30
            }
        );
        let discrete = Hardware {
            ram_bytes: Some(48 << 30),
            gpu: GpuMemory::Discrete {
                dedicated_bytes: Some(8 << 30),
            },
        }
        .defaults();
        assert_eq!(
            discrete,
            CacheLimits {
                cpu_bytes: 12 << 30,
                gpu_bytes: 2 << 30
            }
        );
        for gpu in [
            GpuMemory::Unknown,
            GpuMemory::Discrete {
                dedicated_bytes: None,
            },
            GpuMemory::Discrete {
                dedicated_bytes: Some(0),
            },
        ] {
            assert_eq!(
                Hardware {
                    ram_bytes: Some(48 << 30),
                    gpu
                }
                .defaults()
                .gpu_bytes,
                256 * MIB
            );
        }
        for ram in [None, Some(0)] {
            assert_eq!(
                Hardware {
                    ram_bytes: ram,
                    gpu: GpuMemory::Unified
                }
                .defaults(),
                CacheLimits {
                    cpu_bytes: 1024 * MIB,
                    gpu_bytes: 256 * MIB
                }
            );
        }
    }

    #[test]
    fn exact_mib_units_allow_zero_without_inventing_hardware_minima() {
        assert!(
            CacheLimits {
                cpu_bytes: 0,
                gpu_bytes: MAX_MIB * MIB
            }
            .valid()
        );
        assert!(
            !CacheLimits {
                cpu_bytes: 1,
                gpu_bytes: 0
            }
            .valid()
        );
        for ram in [Some(1), Some(7 * MIB), Some(u64::MAX), None] {
            assert!(
                Hardware {
                    ram_bytes: ram,
                    gpu: GpuMemory::Unified
                }
                .defaults()
                .valid()
            );
        }
        assert_eq!(
            Hardware {
                ram_bytes: Some(1),
                gpu: GpuMemory::Unified
            }
            .defaults()
            .cpu_bytes,
            0
        );
    }
}
