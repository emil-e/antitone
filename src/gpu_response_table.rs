use super::ProcessingParameters;
use glam::Vec4Swizzles;
use std::sync::Mutex;
use wgpu::util::DeviceExt;

pub(super) const CODES: u64 = 65536;
pub(super) const BYTE_LEN: u64 = CODES * 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ResponseKey([u8; 176]);

impl ResponseKey {
    pub(super) fn new(p: ProcessingParameters) -> Self {
        // Match native_response/invert_film, including gain inside the final log/exp.
        let words = [
            p.dmin_floor,
            p.dmin_low,
            p.dmin_tail,
            p.gamma_mode.xyz().extend(0.),
            p.gamma_low.xyz().extend(0.),
            p.gamma_tail.xyz().extend(0.),
            p.shape,
            p.shape_tail,
            p.anchor,
            p.anchor_low,
            p.anchor_tail,
        ];
        Self(bytemuck::cast_slice(&words).try_into().unwrap())
    }
}

struct Entry {
    key: ResponseKey,
    buffer: wgpu::Buffer,
}

/// One most-recent immutable response; encoded/in-flight commands retain evicted buffers.
pub(super) struct ResponseCache {
    pipeline: wgpu::ComputePipeline,
    queue: wgpu::Queue,
    submissions: super::SubmissionGate,
    latest: Mutex<Option<Entry>>,
    pub(super) empty: wgpu::Buffer,
}

impl ResponseCache {
    pub(super) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        module: &wgpu::ShaderModule,
        submissions: super::SubmissionGate,
    ) -> Self {
        Self {
            pipeline: device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("exact RGB16 response table generation"),
                layout: None,
                module,
                entry_point: Some("build_response_codes"),
                compilation_options: Default::default(),
                cache: None,
            }),
            queue: queue.clone(),
            submissions,
            latest: Mutex::new(None),
            empty: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("unused response table"),
                size: 40,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            }),
        }
    }

    pub(super) fn buffer(
        &self,
        device: &wgpu::Device,
        parameters: ProcessingParameters,
    ) -> wgpu::Buffer {
        let key = ResponseKey::new(parameters);
        let mut latest = self.latest.lock().unwrap();
        if let Some(entry) = latest.as_ref().filter(|entry| entry.key == key) {
            return entry.buffer.clone();
        }
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("immutable exact RGB16 response"),
            size: BYTE_LEN,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("response generation parameters"),
            contents: bytemuck::bytes_of(&parameters),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("response generation"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: buffer.as_entire_binding(),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("initialize exact response before publication"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("RGB16 response table generation"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(CODES as u32 / 64, 1, 1);
        }
        // A caller may discard its encoder or submit it after another worker. Submit
        // initialization first, publish only then, and never overwrite an old table.
        self.submissions.submit(&self.queue, [encoder.finish()]);
        *latest = Some(Entry {
            key,
            buffer: buffer.clone(),
        });
        buffer
    }
}

#[cfg(all(test, target_os = "macos"))]
#[path = "gpu_response_table_tests.rs"]
mod tests;
