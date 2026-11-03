use super::*;
use crate::{
    gpu::{
        SubmissionGate,
        evaluation::{self, Budget, CellRegion, Domain, Grid, Meaning, Region, Source},
    },
    looks::LutChain,
    rendering::{OutputReference, OutputTarget},
};
use std::sync::Arc;

const WIDTH: u32 = 2049;

struct Ramp(Arc<Vec<[f32; 4]>>);
impl Source for Ramp {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some(CellRegion::new([0, 0], [WIDTH, 1])),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::Graded,
        }
    }
    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        assert_eq!((region.y, region.height), (0, 1));
        Ok(self.0[region.x as usize..region.right() as usize].to_vec())
    }
}

#[test]
fn all_materials_keep_neutral_exposure_order_through_both_delivered_targets() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let gate = SubmissionGate::default();
    let pixels = Arc::new(
        (0..WIDTH)
            .map(|i| {
                let ev = -6.5 + 13. * f64::from(i) / f64::from(WIDTH - 1);
                let v = (0.18 * ev.exp2()) as f32;
                [v, v, v, 0.]
            })
            .collect::<Vec<_>>(),
    );
    let source = evaluation::source(Arc::new(Ramp(pixels)));
    let domain = source.domain();
    for paper in Paper::all() {
        for negative in std::iter::once(None).chain(Negative::all().map(Some)) {
            let selection = Selection { negative, paper };
            let asset = Arc::new(
                Model::new(selection)
                    .bake(
                        crate::looks::authoring::DEFAULT_GRID,
                        &AtomicBool::new(false),
                    )
                    .unwrap(),
            );
            let chain = LutChain {
                lut: None,
                film_and_paper: Some(asset),
            };
            for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
                let image = evaluation::Reference::capture(
                    &device,
                    &renderer,
                    OutputReference::DisplayReferred(target),
                    &chain,
                )
                .unwrap()
                .compose(source.clone(), &renderer)
                .unwrap();
                let request = renderer
                    .evaluation()
                    .request(
                        image,
                        Region {
                            bounds: domain.finite().unwrap(),
                            grid: domain.grid,
                        },
                        Budget {
                            tile: [256, 1],
                            scratch_bytes: 8 << 20,
                        },
                    )
                    .unwrap();
                let mut encoded = vec![[f64::NAN; 3]; WIDTH as usize];
                let completed = renderer
                    .evaluation()
                    .read(
                        &queue,
                        &gate,
                        &request,
                        || false,
                        |region, pixels| {
                            for (x, pixel) in pixels.iter().enumerate() {
                                assert!(pixel.iter().all(|v| v.is_finite()) && pixel[3] >= 0.);
                                for c in 0..3 {
                                    encoded[region.x as usize + x][c] =
                                        target.encode(pixel[c] as f64)?;
                                }
                            }
                            Ok(())
                        },
                    )
                    .unwrap();
                assert!(completed.is_some());
                assert!(encoded.iter().flatten().all(|v| v.is_finite()));
                for c in 0..3 {
                    // Increasing exposure must not reverse this neutral response.
                    // Allow f32 output-transform roundoff, not a visibility limit.
                    for pair in encoded.windows(2) {
                        assert!(
                            pair[1][c] + 64. * f64::from(f32::EPSILON) >= pair[0][c],
                            "{} {target:?} reversed: {pair:?}",
                            selection.name()
                        );
                    }
                    // Reject a constant output that would trivially preserve order.
                    assert!(encoded[WIDTH as usize - 1][c] - encoded[0][c] > 0.1);
                }
            }
        }
    }
}
