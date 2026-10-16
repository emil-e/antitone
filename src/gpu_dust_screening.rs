//! Job-local integer screening on the hosted queue; no repaired pixels are computed here.
use crate::{
    Error,
    dust_repair::{ScreenInput, Screener},
    gpu::SubmissionGate,
};
use std::{
    num::NonZeroU64,
    time::{Duration, Instant},
};
use wgpu::util::DeviceExt;

// An optional acceleration working set, not permission to enlarge repair processing.
// Includes packed CPU/GPU features and weights, coordinates, results/staging,
// host result/bound/eligibility vectors and seed storage. Driver bytes are separate.
const MAX_BYTES: u64 = 64 * 1024 * 1024;
const SLICE_POINTS: usize = 32 * 1024 * 1024;
fn working_bytes(frame: usize, context: usize, candidates: usize) -> Option<u64> {
    (frame as u64)
        .checked_mul(40)?
        .checked_add((context as u64).checked_mul(8)?)?
        .checked_add((candidates as u64).checked_mul(192)?)?
        .checked_add(64 * 1024)
}

pub(crate) struct Screening<'a> {
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    submissions: &'a SubmissionGate,
    pipeline: Option<wgpu::ComputePipeline>,
}
impl<'a> Screening<'a> {
    pub fn new(
        device: &'a wgpu::Device,
        queue: &'a wgpu::Queue,
        submissions: &'a SubmissionGate,
    ) -> Self {
        Self {
            device,
            queue,
            submissions,
            pipeline: None,
        }
    }
    fn alignment(&self) -> usize {
        (self.device.limits().min_storage_buffer_offset_alignment as usize)
            .div_ceil(8)
            .max(1)
    }
    fn initialize(&mut self) -> Result<(), Error> {
        if self.pipeline.is_some() {
            return Ok(());
        }
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("dust screening integer lower bounds"),
                source: wgpu::ShaderSource::Wgsl(
                    include_str!("shaders/dust_screening.wgsl").into(),
                ),
            });
        let pipeline = self
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("dust screening"),
                layout: None,
                module: &shader,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
        if let Some(error) = pollster::block_on(scope.pop()) {
            return Err(Error::Gpu(error.to_string()));
        }
        self.pipeline = Some(pipeline);
        Ok(())
    }
    fn wait(&self, submission: wgpu::SubmissionIndex) -> Result<(), Error> {
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .map_err(|e| Error::Gpu(e.to_string()))?;
        Ok(())
    }
}
impl Screener for Screening<'_> {
    fn admit(&self, frame: usize, context: usize, candidates: usize) -> bool {
        let limits = self.device.limits();
        let binding = limits
            .max_storage_buffer_binding_size
            .min(limits.max_buffer_size);
        frame <= 4_000_000
            && context > 0
            && context <= 4_000_000
            && candidates <= 257 * 257
            && working_bytes(frame, context, candidates).is_some_and(|n| n <= MAX_BYTES)
            && frame as u64 * 20 <= binding
            && context as u64 * 4 <= binding
            && candidates as u64 * 32 <= binding
            && limits.max_compute_workgroup_size_x >= 128
            && limits.max_compute_invocations_per_workgroup >= 128
            && limits.max_compute_workgroup_storage_size >= 128 * 24 + 16
            && limits.max_compute_workgroups_per_dimension >= 1024
            && limits.max_storage_buffers_per_shader_stage >= 4
            && self.alignment() <= 4096
            && context
                .checked_mul(self.alignment())
                .and_then(|n| n.checked_mul(2))
                .is_some_and(|n| n <= SLICE_POINTS)
    }
    fn screen(
        &mut self,
        input: ScreenInput<'_>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<Vec<[u64; 5]>>, Error> {
        if cancelled() {
            return Ok(None);
        }
        let count = input.candidates.len();
        let frame = input.features.len() / 5;
        let context = input.weights.len();
        let width = input.width as usize;
        let local_width = input.local_width as usize;
        if width == 0
            || local_width == 0
            || !input.features.len().is_multiple_of(5)
            || !frame.is_multiple_of(width)
            || !context.is_multiple_of(local_width)
            || !self.admit(frame, context, count)
            || input.weights.iter().any(|w| {
                let floor = w & 511;
                let ceil = w >> 9;
                floor > 256 || ceil > 256 || ceil < floor || ceil - floor > 1
            })
        {
            return Err(Error::Gpu("invalid dust screening input".into()));
        }
        let height = frame / width;
        let local_height = context / local_width;
        let fits = |[x, y]: [u32; 2]| {
            (x as usize)
                .checked_add(local_width)
                .is_some_and(|v| v <= width)
                && (y as usize)
                    .checked_add(local_height)
                    .is_some_and(|v| v <= height)
        };
        if !fits(input.target) || input.candidates.iter().any(|p| !fits(*p)) {
            return Err(Error::Gpu(
                "dust screening coordinates exceed source".into(),
            ));
        }
        if count == 0 {
            return Ok(Some(Vec::new()));
        }
        self.initialize()?;
        if cancelled() {
            return Ok(None);
        }
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let allocation = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let storage = |contents: &[u8]| {
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("dust screening input"),
                    contents,
                    usage: wgpu::BufferUsages::STORAGE,
                })
        };
        let features = storage(bytemuck::cast_slice(input.features));
        let weights = storage(bytemuck::cast_slice(input.weights));
        let candidates = storage(bytemuck::cast_slice(input.candidates));
        let mut params = [
            input.width,
            input.target[0],
            input.target[1],
            input.local_width,
            context as u32,
            count as u32,
            input.factors[0].to_bits(),
            input.factors[1].to_bits(),
            input.ceiling.to_bits(),
            u32::from(input.paired_weights),
            input.upper_totals[0],
            input.upper_totals[1],
        ];
        let uniform = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("dust screening parameters"),
                contents: bytemuck::cast_slice(&params),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
        let bytes = count as u64 * 32;
        let output = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("dust screening bounds"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("dust screening readback"),
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let allocation_error = pollster::block_on(allocation.pop());
        let validation_error = pollster::block_on(validation.pop());
        if let Some(error) = allocation_error.or(validation_error) {
            return Err(Error::Gpu(error.to_string()));
        }
        let pipeline = self.pipeline.as_ref().unwrap();
        let alignment = self.alignment();
        let passes = if input.paired_weights { 2 } else { 1 };
        let maximum = (SLICE_POINTS / (passes * context)).min(4096) / alignment * alignment;
        let mut batch = maximum;
        let mut start = 0;
        while start < count {
            if cancelled() {
                return Ok(None);
            }
            let clock = Instant::now();
            let length = batch.min(count - start);
            params[5] = length as u32;
            self.queue
                .write_buffer(&uniform, 0, bytemuck::cast_slice(&params));
            let entries = [
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: features.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: weights.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &candidates,
                        offset: start as u64 * 8,
                        size: NonZeroU64::new(length as u64 * 8),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &output,
                        offset: start as u64 * 32,
                        size: NonZeroU64::new(length as u64 * 32),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: uniform.as_entire_binding(),
                },
            ];
            let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("dust screening slice"),
                layout: &pipeline.get_bind_group_layout(0),
                entries: &entries,
            });
            let mut encoder = self.device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("dust screening slice"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(length.min(1024) as u32, length.div_ceil(1024) as u32, 1);
            }
            self.wait(self.submissions.submit(self.queue, [encoder.finish()]))?;
            start += length;
            // A work bound limits the first slice; observed latency can only shrink
            // subsequent slices. Completion waits never hold the submission gate.
            if clock.elapsed() > Duration::from_millis(8) {
                batch = (batch / 2 / alignment * alignment).max(alignment);
            }
        }
        if cancelled() {
            return Ok(None);
        }
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
        let submission = self.submissions.submit(self.queue, [encoder.finish()]);
        let (send, receive) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = send.send(result);
            });
        self.wait(submission)?;
        receive
            .recv()
            .map_err(|e| Error::Gpu(e.to_string()))?
            .map_err(|e| Error::Gpu(e.to_string()))?;
        let mapped = readback
            .get_mapped_range(..)
            .map_err(|e| Error::Gpu(e.to_string()))?;
        let sums = bytemuck::cast_slice::<u8, [u32; 8]>(&mapped)
            .iter()
            .map(|v| {
                [
                    u64::from(v[0]) + (u64::from(v[1]) << 32),
                    u64::from(v[2]) + (u64::from(v[3]) << 32),
                    u64::from(v[4]),
                    u64::from(v[5]),
                    u64::from(v[6]),
                ]
            })
            .collect();
        drop(mapped);
        readback.unmap();
        Ok((!cancelled()).then_some(sums))
    }
}
