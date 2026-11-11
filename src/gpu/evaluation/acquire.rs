//! Declared acquisition IO work; callers own bounded scheduling and private accumulators.

use super::*;

fn external(fragment: &mut Fragment, size: [u32; 2], writable: bool) -> PlannedImage {
    let bounds = CellRegion::new([0, 0], size);
    PlannedImage {
        domain: Domain {
            bounds: Some(bounds),
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

/// Convert calculated sample coverage to presentation opacity at the texture boundary.
pub fn present_sized(
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
