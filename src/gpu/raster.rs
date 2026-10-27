//! Shared geometry/coverage contract for pixel stages; metadata is otherwise opaque.
use crate::{Error, frame_raster::FrameRaster, geometry::SourceRegion};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Raster {
    pub dimensions: [u32; 4],
    pub origin: [f32; 4],
    pub axis_x: [f32; 4],
    pub axis_y: [f32; 4],
}

impl Raster {
    pub fn frame(frame: FrameRaster) -> Self {
        let origin = frame.source_point([0.5, 0.5]).0;
        let x = frame.source_point([1.5, 0.5]).0;
        let y = frame.source_point([0.5, 1.5]).0;
        Self {
            dimensions: [frame.output[0], frame.output[1], 8, 0],
            origin: [origin[0] as f32, origin[1] as f32, 0., 0.],
            axis_x: [(x[0] - origin[0]) as f32, (x[1] - origin[1]) as f32, 0., 0.],
            axis_y: [(y[0] - origin[0]) as f32, (y[1] - origin[1]) as f32, 0., 0.],
        }
    }

    pub fn region(region: SourceRegion, size: [u32; 2]) -> Self {
        let dx = region.width as f32 / size[0] as f32;
        let dy = region.height as f32 / size[1] as f32;
        Self {
            dimensions: [size[0], size[1], 0, 0],
            origin: [
                region.x as f32 + 0.5 * dx,
                region.y as f32 + 0.5 * dy,
                0.,
                0.,
            ],
            axis_x: [dx, 0., 0., 0.],
            axis_y: [0., dy, 0., 0.],
        }
    }

    pub fn native(region: SourceRegion) -> Self {
        Self {
            dimensions: [region.width, region.height, 0, 0],
            origin: [region.x as f32 + 0.5, region.y as f32 + 0.5, 0., 0.],
            axis_x: [1., 0., 0., 0.],
            axis_y: [0., 1., 0., 0.],
        }
    }

    pub fn size(self) -> [u32; 2] {
        [self.dimensions[0], self.dimensions[1]]
    }
    pub fn pitch(self) -> [f32; 2] {
        [
            self.axis_x[0].hypot(self.axis_x[1]),
            self.axis_y[0].hypot(self.axis_y[1]),
        ]
    }
    pub fn footprint(self) -> f64 {
        self.pitch().into_iter().fold(0., f32::max) as f64
    }
    pub fn count(self) -> u32 {
        self.dimensions[0] * self.dimensions[1]
    }
}

pub(crate) fn buffer(
    device: &wgpu::Device,
    size: [u32; 2],
    label: &str,
) -> Result<wgpu::Buffer, Error> {
    let bytes = u64::from(size[0]) * u64::from(size[1]) * 16;
    let limits = device.limits();
    if size.contains(&0)
        || bytes > limits.max_buffer_size
        || bytes > limits.max_storage_buffer_binding_size
    {
        return Err(Error::Gpu("pixel raster exceeds GPU buffer limits".into()));
    }
    Ok(device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    }))
}

pub(crate) fn uniform<T: bytemuck::Pod>(
    device: &wgpu::Device,
    value: &T,
    label: &str,
) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytemuck::bytes_of(value),
        usage: wgpu::BufferUsages::UNIFORM,
    })
}

pub(crate) fn dispatch(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &wgpu::ComputePipeline,
    entries: &[wgpu::BindGroupEntry<'_>],
    count: u32,
) {
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries,
    });
    let groups = count.div_ceil(64);
    let x = groups.min(device.limits().max_compute_workgroups_per_dimension);
    let mut pass = encoder.begin_compute_pass(&Default::default());
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, &group, &[]);
    pass.dispatch_workgroups(x, groups.div_ceil(x), 1);
}
