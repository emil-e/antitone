//! Supplied GPU samples for reconstruction arithmetic and output-sink tests.
use super::*;

pub(crate) fn supplied_mask(
    image: &Arc<ImageNode>,
    output: Rect,
    samples: Rect,
) -> Option<Vec<u32>> {
    let operation = (image.operation.as_ref() as &dyn std::any::Any)
        .downcast_ref::<Reconstruction>()
        .unwrap();
    let mut fragment = Fragment::default();
    let Binding::Read(resource) = operation.support_binding(&mut fragment, output, samples)? else {
        unreachable!()
    };
    let Storage::Data(bytes) = &fragment.resources[resource.0] else {
        unreachable!()
    };
    Some(bytemuck::cast_slice(bytes).to_vec())
}

pub(crate) fn encode_supplied(
    context: &Context,
    encoder: &mut wgpu::CommandEncoder,
    (texture, working, from_texture, source_size): (&wgpu::Texture, &wgpu::Buffer, bool, [u32; 2]),
    pixels: &wgpu::Buffer,
    (region, mapping): (Rect, Mapping),
    support: Option<&[u32]>,
) -> Result<(), Error> {
    let input = Domain {
        bounds: Some(crate::gpu::evaluation::CellRegion::new([0, 0], source_size)),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Graded,
    };
    let domain = Domain {
        bounds: Some(region),
        grid: Grid {
            origin: mapping.start,
            axes: mapping.axes,
            pitch: mapping.axes.map(|axis| axis[0].hypot(axis[1])),
            coverage: 1,
        },
        meaning: Meaning::Graded,
    };
    let operation = Reconstruction {
        input,
        mapping,
        packed: mapping.packed(),
        coverage: 1,
        support: Some(Support {
            output: region,
            coverage: crate::coverage::Coverage::rectangle(
                input.finite().unwrap().source().unwrap(),
            ),
        }),
    };
    let mut fragment = Fragment::default();
    let parameters = fragment.uniform(&[source_size[0], u32::from(from_texture), 0, 0]);
    let sample = Sampled {
        code: r#"
@group(0) @binding(32) var<storage,read> supplied_working:array<vec4f>;
@group(0) @binding(33) var supplied_texture:texture_2d<f32>;
@group(0) @binding(34) var<uniform> supplied_parameters:vec4u;
fn supplied_sample(p:vec2i)->vec4f {
    let original=supplied_working[u32(p.y)*supplied_parameters.x+u32(p.x)];
    var value=original;
    if supplied_parameters.y!=0u {value=textureLoad(supplied_texture,p,0);}
    // The displayed fixture's working precursor still determines its validity.
    if !all(abs(original.rgb)<=vec3f(3.402823e38)) || !all(abs(value.rgb)<=vec3f(3.402823e38)) {
        return vec4f(0.0,0.0,0.0,-1.0);
    }
    if original.w<0.0 ||value.w<0.0 {
        return vec4f(0.0,0.0,0.0,min(original.w,value.w));
    }
    return value;
}
"#
        .into(),
        symbol: "supplied_sample".into(),
        bindings: vec![
            (32, Binding::Shared(Shared::Buffer(working.clone()))),
            (
                33,
                Binding::Shared(Shared::Texture(texture.create_view(&Default::default()))),
            ),
            (34, parameters),
        ],
    };
    let result = PlannedImage {
        domain,
        region,
        resource: fragment.resource(Storage::Input {
            slot: 0,
            bytes: pixels.size(),
            writable: true,
        }),
    };
    let geometry = operation.geometry(&mut fragment, region, region, input.finite().unwrap());
    let code = Reconstruction::code(&sample, support.is_some());
    let mut bindings = sample.bindings;
    bindings.extend([(21, Binding::Write(result.resource)), (22, geometry)]);
    if let Some(bits) = support {
        bindings.push((
            25,
            Binding::Read(fragment.resource(Storage::Data(bytemuck::cast_slice(bits).into()))),
        ));
    }
    fragment.compute(code, "main", bindings, result.cells());
    context.encode_work(fragment, result, encoder, &[pixels])
}
