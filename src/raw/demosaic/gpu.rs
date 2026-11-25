//! Bayer development on the caller's device, returning packed camera RGBG to LibRaw.

use std::sync::{Mutex, OnceLock, mpsc};
use std::time::Duration;
use wgpu::util::DeviceExt;

const TRANSFER_BYTES: u64 = 128 << 20;
const STAGES: &[&str] = &[
    "prepare",
    "reduce_bounds",
    "hot_ab",
    "hot_ba",
    "hot_ab",
    "hv_directions",
    "checker0",
    "checker1",
    "hv_refine",
    "make_green",
    "diagonal_directions",
    "diagonal_refine",
    "red_blue",
    "output",
];

/// Shared-device Bayer acceleration. Calls serialize their bounded working storage;
/// pipelines survive calls, but completed frames do not retain scratch allocations.
/// Unavailable acceleration leaves the supplied prepared image unchanged.
pub struct Demosaicer {
    device: wgpu::Device,
    queue: crate::gpu::submission::Queue,
    working_bytes: u64,
    pressure: fn() -> crate::sys::MemoryPressure,
    execution: Mutex<()>,
    programs: OnceLock<Result<Programs, String>>,
}

struct Programs {
    layout: wgpu::BindGroupLayout,
    pipelines: Vec<wgpu::ComputePipeline>,
}

struct Layout {
    parameters: [u32; 8],
    sizes: [u64; 10],
    image_bytes: u64,
    working_bytes: u64,
    dispatch: [u32; 2],
}

impl Layout {
    fn new(width: u32, height: u32, length: usize, pattern: [u8; 4]) -> Result<Self, String> {
        if !matches!(
            pattern,
            [0, 1, 1, 2] | [2, 1, 1, 0] | [1, 0, 2, 1] | [1, 2, 0, 1]
        ) {
            return Err("demosaic requires three-colour Bayer samples".into());
        }
        let width = u64::from(width);
        let height = u64::from(height);
        if width == 0 || height == 0 || width.checked_mul(height) != Some(length as u64) {
            return Err("invalid demosaic dimensions".into());
        }
        let pitch = width + 8;
        let rows = height + 8;
        let count = pitch
            .checked_mul(rows)
            .filter(|&n| n <= i32::MAX as u64)
            .ok_or("demosaic padded image exceeds signed indexing")?;
        let rb_pitch = pitch.div_ceil(2);
        let gx = pitch.div_ceil(16);
        let gy = rows.div_ceil(16);
        let image_bytes = length as u64 * 8;
        let sizes = [
            image_bytes,
            count * 4,
            count * 4,
            count * 4,
            rb_pitch * rows * 8,
            count * 4,
            count * 4,
            image_bytes,
            gx * gy * 32,
            32,
        ];
        let working_bytes =
            sizes.iter().sum::<u64>() + image_bytes + image_bytes.min(TRANSFER_BYTES) + 32;
        Ok(Self {
            parameters: [
                width as u32,
                height as u32,
                pitch as u32,
                rows as u32,
                pattern
                    .iter()
                    .enumerate()
                    .map(|(i, &c)| u32::from(c) << (2 * i))
                    .sum(),
                rb_pitch as u32,
                gx as u32,
                (gx * gy) as u32,
            ],
            sizes,
            image_bytes,
            working_bytes,
            dispatch: [gx as u32, gy as u32],
        })
    }

    fn admit(&self, limits: &wgpu::Limits, working_bytes: u64) -> Result<(), String> {
        if limits.max_storage_buffers_per_shader_stage < 10 {
            return Err("demosaic needs ten storage bindings".into());
        }
        if self.working_bytes > working_bytes {
            return Err("demosaic exceeds its working-memory allowance".into());
        }
        if self.sizes.iter().any(|&bytes| {
            bytes > limits.max_buffer_size || bytes > limits.max_storage_buffer_binding_size
        }) || self
            .dispatch
            .iter()
            .any(|&n| n > limits.max_compute_workgroups_per_dimension)
        {
            return Err("demosaic exceeds device buffer or dispatch limits".into());
        }
        Ok(())
    }
}

impl Demosaicer {
    /// Handles and submission authority must belong to the same device/queue.
    /// The allowance bounds one call's buffers, readback and input staging, not image retention.
    pub fn new(
        device: &wgpu::Device,
        queue: &crate::gpu::submission::Queue,
        working_bytes: u64,
    ) -> Self {
        Self {
            device: device.clone(),
            queue: queue.clone(),
            working_bytes,
            pressure: || crate::sys::MemoryPressure::Normal,
            execution: Mutex::new(()),
            programs: OnceLock::new(),
        }
    }

    pub(crate) fn with_system_pressure(mut self) -> Self {
        self.pressure = crate::sys::memory_pressure;
        self
    }

    pub fn demosaic(
        &self,
        image: &mut [[u16; 4]],
        width: u32,
        height: u32,
        pattern: [u8; 4],
    ) -> Result<(), String> {
        let layout = Layout::new(width, height, image.len(), pattern)?;
        layout.admit(&self.device.limits(), self.working_bytes)?;
        let _execution = self
            .execution
            .lock()
            .map_err(|_| "demosaic execution failed")?;
        if (self.pressure)() != crate::sys::MemoryPressure::Normal {
            return Err("demosaic deferred to CPU under memory pressure".into());
        }
        let programs = self
            .programs
            .get_or_init(|| self.create_programs())
            .as_ref()
            .map_err(Clone::clone)?;
        let scopes = [
            wgpu::ErrorFilter::Validation,
            wgpu::ErrorFilter::OutOfMemory,
            wgpu::ErrorFilter::Internal,
        ]
        .map(|filter| self.device.push_error_scope(filter));
        let result = self.execute(programs, &layout, image, TRANSFER_BYTES);
        let mut error = None;
        for scope in scopes.into_iter().rev() {
            if let Some(found) = pollster::block_on(scope.pop()) {
                error = Some(found.to_string());
            }
        }
        if let Some(error) = error {
            return Err(error);
        }
        let frame = result?;
        // Mapping and all GPU errors are resolved before the sole write to the caller's image.
        frame.copy_into(image)
    }

    fn create_programs(&self) -> Result<Programs, String> {
        let scopes = [
            wgpu::ErrorFilter::Validation,
            wgpu::ErrorFilter::OutOfMemory,
            wgpu::ErrorFilter::Internal,
        ]
        .map(|filter| self.device.push_error_scope(filter));
        let entries: Vec<_> = (0..11)
            .map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: if binding == 0 {
                        wgpu::BufferBindingType::Uniform
                    } else {
                        wgpu::BufferBindingType::Storage {
                            read_only: binding == 1,
                        }
                    },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            })
            .collect();
        let layout = self
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Bayer DHT"),
                entries: &entries,
            });
        let pipeline_layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Bayer DHT"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Bayer DHT"),
                source: wgpu::ShaderSource::Wgsl(include_str!("dht.wgsl").into()),
            });
        let pipelines = STAGES
            .iter()
            .map(|entry| {
                self.device
                    .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: Some(entry),
                        layout: Some(&pipeline_layout),
                        module: &shader,
                        entry_point: Some(entry),
                        compilation_options: Default::default(),
                        cache: None,
                    })
            })
            .collect();
        let mut error = None;
        for scope in scopes.into_iter().rev() {
            if let Some(found) = pollster::block_on(scope.pop()) {
                error = Some(found.to_string());
            }
        }
        match error {
            Some(error) => Err(error),
            None => Ok(Programs { layout, pipelines }),
        }
    }

    fn execute(
        &self,
        programs: &Programs,
        layout: &Layout,
        image: &[[u16; 4]],
        transfer_bytes: u64,
    ) -> Result<Frame, String> {
        let uniform = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("DHT parameters"),
                contents: bytemuck::cast_slice(&layout.parameters),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let buffers: Vec<_> = layout
            .sizes
            .iter()
            .map(|&size| {
                self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("DHT working storage"),
                    size,
                    usage: wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::COPY_SRC
                        | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect();
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform.as_entire_binding(),
        }];
        entries.extend(
            buffers
                .iter()
                .enumerate()
                .map(|(i, b)| wgpu::BindGroupEntry {
                    binding: i as u32 + 1,
                    resource: b.as_entire_binding(),
                }),
        );
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("DHT frame"),
            layout: &programs.layout,
            entries: &entries,
        });
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("DHT readback"),
            size: layout.image_bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let frame = Frame {
            readback,
            mapping: false,
        };
        let input = bytemuck::cast_slice::<_, u8>(image);
        self.transfer_batches(
            layout.image_bytes,
            transfer_bytes,
            |encoder, offset, size| {
                // Explicit staging keeps this copy off wgpu's pending-write list, where it could
                // otherwise become part of an unrelated window submission.
                let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("DHT upload staging"),
                    size,
                    usage: wgpu::BufferUsages::MAP_WRITE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: true,
                });
                staging
                    .slice(..)
                    .get_mapped_range_mut()
                    .map_err(|error| error.to_string())?
                    .copy_from_slice(&input[offset as usize..(offset + size) as usize]);
                staging.unmap();
                encoder.copy_buffer_to_buffer(&staging, 0, &buffers[0], offset, size);
                Ok(())
            },
        )?;

        // The full bind group would otherwise trigger lazy initialization of all scratch
        // buffers in the first compute submission.
        for buffer in &buffers[1..] {
            self.transfer_batches(buffer.size(), transfer_bytes, |encoder, offset, size| {
                encoder.clear_buffer(buffer, offset, Some(size));
                Ok(())
            })?;
        }

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Bayer DHT"),
            });
        for (stage, pipeline) in STAGES.iter().zip(&programs.pipelines) {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(stage),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &group, &[]);
            let [x, y] = if *stage == "reduce_bounds" {
                [1, 1]
            } else {
                layout.dispatch
            };
            pass.dispatch_workgroups(x, y, 1);
        }
        self.submit_batch(encoder)?;

        self.transfer_batches(
            layout.image_bytes,
            transfer_bytes,
            |encoder, offset, size| {
                encoder.copy_buffer_to_buffer(&buffers[7], offset, &frame.readback, offset, size);
                Ok(())
            },
        )?;
        let mut frame = frame;
        let (send, receive) = mpsc::sync_channel(1);
        frame
            .readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = send.send(result);
            });
        frame.mapping = true;
        self.await_callback(receive)?
            .map_err(|error| error.to_string())?;
        Ok(frame)
    }

    fn transfer_batches(
        &self,
        bytes: u64,
        batch_bytes: u64,
        mut record: impl FnMut(&mut wgpu::CommandEncoder, u64, u64) -> Result<(), String>,
    ) -> Result<(), String> {
        for offset in (0..bytes).step_by(batch_bytes as usize) {
            let size = (bytes - offset).min(batch_bytes);
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("DHT transfer"),
                });
            record(&mut encoder, offset, size)?;
            self.submit_batch(encoder)?;
        }
        Ok(())
    }

    fn submit_batch(&self, encoder: wgpu::CommandEncoder) -> Result<(), String> {
        let (send, receive) = mpsc::sync_channel(1);
        self.queue
            .submit_with_completion([encoder.finish()], move || {
                let _ = send.send(());
            });
        self.await_callback(receive)
    }

    fn await_callback<T>(&self, receive: mpsc::Receiver<T>) -> Result<T, String> {
        // A blocking wgpu poll can hold a device lock needed by surface presentation.
        // Drive callbacks here as well as in the host so DHT does not depend on redraws.
        loop {
            self.device
                .poll(wgpu::PollType::Poll)
                .map_err(|error| error.to_string())?;
            match receive.recv_timeout(Duration::from_millis(1)) {
                Ok(result) => return Ok(result),
                Err(mpsc::RecvTimeoutError::Timeout) => (),
                Err(error) => return Err(error.to_string()),
            }
        }
    }
}

struct Frame {
    readback: wgpu::Buffer,
    mapping: bool,
}

impl Frame {
    fn copy_into(&self, image: &mut [[u16; 4]]) -> Result<(), String> {
        let mapped = self
            .readback
            .slice(..)
            .get_mapped_range()
            .map_err(|e| e.to_string())?;
        bytemuck::cast_slice_mut(image).copy_from_slice(&mapped);
        Ok(())
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        if self.mapping {
            self.readback.unmap();
        }
    }
}

#[cfg(test)]
mod tests;
