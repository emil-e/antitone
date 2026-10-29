//! Declared acquisition IO work; callers own bounded scheduling and private accumulators.
use super::*;

fn external(fragment: &mut Fragment, size: [u32; 2], writable: bool) -> PlannedImage {
    let bounds = [0, 0, size[0], size[1]].into();
    PlannedImage {
        domain: Domain {
            bounds,
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::Neutral,
        },
        region: bounds,
        resource: fragment.resource(Storage::Input {
            slot: 0,
            bytes: u64::from(size[0]) * u64::from(size[1]) * 16,
            writable,
        }),
    }
}

pub fn copy(
    context: &Context,
    encoder: &mut wgpu::CommandEncoder,
    pixels: &wgpu::Buffer,
    texture: &wgpu::Texture,
) -> Result<(), Error> {
    copy_sized(
        context,
        encoder,
        pixels,
        [texture.width(), texture.height()],
        texture,
    )
}
pub fn copy_sized(
    context: &Context,
    encoder: &mut wgpu::CommandEncoder,
    pixels: &wgpu::Buffer,
    source_size: [u32; 2],
    texture: &wgpu::Texture,
) -> Result<(), Error> {
    if source_size.contains(&0)
        || source_size[0] > texture.width()
        || source_size[1] > texture.height()
    {
        return Err(Error::InvalidParameter(
            "copy source differs from its texture",
        ));
    }
    if texture.format() != wgpu::TextureFormat::Rgba32Float {
        return Err(Error::InvalidParameter(
            "acquisition copy requires RGBA32 float storage",
        ));
    }
    if !texture
        .usage()
        .contains(wgpu::TextureUsages::STORAGE_BINDING)
    {
        return Err(Error::InvalidParameter(
            "acquisition copy requires a writable float texture",
        ));
    }
    let size = [texture.width(), texture.height()];
    let mut fragment = Fragment::default();
    let image = external(&mut fragment, source_size, false);
    let dimensions = fragment.uniform(&[source_size[0], source_size[1], 0u32, 0]);
    fragment.work.push(Work::Compute {
        code: include_str!("copy_texture.wgsl").into(),
        entry: "main",
        bindings: vec![
            (0, Binding::Read(image.resource)),
            (2, dimensions),
            (
                1,
                Binding::Shared(Shared::StorageTexture(
                    texture.create_view(&Default::default()),
                )),
            ),
        ],
        groups: [size[0].div_ceil(8), size[1].div_ceil(8), 1],
        initialize: None,
    });
    context.encode_work(fragment, image, encoder, &[pixels])
}

pub fn cover(
    context: &Context,
    encoder: &mut wgpu::CommandEncoder,
    pixels: &wgpu::Buffer,
) -> Result<(), Error> {
    let count = u32::try_from(pixels.size() / 16)
        .map_err(|_| Error::InvalidParameter("proxy coverage exceeds GPU indexing"))?;
    let mut fragment = Fragment::default();
    let image = external(&mut fragment, [count, 1], true);
    fragment.compute(
        include_str!("../../shaders/view_proxy_coverage.wgsl"),
        "main",
        vec![(0, Binding::Write(image.resource))],
        count,
    );
    context.encode_work(fragment, image, encoder, &[pixels])
}

pub struct Footprint {
    pub source: [u32; 2],
    pub output: [u32; 2],
    pub affine: [[f32; 2]; 3],
    pub origin: [i32; 2],
    pub window: crate::resample::windows::Window,
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Parameters {
    source: [u32; 2],
    output: [u32; 2],
    origin: [i32; 2],
    tile: [u32; 2],
    affine: [[f32; 2]; 3],
    padding: [f32; 2],
    lo: [u32; 2],
    extent: [u32; 2],
}
pub fn filter(
    context: &Context,
    encoder: &mut wgpu::CommandEncoder,
    texture: &wgpu::Texture,
    output: &wgpu::Buffer,
    footprint: Footprint,
) -> Result<(), Error> {
    let Footprint {
        source,
        output: size,
        affine,
        origin,
        window: (lo, extent),
    } = footprint;
    if source.contains(&0)
        || size.contains(&0)
        || extent.contains(&0)
        || (0..2).any(|i| lo[i].checked_add(extent[i]).is_none_or(|end| end > size[i]))
    {
        return Err(Error::InvalidParameter(
            "invalid acquisition destination window",
        ));
    }
    let determinant = affine[1][0] * affine[2][1] - affine[1][1] * affine[2][0];
    if affine.iter().flatten().any(|v| !v.is_finite())
        || !determinant.is_finite()
        || determinant == 0.
    {
        return Err(Error::InvalidParameter(
            "acquisition footprint exceeds GPU precision",
        ));
    }
    let parameters = Parameters {
        source,
        output: size,
        origin,
        tile: [texture.width(), texture.height()],
        affine,
        padding: [0.; 2],
        lo,
        extent,
    };
    let mut fragment = Fragment::default();
    let image = external(&mut fragment, size, true);
    let uniform = fragment.uniform(&parameters);
    static CODE: std::sync::OnceLock<Arc<str>> = std::sync::OnceLock::new();
    let code = CODE.get_or_init(|| {
        format!(
            "{}\n{}",
            include_str!("../../shaders/footprint.wgsl"),
            include_str!("../../shaders/view_filter.wgsl")
        )
        .into()
    });
    fragment.work.push(Work::Compute {
        code: code.clone(),
        entry: "main",
        bindings: vec![
            (
                0,
                Binding::Shared(Shared::Texture(texture.create_view(&Default::default()))),
            ),
            (1, Binding::Write(image.resource)),
            (2, uniform),
        ],
        groups: [extent[0].div_ceil(8), extent[1].div_ceil(8), 1],
        initialize: None,
    });
    context.encode_work(fragment, image, encoder, &[output])
}

#[allow(clippy::too_many_arguments)]
pub fn repair_copy(
    context: &Context,
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    output: &wgpu::Buffer,
    source: (&wgpu::Buffer, ([u32; 2], [u32; 2])),
    frame: crate::frame_raster::FrameRaster,
    window: ([u32; 2], [u32; 2]),
    repairs: &crate::gpu::repairs::Snapshot,
) -> Result<(), Error> {
    let bounds = frame.processing_bounds()?;
    let affine = frame.affine_in(bounds)?;
    for patch in &repairs.0 {
        let r = patch.region;
        if !(r.x < bounds.x + bounds.width
            && bounds.x < r.x + r.width
            && r.y < bounds.y + bounds.height
            && bounds.y < r.y + r.height)
        {
            continue;
        }
        let mut bytes = bytemuck::cast_slice(&[
            bounds.x,
            bounds.y,
            frame.output[0],
            frame.output[1],
            r.x,
            r.y,
            r.width,
            r.height,
            window.0[0],
            window.0[1],
            window.1[0],
            window.1[1],
            (source.1).0[0],
            (source.1).0[1],
            (source.1).1[0],
            (source.1).1[1],
        ])
        .to_vec();
        for pair in affine {
            bytes.extend_from_slice(bytemuck::cast_slice(&[pair[0], pair[1], 0., 0.]));
        }
        let area = frame.composition.area();
        bytes.extend_from_slice(bytemuck::cast_slice(&[
            area.x as i32 - bounds.x as i32,
            area.y as i32 - bounds.y as i32,
            area.width as i32,
            area.height as i32,
        ]));
        let Some(buffer) = patch.buffer_for(device, patch.region)? else {
            continue;
        };
        let mut fragment = Fragment::default();
        let image = external(&mut fragment, frame.output, true);
        let input = fragment.resource(Storage::Input {
            slot: 1,
            bytes: source.0.size(),
            writable: false,
        });
        let view = fragment.resource(Storage::Uniform(bytes.into()));
        fragment.work.push(Work::Compute {
            code: crate::gpu::ProcessingKernel::module_shader().into(),
            entry: "repair_copy",
            bindings: vec![
                (1, Binding::Write(image.resource)),
                (12, Binding::Shared(Shared::Buffer(buffer))),
                (14, Binding::Uniform(view)),
                (15, Binding::Read(input)),
            ],
            groups: [window.1[0].div_ceil(8), window.1[1].div_ceil(8), 1],
            initialize: None,
        });
        context.encode_work(fragment, image, encoder, &[output, source.0])?;
    }

    Ok(())
}
