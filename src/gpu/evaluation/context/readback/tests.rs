use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

struct Pattern {
    domain: Domain,
    reads: Arc<AtomicUsize>,
}

impl Source for Pattern {
    fn domain(&self) -> Domain {
        self.domain
    }

    fn read(&self, region: SourceRegion) -> Result<Vec<[f32; 4]>, Error> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        Ok((region.y..region.y + region.height)
            .flat_map(|y| {
                (region.x..region.x + region.width)
                    .map(move |x| [-(x as f32) * 0.125, y as f32 * 0.25, 2., 13.])
            })
            .collect())
    }
}

fn image(reads: Arc<AtomicUsize>) -> Arc<ImageNode> {
    let domain = Domain {
        bounds: [11, 17, 7, 5].into(),
        grid: Grid {
            origin: [3., -5.],
            axes: [[0., 2.], [-3., 0.]],
            pitch: [2., 3.],
            coverage: 1,
        },
        meaning: Meaning::Graded,
    };
    pointwise(
        operators::source(Arc::new(Pattern { domain, reads })),
        Meaning::Graded,
        Function::new(
            r#"
@group(0) @binding(1)
var<uniform> parameters: vec4f;

fn transform(pixel: vec4f, position: vec2f) -> vec4f {
    let bias = position.x * 0.001 + position.y * 0.002;
    return vec4f(pixel.rgb * parameters.x + vec3f(bias), pixel.w);
}
"#,
            "transform",
        )
        .with_uniform(1, &[2f32, 0., 0., 0.]),
    )
}

fn request(context: &Context, image: Arc<ImageNode>, scratch_bytes: u64) -> Evaluation {
    context
        .request(
            image.clone(),
            Region {
                bounds: [12, 18, 5, 3].into(),
                grid: image.domain.grid,
            },
            Budget {
                tile: [2, 2],
                scratch_bytes,
            },
        )
        .unwrap()
}

#[test]
fn host_delivery_preserves_grid_coordinates_metadata_and_signed_hdr_values_across_blocks() {
    let (_device, queue, renderer) = crate::gpu::test_support::render_context();
    let context = renderer.evaluation();
    let reads = Arc::new(AtomicUsize::new(0));
    let image = image(reads.clone());
    let grid = image.domain.grid;
    let evaluation = request(context, image, 4096);
    let mut locations = std::collections::BTreeSet::new();

    let stats = context
        .read(
            &queue,
            &crate::gpu::SubmissionGate::default(),
            &evaluation,
            || false,
            |region, pixels| {
                assert!(region.width <= 2 && region.height <= 2);
                assert_eq!(pixels.len(), (region.width * region.height) as usize);
                for (index, pixel) in pixels.iter().enumerate() {
                    let x = region.x + index as u32 % region.width;
                    let y = region.y + index as u32 / region.width;
                    assert!(
                        locations.insert([x, y]),
                        "each requested cell is delivered once"
                    );
                    let position = grid.point([f64::from(x) + 0.5, f64::from(y) + 0.5]);
                    let bias = position[0] * 0.001 + position[1] * 0.002;
                    let expected = [
                        -f64::from(x) * 0.25 + bias,
                        f64::from(y) * 0.5 + bias,
                        4. + bias,
                    ];
                    for channel in 0..3 {
                        assert!((f64::from(pixel[channel]) - expected[channel]).abs() < 2e-5);
                    }
                    assert_eq!(pixel[3], 13.);
                }
                Ok(())
            },
        )
        .unwrap()
        .unwrap();

    assert_eq!(locations.len(), 15);
    assert_eq!(locations.first(), Some(&[12, 18]));
    assert_eq!(locations.last(), Some(&[16, 20]));
    assert_eq!(stats.submissions, 6);
    assert_eq!(stats.source_cells, 15);
    assert_eq!(reads.load(Ordering::Relaxed), 6);
}

#[test]
fn host_delivery_stops_before_acquisition_or_between_blocks_and_can_be_reused_after_errors() {
    let (_device, queue, renderer) = crate::gpu::test_support::render_context();
    let context = renderer.evaluation();
    let reads = Arc::new(AtomicUsize::new(0));
    let image = image(reads.clone());
    let evaluation = request(context, image.clone(), 4096);
    let gate = crate::gpu::SubmissionGate::default();
    let cancelled = AtomicBool::new(true);

    assert!(
        context
            .read(
                &queue,
                &gate,
                &evaluation,
                || cancelled.load(Ordering::Relaxed),
                |_, _| panic!("cancelled delivery")
            )
            .unwrap()
            .is_none()
    );
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    let invalid = request(context, image, 0);
    assert!(
        context
            .read(
                &queue,
                &gate,
                &invalid,
                || false,
                |_, _| panic!("inadmissible delivery")
            )
            .is_err()
    );
    assert_eq!(reads.load(Ordering::Relaxed), 0);

    cancelled.store(false, Ordering::Relaxed);
    assert!(
        context
            .read(
                &queue,
                &gate,
                &evaluation,
                || cancelled.load(Ordering::Relaxed),
                |_, _| {
                    cancelled.store(true, Ordering::Relaxed);
                    Ok(())
                }
            )
            .unwrap()
            .is_none()
    );
    assert_eq!(reads.load(Ordering::Relaxed), 1);

    let failure = context.read(
        &queue,
        &gate,
        &evaluation,
        || false,
        |_, _| Err(Error::InvalidParameter("consumer stopped")),
    );
    assert!(matches!(
        failure,
        Err(Error::InvalidParameter("consumer stopped"))
    ));
    assert_eq!(reads.load(Ordering::Relaxed), 2);

    let mut cells = 0;
    assert!(
        context
            .read(
                &queue,
                &gate,
                &evaluation,
                || false,
                |_, pixels| {
                    cells += pixels.len();
                    Ok(())
                }
            )
            .unwrap()
            .is_some()
    );
    assert_eq!(cells, 15);
    assert_eq!(reads.load(Ordering::Relaxed), 8);
}

#[test]
fn host_float_delivery_rejects_a_packed_result_before_copying() {
    struct Packed;

    impl Operation for Packed {
        fn input(&self, _: SourceRegion) -> Result<Option<SourceRegion>, Error> {
            Ok(None)
        }

        fn lower(
            &self,
            fragment: &mut Fragment,
            _: Option<PlannedImage>,
            output: PlannedImage,
        ) -> Result<(), Error> {
            fragment.resources[output.resource.0] = Storage::Buffer(u64::from(output.cells()) * 8);
            Ok(())
        }
    }

    let (_device, queue, renderer) = crate::gpu::test_support::render_context();
    let context = renderer.evaluation();
    let image = ImageNode::new(
        Domain {
            bounds: [0, 0, 1, 1].into(),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::Graded,
        },
        None,
        Packed,
    );
    let evaluation = context
        .request(
            image.clone(),
            Region {
                bounds: image.domain.bounds,
                grid: image.domain.grid,
            },
            Budget {
                tile: [1, 1],
                scratch_bytes: 64,
            },
        )
        .unwrap();
    assert!(matches!(
        context.read(
            &queue,
            &Default::default(),
            &evaluation,
            || false,
            |_, _| panic!("packed float delivery")
        ),
        Err(Error::InvalidParameter(_))
    ));
}
