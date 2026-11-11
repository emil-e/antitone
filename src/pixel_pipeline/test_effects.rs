//! An independently supplied finite-neighbourhood component used by composition tests.

use crate::gpu::evaluation::{self, Domain, Operation, value};
use crate::{
    Error, gpu::evaluation::CellRegion as Rect, pixel_pipeline::EffectDefinition,
    processing_support::Support,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NeighbourEffect {
    pub radius: f64,
    pub gain: f32,
}

impl EffectDefinition for NeighbourEffect {
    fn compose(&self, input: evaluation::Value) -> Result<evaluation::Value, Error> {
        let neighbourhood = evaluation::Value::from_operation(
            input.domain(),
            Some(input.clone()),
            Neighbour {
                domain: input.domain(),
                margin: Support([
                    (self.radius / input.domain().grid.pitch[0]).ceil() as u32,
                    0,
                ]),
                gain: self.gain,
            },
        );
        Ok(evaluation::pointwise(
            neighbourhood,
            input.domain().meaning,
            evaluation::Function {
                code: r#"
fn independent_source_bias(pixel: vec4f, position: vec2f) -> vec4f {
    return vec4f(pixel.rgb + vec3f(position.x * 0.000001 + position.y * 0.000002), pixel.w);
}
"#
                .into(),
                symbol: "independent_source_bias",
                bindings: vec![],
            },
        ))
    }
}

struct Neighbour {
    domain: Domain,
    margin: Support,
    gain: f32,
}

impl Operation for Neighbour {
    fn reads(&self, output: Rect) -> Result<Vec<crate::gpu::evaluation::Read>, Error> {
        Ok(vec![crate::gpu::evaluation::Read::new(
            0,
            output.expanded(self.margin.0, self.domain.bounds)?,
        )])
    }

    fn calculation(
        self: std::sync::Arc<Self>,
        inputs: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        output: Rect,
        _budget: crate::gpu::evaluation::Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, Error> {
        let inputs = crate::gpu::evaluation::value::Calculation::references(inputs);
        let read = output.expanded(self.margin.0, self.domain.bounds)?;
        let bounds = self.domain.bounds.unwrap_or(read);
        value::Calculation::new(
            domain,
            output,
            vec![inputs[0].request(read)?],
            value::Operation::new(
                "independent neighbour fixture",
                (self.margin, self.gain),
                r#"
struct Parameters { bounds:vec4i, values:vec4f }
fn pixel(p:vec2i,at:vec2f,parameters:Parameters)->vec4f{
    let center=input_0(p);
    let radius=i32(parameters.values.x);
    let left=input_0(clamp(p-vec2i(radius,0),parameters.bounds.xy,parameters.bounds.zw-vec2i(1)));
    let right=input_0(clamp(p+vec2i(radius,0),parameters.bounds.xy,parameters.bounds.zw-vec2i(1)));
    return vec4f((left.rgb+2.*center.rgb+right.rgb)*0.25*parameters.values.y,center.w);
}
"#,
                "pixel",
                value::Access::Neighborhood,
            )
            .with_uniform(
                "Parameters",
                &[
                    [bounds.x, bounds.y, bounds.right(), bounds.bottom()].map(|value| value as u32),
                    [self.margin.0[0] as f32, self.gain, 0., 0.].map(f32::to_bits),
                ],
            ),
        )
        .map(Some)
    }
}
