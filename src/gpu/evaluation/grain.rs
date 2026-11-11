//! Grain composes an independently reusable stochastic image and photographic application.
use super::*;

pub(crate) fn grain(
    input: Value,
    model: crate::grain::GrainParameters,
    seed: u64,
) -> Result<Value, Error> {
    if model.amplitude <= 0. {
        return Ok(input);
    }
    let field = value::Value::captured(
        Domain {
            meaning: Meaning::Arguments,
            ..input.domain()
        },
        [],
        Arc::new(Field {
            grain_size: model.grain_size,
            seed,
        }),
    );
    let application = value::Value::captured(
        input.domain(),
        [input.clone(), field],
        Arc::new(GrainOperation { model, seed }),
    );
    Ok(application)
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct GrainOperation {
    model: crate::grain::GrainParameters,
    seed: u64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct Field {
    grain_size: f64,
    seed: u64,
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FieldParameters {
    shape: [f32; 4],
    seed: [u32; 4],
}

impl Operation for Field {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(("source-anchored grain field v1", *self)))
    }
    fn access(&self) -> value::Access {
        value::Access::Neighborhood
    }
    fn reads(&self, _: CellRegion) -> Result<Vec<Read>, Error> {
        Ok(Vec::new())
    }
    fn calculation(
        self: std::sync::Arc<Self>,
        _: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _: Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, Error> {
        let footprint = domain.grid.raster(output).footprint();
        let parameters = FieldParameters {
            shape: [self.grain_size as f32, footprint as f32, 0., 0.],
            seed: [crate::gpu::grain::seed(self.seed), 0, 0, 0],
        };
        let code = format!(
            "{}\nstruct FieldParameters {{ shape:vec4f, seed:vec4u }}\nfn pixel(cell:vec2i,position:vec2f,p:FieldParameters)->vec4f {{ return vec4f(grain_field(position,p.shape.x,p.shape.y,p.seed.x),0.0); }}",
            include_str!("../grain/field.wgsl")
        );
        value::Calculation::new(
            domain,
            output,
            Vec::new(),
            value::Operation::new(
                "source-anchored grain field v1",
                (self.grain_size, self.seed),
                code,
                "pixel",
                value::Access::Neighborhood,
            )
            .with_uniform("FieldParameters", &parameters),
        )
        .map(Some)
    }
}

impl Operation for GrainOperation {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(*self))
    }
    fn reads(&self, region: CellRegion) -> Result<Vec<Read>, Error> {
        Ok(vec![Read::new(0, region), Read::new(1, region)])
    }
    fn calculation(
        self: std::sync::Arc<Self>,
        inputs: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _: Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, Error> {
        let inputs = crate::gpu::evaluation::value::Calculation::references(inputs);
        let raster = domain.grid.raster(output);
        let code=[
            include_str!("../raster.wgsl"),include_str!("../working_range.wgsl"),
            include_str!("../grain/parameters.wgsl"),include_str!("../grain/apply.wgsl"),
            "fn pixel(cell:vec2i,position:vec2f,source:vec4f,p:Grain)->vec4f { return apply_grain(source,input_1(cell).rgb,p); }"
        ].join("\n");
        let operation = value::Operation::new(
            "photographic grain v1",
            *self,
            code,
            "pixel",
            value::Access::Pointwise,
        )
        .with_input_access(vec![value::Access::Pointwise, value::Access::Neighborhood])
        .with_uniform(
            "Grain",
            &crate::gpu::grain::parameter_data(raster, self.model, self.seed),
        );
        value::Calculation::new(
            domain,
            output,
            vec![inputs[0].request(output)?, inputs[1].request(output)?],
            operation,
        )
        .map(Some)
    }
}
