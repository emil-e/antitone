//! Bounded view acquisition quality, independent of optional pixel retention.

use super::PixelPipeline;
use crate::{
    Error,
    acquisition::{Plan, Request, Sampling},
    frame_raster::FrameRaster,
};

/// Refinement allowance including real-neighbour padding. A larger presentation keeps its
/// requested cells, subject to the independent active-input and evaluator scratch limits.
const REFINED_CELLS: u64 = 5 * 1024 * 1024;
const ACTIVE_CELLS: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct ViewSampling {
    pub presentation: FrameRaster,
    pub ideal: [u32; 2],
    pub plan: Plan,
}

impl ViewSampling {
    pub fn new(
        presentation: FrameRaster,
        pipeline: &PixelPipeline,
        sampling: Sampling,
    ) -> Result<Self, Error> {
        let build = |output| {
            Plan::new(Request {
                output,
                stages: pipeline,
                sampling,
            })
        };
        let base = build(presentation)?;
        let base_cells = cells(&base)?;
        if base_cells > ACTIVE_CELLS {
            return Err(Error::InvalidParameter(
                "view acquisition exceeds active-cell budget",
            ));
        }
        let native_pitch = match sampling {
            Sampling::CameraProxy { region, size } => (f64::from(region.width)
                / f64::from(size[0]))
            .max(f64::from(region.height) / f64::from(size[1]))
            .max(1.),
            _ => 1.,
        };
        let mut pitch = f64::INFINITY;
        if !matches!(sampling, Sampling::CameraCells { .. }) {
            for stage in &pipeline.stages {
                if let Some(preferred) = stage.view_pitch() {
                    if !preferred.is_finite() || preferred <= 0. {
                        return Err(Error::InvalidParameter(
                            "invalid effect view-sampling pitch",
                        ));
                    }
                    pitch = pitch.min(preferred.max(native_pitch));
                }
            }
        }
        // Subdivide destination cells integrally: two area integrations then retain the
        // exact destination aperture, instead of introducing a second pixel-box blur.
        let factors: [u32; 2] = std::array::from_fn(|i| {
            let extent = presentation.max.0[i] - presentation.min.0[i];
            let presented_pitch = extent / f64::from(presentation.output[i]);
            let wanted = (presented_pitch / pitch).ceil().max(1.);
            let available = (presented_pitch / native_pitch).floor().max(1.);
            wanted
                .min(available)
                .min(f64::from(u32::MAX / presentation.output[i])) as u32
        });
        let ideal = std::array::from_fn(|i| presentation.output[i] * factors[i]);
        let budget = REFINED_CELLS.max(base_cells);
        let candidate = |fraction: f64| -> Result<Plan, Error> {
            let output = std::array::from_fn(|i| {
                presentation.output[i] * (1 + (f64::from(factors[i] - 1) * fraction).floor() as u32)
            });
            build(FrameRaster {
                output,
                ..presentation
            })
        };
        let desired = candidate(1.)?;
        let mut plan = if cells(&desired)? <= budget {
            desired
        } else {
            // Include padding in admission, not only the visible working rectangle. The
            // captured grid is the bounded request; no runtime cache-pressure fallback exists.
            let mut low = 0.;
            let mut high = 1.;
            let mut selected = base;
            for _ in 0..32 {
                let mid = (low + high) * 0.5;
                let next = candidate(mid)?;
                if cells(&next)? <= budget {
                    selected = next;
                    low = mid;
                } else {
                    high = mid;
                }
            }
            selected
        };
        // Discrete factors can leave room to resolve one axis of a wide/nonuniform
        // view. Spend that remainder on the least-resolved axis first, without a loop
        // proportional to source dimensions.
        let mut axes = [0, 1];
        axes.sort_by(|a, b| {
            let unresolved = |i: usize| {
                (presentation.max.0[i] - presentation.min.0[i]) / f64::from(plan.output().output[i])
            };
            unresolved(*b).total_cmp(&unresolved(*a))
        });
        for axis in axes {
            let mut low = plan.output().output[axis] / presentation.output[axis];
            let mut high = factors[axis];
            while low < high {
                let mid = low + (high - low).div_ceil(2);
                let mut output = plan.output();
                output.output[axis] = presentation.output[axis] * mid;
                let next = build(output)?;
                if cells(&next)? <= budget {
                    plan = next;
                    low = mid;
                } else {
                    high = mid - 1;
                }
            }
        }
        Ok(Self {
            presentation,
            ideal,
            plan,
        })
    }

    pub fn limited(&self) -> bool {
        self.plan.output().output != self.ideal
    }
}

fn cells(plan: &Plan) -> Result<u64, Error> {
    let size = plan.processing_frame()?.output;
    Ok(u64::from(size[0]) * u64::from(size[1]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        geometry::{Composition, FramePoint},
        image_mtf::ImageMtfParameters,
        pixel_pipeline::PixelStage,
    };

    fn frame(side: u32, output: [u32; 2]) -> FrameRaster {
        FrameRaster::new(
            Composition::new(
                [17, 23, side, side].into(),
                Default::default(),
                Default::default(),
            )
            .unwrap(),
            FramePoint([0.; 2]),
            FramePoint([f64::from(side); 2]),
            output,
        )
        .unwrap()
    }

    #[test]
    fn preferred_pitch_refines_source_acquisition_not_presentation_or_native_scale() {
        for side in [600, 4096, 8640] {
            let frame = frame(side, [113, 97]);
            let pipeline = PixelPipeline {
                stages: vec![PixelStage::new(ImageMtfParameters {
                    sigma: f64::from(side) * 0.0004,
                })],
            };
            let sampled = ViewSampling::new(frame, &pipeline, Sampling::NeutralFootprints).unwrap();
            assert_eq!(sampled.presentation, frame);
            let acquired = sampled.plan.processing_frame().unwrap();
            assert!(acquired.output[0] > frame.output[0]);
            assert!(cells(&sampled.plan).unwrap() <= REFINED_CELLS);
            assert!(sampled.plan.pitch().iter().all(|p| *p >= 1. - 1e-12));
            assert_eq!(sampled.plan.pipeline(), &pipeline);
            assert!(
                !sampled.limited(),
                "ordinary default-scale refinement fits its allowance"
            );
        }
    }

    #[test]
    fn no_effect_is_not_refined_and_proxy_cells_are_not_invented() {
        let frame = frame(6000, [512; 2]);
        let clean = ViewSampling::new(
            frame,
            &PixelPipeline::default(),
            Sampling::NeutralFootprints,
        )
        .unwrap();
        assert_eq!(clean.plan.output(), frame);
        let pipeline = PixelPipeline {
            stages: vec![PixelStage::new(ImageMtfParameters { sigma: 2.4 })],
        };
        let proxy = ViewSampling::new(
            frame,
            &pipeline,
            Sampling::CameraProxy {
                region: frame.composition.area(),
                size: [512; 2],
            },
        )
        .unwrap();
        assert_eq!(proxy.plan.output(), frame);
        let small = ViewSampling::new(frame, &pipeline, Sampling::NeutralFootprints).unwrap();
        assert!(small.plan.output().output[0] > 512);
    }

    #[test]
    fn extreme_refinement_and_wide_views_have_bounded_required_input() {
        for output in [[512, 512], [4096, 512], [3840, 2160]] {
            let frame = frame(9000, output);
            let pipeline = PixelPipeline {
                stages: vec![PixelStage::new(ImageMtfParameters { sigma: 0.45 })],
            };
            let sampled = ViewSampling::new(frame, &pipeline, Sampling::NeutralFootprints).unwrap();
            assert!(sampled.limited());
            assert!(
                cells(&sampled.plan).unwrap()
                    <= REFINED_CELLS.max(u64::from(output[0] + 2) * u64::from(output[1] + 2))
            );
            assert!((0..2).all(|i| sampled.plan.output().output[i] >= output[i]));
        }
        assert!(
            ViewSampling::new(
                frame(9000, [8192; 2]),
                &PixelPipeline::default(),
                Sampling::NeutralFootprints
            )
            .is_err()
        );
    }
}
