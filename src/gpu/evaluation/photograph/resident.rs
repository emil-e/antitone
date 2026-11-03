//! Borrowed camera-code storage is addressable without expanding or correcting a complete grid.
use super::*;

pub(super) struct ResidentCamera {
    pub domain: Domain,
    pub pixels: wgpu::Buffer,
    pub continuous: bool,
}

impl Operation for ResidentCamera {
    fn input(&self, _: CellRegion) -> Result<Option<CellRegion>, Error> {
        Ok(None)
    }
    fn sample_producer(&self) -> Option<&dyn SampleProducer> {
        Some(self)
    }
    fn lower(
        &self,
        fragment: &mut Fragment,
        _: Option<PlannedImage>,
        output: PlannedImage,
    ) -> Result<(), Error> {
        self.emit(fragment, None, output.domain, output.region)?
            .materialize(fragment, output)
    }
}
impl SampleProducer for ResidentCamera {
    fn access(&self) -> Access {
        Access::Callable
    }
    fn emit(
        &self,
        fragment: &mut Fragment,
        _: Option<SampleInput>,
        _: Domain,
        _: CellRegion,
    ) -> Result<Sampled, Error> {
        let b = self.domain.finite()?;
        let bounds = fragment.uniform(&b.packed());
        let (element, value) = if self.continuous {
            ("vec4f", "return resident_codes[index];")
        } else {
            (
                "vec2u",
                "let packed=resident_codes[index]; return vec4f(f32(packed.x & 65535u),f32(packed.x >> 16u),f32(packed.y & 65535u),0.0);",
            )
        };
        Ok(Sampled {
            code: format!(
                r#"
@group(0) @binding(32) var<storage,read> resident_codes: array<{element}>;
@group(0) @binding(33) var<uniform> resident_bounds: vec4u;
fn sample_resident_camera(p:vec2i) -> vec4f {{
    let local=vec2u(p)-resident_bounds.xy;
    let index=local.y*resident_bounds.z+local.x;
    {value}
}}
"#
            )
            .into(),
            symbol: "sample_resident_camera".into(),
            bindings: vec![
                (32, Binding::Shared(Shared::Buffer(self.pixels.clone()))),
                (33, bounds),
            ],
        })
    }
}

impl SampleProducer for Camera {
    fn access(&self) -> Access {
        Access::Callable
    }
    fn emit(
        &self,
        fragment: &mut Fragment,
        input: Option<SampleInput>,
        domain: Domain,
        region: CellRegion,
    ) -> Result<Sampled, Error> {
        let Some(SampleInput::Callable(input)) = input else {
            return Err(Error::InvalidParameter(
                "camera correction requires captured samples",
            ));
        };
        let (function, coordinates) = if let Some(function) = self.pointwise() {
            (function, domain)
        } else {
            let capture = &self.captured;
            let geometry = capture.mapping.geometry(region)?;
            let field = match &capture.field {
                Some(field) => FieldGeometry::new(
                    field,
                    geometry.ok_or(Error::InvalidParameter(
                        "corrected source requires geometry",
                    ))?,
                ),
                None => FieldGeometry::uncorrected(geometry),
            };
            let function = Function::new(
                format!(
                    "{}\n{}",
                    crate::gpu::photograph::pixel_shader(),
                    r#"
fn corrected_resident_camera_light(pixel:vec4f, position:vec2f) -> vec4f {
    var source=vec2u(0u);
    if field.grid.z != 0u {
        let xy=vec2u(floor(position));
        source=source_pixel(xy.y*field.dimensions.x+xy.x);
    }
    return vec4f(camera_light_at(pixel.rgb,source),0.0);
}
"#
                ),
                "corrected_resident_camera_light",
            );
            let function = capture.bind_field(function, &field);
            // Nearest-centre development uses integral proxy indices, not rounded world positions.
            (
                function,
                Domain {
                    grid: Grid::axis_aligned([0.; 2], [1.; 2]),
                    ..domain
                },
            )
        };
        input.map(fragment, coordinates, region, vec![function])
    }
}
