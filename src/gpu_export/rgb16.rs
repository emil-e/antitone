//! Codec-independent rendered RGB16 output encoding; bounded and submitted by its caller.
use crate::{Error, rendering::OutputTarget};
use wgpu::util::DeviceExt;

pub(super) const THRESHOLD_BYTES: u64 = 65535 * 4;

pub(super) struct Rgb16 {
    pipeline: wgpu::ComputePipeline,
    thresholds: wgpu::Buffer,
}

impl Rgb16 {
    pub fn new(device: &wgpu::Device, target: OutputTarget) -> Result<Self, Error> {
        let source = match target {
            OutputTarget::Srgb | OutputTarget::DisplayP3 => include_str!("../shaders/rgb16.wgsl"),
        };
        Self::from_source(device, target, source)
    }

    fn from_source(
        device: &wgpu::Device,
        target: OutputTarget,
        source: &str,
    ) -> Result<Self, Error> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("RGB16 output encoding"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
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
        let thresholds = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("RGB16 quantization boundaries"),
            contents: bytemuck::cast_slice(&threshold_bits(target)?),
            usage: wgpu::BufferUsages::STORAGE,
        });
        Ok(Self {
            pipeline,
            thresholds,
        })
    }

    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::Buffer,
        output: &wgpu::Buffer,
        count: u32,
    ) -> Result<(), Error> {
        if count == 0
            || count > super::TILE[0] * super::TILE[1]
            || input.size() < u64::from(count) * 16
            || output.size() < u64::from(count) * 8
        {
            return Err(Error::InvalidParameter("invalid RGB16 output unit"));
        }
        let dimensions = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("RGB16 active destination count"),
            contents: bytemuck::cast_slice(&[count, 0, 0, 0]),
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
                    resource: dimensions.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.thresholds.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("RGB16 transfer and packing"),
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(count.div_ceil(64), 1, 1);
        Ok(())
    }
}

// Positive f32 bit patterns are ordered. Each boundary is the first input whose existing
// CPU transfer/rounding reaches the next RGB16 code, not an approximation of a film response.
fn threshold_bits(target: OutputTarget) -> Result<Vec<u32>, Error> {
    let quantize = |bits| -> Result<u32, Error> {
        Ok((target.encode(f64::from(f32::from_bits(bits)))? * 65535.).round() as u32)
    };
    let mut thresholds = Vec::with_capacity((THRESHOLD_BYTES / 4) as usize);
    for code in 1..=65535 {
        let mut bits = (target.decode((f64::from(code) - 0.5) / 65535.)? as f32).to_bits();
        // Inverse transfer supplies a nearby candidate; verify rather than assume its rounding.
        let mut layers = 0;
        while quantize(bits)? < code {
            bits += 1;
            layers += 1;
            if layers > 8 {
                return Err(Error::Gpu("RGB16 boundary search did not converge".into()));
            }
        }
        while quantize(bits - 1)? >= code {
            bits -= 1;
            layers += 1;
            if layers > 8 {
                return Err(Error::Gpu("RGB16 boundary search did not converge".into()));
            }
        }
        if thresholds.last().is_some_and(|previous| *previous >= bits) {
            return Err(Error::Gpu("RGB16 boundaries are not increasing".into()));
        }
        thresholds.push(bits);
    }
    Ok(thresholds)
}

pub(super) fn decode(records: &[[u32; 2]]) -> Result<Vec<[u16; 3]>, Error> {
    let status = records.iter().fold(0, |flags, p| flags | (p[1] >> 16));
    if status & 1 != 0 || status & !3 != 0 {
        return Err(Error::Gpu(
            "RGB16 input has invalid reconstruction values".into(),
        ));
    }
    if status & 2 != 0 {
        return Err(Error::InvalidParameter(
            "rendered/encoded output channel must be finite and within 0..1",
        ));
    }
    Ok(records
        .iter()
        .map(|p| [p[0] as u16, (p[0] >> 16) as u16, p[1] as u16])
        .collect())
}

#[cfg(test)]
mod tests;
