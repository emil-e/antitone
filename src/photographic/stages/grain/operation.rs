//! Source-anchored grain owns field generation and its photographic application.

use super::GrainParameters;
use crate::Error;
use crate::{
    acquisition::raster::Raster,
    evaluation::{
        CellRegion, Domain, Operation, Read, RetainedKey, Tiling, Value,
        value::{self, Access, Calculation},
    },
};
use std::sync::Arc;

impl GrainParameters {
    pub(crate) fn compose(self, input: Value, seed: u64) -> Value {
        if self.amplitude <= 0. {
            return input;
        }

        let domain = input.domain();
        let field = Value::from_operation(
            domain,
            [],
            Field {
                grain_size: self.grain_size,
                seed,
            },
        );
        Value::from_operation(domain, [input, field], GrainOperation { model: self, seed })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Hash)]
struct GrainOperation {
    model: GrainParameters,
    seed: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Field {
    grain_size: f64,
    seed: u64,
}

impl std::hash::Hash for Field {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        crate::float_hash::f64(self.grain_size, state);
        self.seed.hash(state);
    }
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

    fn access(&self) -> Access {
        Access::Neighborhood
    }

    fn reads(&self, _: CellRegion) -> Result<Vec<Read>, Error> {
        Ok(Vec::new())
    }

    fn calculation(
        self: Arc<Self>,
        _: &[Value],
        domain: Domain,
        output: CellRegion,
        _: Tiling,
    ) -> Result<Option<Calculation>, Error> {
        let footprint = domain.grid.raster(output).footprint();
        let parameters = FieldParameters {
            shape: [self.grain_size as f32, footprint as f32, 0., 0.],
            seed: [pack_seed(self.seed), 0, 0, 0],
        };
        let code = format!(
            "{}\nstruct FieldParameters {{ shape:vec4f, seed:vec4u }}\nfn pixel(cell:vec2i,position:vec2f,p:FieldParameters)->vec4f {{ return vec4f(grain_field(position,p.shape.x,p.shape.y,p.seed.x),0.0); }}",
            include_str!("field.wgsl")
        );
        Calculation::new(
            domain,
            output,
            Vec::new(),
            value::Operation::new(
                "source-anchored grain field v1",
                *self,
                code,
                "pixel",
                Access::Neighborhood,
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
        self: Arc<Self>,
        inputs: &[Value],
        domain: Domain,
        output: CellRegion,
        _: Tiling,
    ) -> Result<Option<Calculation>, Error> {
        let inputs = Calculation::references(inputs);
        let raster = domain.grid.raster(output);
        let code = [
            include_str!("../../../acquisition/raster.wgsl"),
            include_str!("../working_range.wgsl"),
            include_str!("parameters.wgsl"),
            include_str!("apply.wgsl"),
            "fn pixel(cell:vec2i,position:vec2f,source:vec4f,p:Grain)->vec4f { return apply_grain(source,input_1(cell).rgb,p); }"
        ]
        .join("\n");
        let operation = value::Operation::new(
            "photographic grain v1",
            *self,
            code,
            "pixel",
            Access::Pointwise,
        )
        .with_input_access(vec![Access::Pointwise, Access::Neighborhood])
        .with_uniform("Grain", &parameter_data(raster, self.model, self.seed));
        Calculation::new(
            domain,
            output,
            vec![inputs[0].request(output)?, inputs[1].request(output)?],
            operation,
        )
        .map(Some)
    }
}

#[cfg(all(test, feature = "editor"))]
pub(crate) fn is_field_key(key: &RetainedKey) -> bool {
    key.get::<(&'static str, Field)>()
        .is_some_and(|(name, _)| *name == "source-anchored grain field v1")
}

#[cfg(test)]
pub(crate) fn shader() -> String {
    [
        include_str!("../../../acquisition/raster.wgsl"),
        include_str!("../working_range.wgsl"),
        include_str!("parameters.wgsl"),
        include_str!("bindings.wgsl"),
        include_str!("apply.wgsl"),
        include_str!("field.wgsl"),
        include_str!("kernels.wgsl"),
    ]
    .join("\n")
}

fn pack_seed(value: u64) -> u32 {
    value as u32 ^ ((value >> 32) as u32).rotate_left(16)
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Parameters {
    raster: Raster,
    values: [f32; 4],
    variance: [f32; 4],
    seed: [u32; 4],
}
fn parameter_data(raster: Raster, model: GrainParameters, noise_seed: u64) -> Parameters {
    let variance = model.channel_variance(raster.footprint());
    Parameters {
        raster,
        values: [
            model.grain_size as f32,
            model.amplitude as f32,
            model.colour as f32,
            0.,
        ],
        variance: [
            variance[0] as f32,
            variance[1] as f32,
            variance[2] as f32,
            0.,
        ],
        seed: [pack_seed(noise_seed), 0, 0, 0],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluation::{Grid, borrowed_input};

    #[test]
    fn field_identity_distinguishes_seeds_with_the_same_packed_word() {
        let seeds = [1, 1 << 48];
        assert_eq!(pack_seed(seeds[0]), pack_seed(seeds[1]));

        let input = borrowed_input(Domain {
            bounds: Some(CellRegion::new([0; 2], [8; 2])),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        });
        let grain = GrainParameters {
            grain_size: 2.,
            amplitude: 0.055,
            colour: 0.5,
        };
        let [a, b] = seeds.map(|seed| grain.compose(input.clone(), seed));
        let same = grain.compose(input, seeds[0]);
        assert!(a.inputs()[1].same_definition(&same.inputs()[1]));
        assert!(!a.inputs()[1].same_definition(&b.inputs()[1]));
    }
}
