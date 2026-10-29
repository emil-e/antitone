//! Test-only authoring examples: logical values compose nodes without a device or consumer hooks.
use crate::{
    Error,
    geometry::SourceRegion as Rect,
    gpu::evaluation::{
        self, Binding, Domain, Fragment, Function, ImageNode, Meaning, Operation, PlannedImage,
    },
    pixel_pipeline::EffectDefinition,
    processing_support::Support,
};
use std::sync::Arc;

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
    fn support(&self, _: [f64; 2]) -> Support {
        Support::NONE
    }
    fn compose(&self, input: Arc<ImageNode>) -> Result<Arc<ImageNode>, Error> {
        if input.domain.meaning != Meaning::Graded {
            return Err(Error::InvalidParameter("example curve requires graded AP1"));
        }
        let parameters = [
            self.gain,
            self.curvature,
            input.domain.grid.coverage as f32,
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
    fn support(&self, pitch: [f64; 2]) -> Support {
        Support(pitch.map(|p| (self.spacing / p).ceil() as u32))
    }
    fn compose(&self, input: Arc<ImageNode>) -> Result<Arc<ImageNode>, Error> {
        if input.domain.meaning != Meaning::Graded {
            return Err(Error::InvalidParameter(
                "example convolution requires graded AP1",
            ));
        }
        let support = self.support(input.domain.grid.pitch);
        if support.0.iter().any(|r| *r > i32::MAX as u32) {
            return Err(Error::InvalidParameter(
                "example convolution exceeds GPU coordinates",
            ));
        }
        Ok(ImageNode::new(
            input.domain,
            Some(input.clone()),
            Tent {
                domain: input.domain,
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
    fn input(&self, output: Rect) -> Result<Option<Rect>, Error> {
        Ok(Some(self.support.expand(output, self.domain.bounds)))
    }
    fn lower(
        &self,
        work: &mut Fragment,
        input: Option<PlannedImage>,
        output: PlannedImage,
    ) -> Result<(), Error> {
        let input = input.unwrap();
        let horizontal = work.image(input.domain, input.region);
        // Only the requested columns of the horizontal result are consumed by the vertical
        // pass. Their full input halo is supplied even when an evaluation tile ends here.
        for (axis, source, destination) in [(0, input, horizontal), (1, horizontal, output)] {
            let parameters = work.uniform(&[
                [
                    source.region.x,
                    source.region.y,
                    source.region.width,
                    source.region.height,
                ],
                [
                    destination.region.x,
                    destination.region.y,
                    destination.region.width,
                    destination.region.height,
                ],
                [
                    self.support.0[axis],
                    axis as u32,
                    self.domain.grid.coverage,
                    0,
                ],
            ]);
            work.compute(
                include_str!("fixtures/tent_convolution.wgsl"),
                "convolve",
                vec![
                    (0, Binding::Read(source.resource)),
                    (1, Binding::Write(destination.resource)),
                    (2, parameters),
                ],
                destination.cells(),
            );
        }
        Ok(())
    }
}
