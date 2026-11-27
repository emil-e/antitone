//! Codec-independent rendered RGB16 output encoding; bounded and submitted by its caller.
use crate::{Error, rendering::OutputTarget};
use wgpu::util::DeviceExt;

pub(crate) struct Rgb16 {
    pipeline: wgpu::ComputePipeline,
    target: OutputTarget,
}

impl Rgb16 {
    pub fn new(device: &wgpu::Device, target: OutputTarget) -> Result<Self, Error> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("RGB16 output encoding"),
            source: wgpu::ShaderSource::Wgsl(include_str!("rgb16.wgsl").into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("RGB16 output encoding"),
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        if let Some(error) = pollster::block_on(scope.pop()) {
            return Err(Error::Gpu(error.to_string()));
        }
        Ok(Self { pipeline, target })
    }

    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::Buffer,
        output: &wgpu::Buffer,
        count: u32,
    ) -> Result<(), Error> {
        let groups = count.div_ceil(64);
        if count == 0
            || groups > device.limits().max_compute_workgroups_per_dimension
            || input.size() < u64::from(count) * 16
            || output.size() < u64::from(count) * 8
        {
            return Err(Error::InvalidParameter("invalid RGB16 output unit"));
        }
        let settings = [
            count,
            (self.target.linear_peak() as f32).to_bits(),
            u32::from(self.target.is_hdr()),
            0,
        ];
        let parameters = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("RGB16 block and transfer settings"),
            contents: bytemuck::cast_slice(&settings),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("RGB16 output bindings"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: input.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: output.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: parameters.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("RGB16 transfer and packing"),
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(groups, 1, 1);
        Ok(())
    }
}

pub(crate) fn validate(records: &[[u32; 2]]) -> Result<(), Error> {
    let status = records.iter().fold(0, |flags, p| flags | (p[1] >> 16));
    if status & 1 != 0 || status & !3 != 0 {
        return Err(Error::Gpu(
            "RGB16 input has invalid reconstruction values".into(),
        ));
    }
    if status & 2 != 0 {
        return Err(Error::InvalidParameter(
            "RGB16 input channel is outside the display target range",
        ));
    }
    Ok(())
}

pub(crate) fn decode(records: &[[u32; 2]]) -> Result<Vec<[u16; 3]>, Error> {
    validate(records)?;
    Ok(records
        .iter()
        .map(|p| [p[0] as u16, (p[0] >> 16) as u16, p[1] as u16])
        .collect())
}

#[cfg(test)]
mod tests;
