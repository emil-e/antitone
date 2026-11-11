//! Test-only authoring examples: logical values compose nodes without a device or consumer hooks.

use crate::{
    Error,
    gpu::evaluation::CellRegion as Rect,
    gpu::evaluation::{self, Domain, Function, Meaning, Operation, Value, value},
    pixel_pipeline::EffectDefinition,
    processing_support::Support,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SignedCurve {
    gain: f32,
    curvature: f32,
}

impl SignedCurve {
    pub fn new(gain: f32, curvature: f32) -> Result<Self, Error> {
        if !gain.is_finite() || !curvature.is_finite() || curvature < 0. {
            return Err(Error::InvalidParameter("invalid example curve parameters"));
        }
        Ok(Self { gain, curvature })
    }
}

impl EffectDefinition for SignedCurve {
    fn compose(&self, input: Value) -> Result<Value, Error> {
        if input.domain().meaning != Meaning::Graded {
            return Err(Error::InvalidParameter("example curve requires graded AP1"));
        }
        let parameters = [
            self.gain,
            self.curvature,
            input.domain().grid.coverage as f32,
            0.,
        ];
        Ok(evaluation::pointwise(
            input,
            Meaning::Graded,
            Function::new(
                include_str!("fixtures/signed_curve.wgsl"),
                "apply_signed_curve",
            )
            .with_uniform(18, &parameters),
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TentConvolution {
    spacing: f64,
}

impl TentConvolution {
    pub fn new(spacing: f64) -> Result<Self, Error> {
        if !spacing.is_finite() || spacing <= 0. {
            return Err(Error::InvalidParameter(
                "invalid example convolution spacing",
            ));
        }
        Ok(Self { spacing })
    }
}

impl EffectDefinition for TentConvolution {
    fn compose(&self, input: Value) -> Result<Value, Error> {
        if input.domain().meaning != Meaning::Graded {
            return Err(Error::InvalidParameter(
                "example convolution requires graded AP1",
            ));
        }
        let support = Support(
            input
                .domain()
                .grid
                .pitch
                .map(|p| (self.spacing / p).ceil() as u32),
        );
        if support.0.iter().any(|r| *r > i32::MAX as u32) {
            return Err(Error::InvalidParameter(
                "example convolution exceeds GPU coordinates",
            ));
        }
        Ok(Value::from_operation(
            input.domain(),
            Some(input.clone()),
            Tent {
                domain: input.domain(),
                support,
            },
        ))
    }
}

struct Tent {
    domain: Domain,
    support: Support,
}

impl Operation for Tent {
    fn reads(&self, output: Rect) -> Result<Vec<crate::gpu::evaluation::Read>, Error> {
        Ok(vec![crate::gpu::evaluation::Read::new(
            0,
            output.expanded(self.support.0, self.domain.bounds)?,
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
        let horizontal = output.expanded([0, self.support.0[1]], domain.bounds)?;
        let read = horizontal.expanded([self.support.0[0], 0], domain.bounds)?;
        let mut input = inputs[0].request(read)?;
        let bounds = domain.bounds.unwrap_or(read);
        for (axis, coverage) in [(0, horizontal), (1, output)] {
            input = value::Calculation::new(
                domain,
                coverage,
                vec![input],
                value::Operation::new(
                    "example tent axis",
                    (self.support, axis, domain.grid.coverage),
                    include_str!("fixtures/tent_convolution.wgsl"),
                    "convolve",
                    value::Access::Neighborhood,
                )
                .with_uniform(
                    "Parameters",
                    &[
                        [bounds.x, bounds.y, bounds.right(), bounds.bottom()]
                            .map(|value| value as u32),
                        [self.support.0[axis], axis as u32, domain.grid.coverage, 0],
                    ],
                ),
            )?;
        }
        Ok(Some(input))
    }
}
