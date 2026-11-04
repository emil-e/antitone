use super::*;
use crate::{
    geometry::SourcePoint, gpu::evaluation::CellRegion as Rect, gpu::grading::Grading,
    processing::RollPipeline, rendering::OutputTarget,
};
use glam::DVec3;

#[derive(Clone)]
pub(super) struct Authored {
    domain: Domain,
    pixels: Arc<Vec<[f32; 4]>>,
}

impl Source for Authored {
    fn domain(&self) -> Domain {
        self.domain
    }

    fn read(&self, rect: Rect) -> Result<Vec<[f32; 4]>, Error> {
        assert!(self.domain.finite().unwrap().contains_region(rect));
        let area = self.domain.finite().unwrap();
        Ok((rect.y..rect.bottom())
            .flat_map(|y| (rect.x..rect.right()).map(move |x| self.pixels[area.index([x, y])]))
            .collect())
    }
}

pub(super) fn fixture() -> Authored {
    let area = Rect {
        x: 11,
        y: 17,
        width: 81,
        height: 67,
    };
    let pixels = (0..area.height)
        .flat_map(|y| {
            (0..area.width).map(move |x| {
                let base = [0.002, 0.03, 0.18, 0.7, 2.0, 4.0][((x * 3 + y * 5) % 6) as usize];
                let mark = if x == 37 && y == 29 { 1.7 } else { 0. };
                if x < 18 && y < 25 {
                    return [1.8, 0.003, 0.001, 0.];
                }
                [
                    base + mark,
                    base * 0.7 + (x as f32 * 0.003),
                    base * 0.5 + y as f32 * 0.002,
                    0.,
                ]
            })
        })
        .collect();
    Authored {
        domain: Domain {
            bounds: Some(area),
            grid: Grid::axis_aligned([0., 0.], [1., 1.]),
            meaning: Meaning::Camera,
        },
        pixels: Arc::new(pixels),
    }
}

use crate::gpu::photograph::test_support::{matrix, settings};

pub(super) fn reduced(domain: Domain, size: [u32; 2]) -> Domain {
    let area = domain.finite().unwrap();
    Domain {
        bounds: Some(crate::gpu::evaluation::CellRegion::new([0, 0], size)),
        grid: Grid::axis_aligned(
            domain.point([f64::from(area.x), f64::from(area.y)]),
            [
                f64::from(area.width) * domain.grid.pitch[0] / f64::from(size[0]),
                f64::from(area.height) * domain.grid.pitch[1] / f64::from(size[1]),
            ],
        ),
        meaning: domain.meaning,
    }
}

pub(super) fn chain(
    device: &wgpu::Device,
    renderer: &crate::renderer::GpuRenderer,
    data: Arc<dyn Source>,
    physical: bool,
    target: Option<OutputTarget>,
    sigma: f64,
) -> Arc<ImageNode> {
    let domain = data.domain();
    let mut node = interpret(device, source(data), matrix()).unwrap();
    if physical {
        node = reconstruct(node.clone(), reduced(node.domain, [41, 35])).unwrap();
    }
    let (roll, snapshot) = settings(domain.finite().unwrap());
    node = grade(
        node,
        device,
        Arc::new(
            Grading::new(
                &device.limits(),
                roll,
                snapshot,
                domain.finite().unwrap().source().unwrap(),
            )
            .unwrap(),
        ),
    )
    .unwrap();
    if sigma > 0. {
        node = gaussian(node, sigma).unwrap();
    }
    if let Some(target) = target {
        node = display(node, renderer, target).unwrap();
    }
    if !physical {
        node = reconstruct(node.clone(), reduced(node.domain, [41, 35])).unwrap();
    }
    node
}

pub(super) fn make_plan(node: Arc<ImageNode>, tile: [u32; 2]) -> Plan {
    Plan::new(
        node.clone(),
        node.domain.finite().unwrap(),
        Budget {
            tile,
            scratch_bytes: 32 << 20,
        },
    )
    .unwrap()
}

fn grade_scalar(values: Vec<DVec3>, domain: Domain, source_area: Rect) -> Vec<DVec3> {
    let (roll, snapshot) = settings(source_area);
    let roll = RollPipeline::new(roll).unwrap();
    let layers =
        crate::adjustment_layers::AdjustmentLayerStack::new(snapshot.layers().to_vec()).unwrap();
    values
        .into_iter()
        .enumerate()
        .map(|(i, p)| {
            let xy = [
                f64::from(domain.finite().unwrap().x)
                    + (i % domain.finite().unwrap().width as usize) as f64
                    + 0.5,
                f64::from(domain.finite().unwrap().y)
                    + (i / domain.finite().unwrap().width as usize) as f64
                    + 0.5,
            ];
            layers
                .apply_rgb(roll.apply(p).unwrap(), SourcePoint(domain.point(xy)))
                .unwrap()
        })
        .collect()
}

fn blur_scalar(values: &[DVec3], domain: Domain, sigma: f64) -> Vec<DVec3> {
    if sigma == 0. {
        return values.to_vec();
    }
    let [width, height] = [
        domain.finite().unwrap().width as usize,
        domain.finite().unwrap().height as usize,
    ];
    let mut image = values.to_vec();
    for axis in 0..2 {
        let s = sigma / domain.grid.pitch[axis];
        let radius = (4. * s).ceil() as isize;
        let norm: f64 = (-radius..=radius)
            .map(|d| (-0.5 * (d * d) as f64 / (s * s)).exp())
            .sum();
        image = (0..image.len())
            .map(|index| {
                (-radius..=radius).fold(DVec3::ZERO, |sum, d| {
                    let x = (index % width) as isize + if axis == 0 { d } else { 0 };
                    let y = (index / width) as isize + if axis == 1 { d } else { 0 };
                    let sample = image[y.clamp(0, height as isize - 1) as usize * width
                        + x.clamp(0, width as isize - 1) as usize];
                    sum + sample * ((-0.5 * (d * d) as f64 / (s * s)).exp() / norm)
                })
            })
            .collect();
    }
    image
}

pub(super) fn reconstruct_scalar(values: &[DVec3], input: Domain, output: Domain) -> Vec<DVec3> {
    let area = output.finite().unwrap();
    (area.y..area.bottom())
        .flat_map(|y| {
            (area.x..area.right()).map(move |x| {
                let lo = output.point([f64::from(x), f64::from(y)]);
                let hi = output.point([f64::from(x + 1), f64::from(y + 1)]);
                let mut sum = DVec3::ZERO;
                let mut mass = 0.;
                for sy in input.finite().unwrap().y..input.finite().unwrap().bottom() {
                    for sx in input.finite().unwrap().x..input.finite().unwrap().right() {
                        let a = input.point([f64::from(sx), f64::from(sy)]);
                        let b = input.point([f64::from(sx + 1), f64::from(sy + 1)]);
                        let w = (hi[0].min(b[0]) - lo[0].max(a[0])).max(0.)
                            * (hi[1].min(b[1]) - lo[1].max(a[1])).max(0.);
                        sum += values[input.finite().unwrap().index([sx, sy])] * w;
                        mass += w;
                    }
                }
                sum / mass
            })
        })
        .collect()
}

pub(super) fn scalar(
    data: &Authored,
    physical: bool,
    target: Option<OutputTarget>,
    sigma: f64,
) -> Vec<DVec3> {
    let mut domain = Domain {
        meaning: Meaning::Neutral,
        ..data.domain
    };
    let mut pixels = data
        .pixels
        .iter()
        .map(|p| matrix() * DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64))
        .collect::<Vec<_>>();
    if physical {
        let next = reduced(domain, [41, 35]);
        pixels = reconstruct_scalar(&pixels, domain, next);
        domain = next;
    }
    pixels = grade_scalar(pixels, domain, data.domain.finite().unwrap());
    pixels = blur_scalar(&pixels, domain, sigma);
    #[cfg(feature = "ocio-reference")]
    if let Some(target) = target {
        pixels = crate::rendering::CpuRenderer::new(target)
            .unwrap()
            .render(&pixels)
            .unwrap()
            .into_iter()
            .map(|p| p.as_dvec3())
            .collect();
    }
    #[cfg(not(feature = "ocio-reference"))]
    assert!(target.is_none());
    if !physical {
        pixels = reconstruct_scalar(&pixels, domain, reduced(domain, [41, 35]));
    }
    pixels
}

pub(super) fn checked_error(expected: &[DVec3], actual: &[[f32; 4]]) -> Result<f64, &'static str> {
    if expected.len() != actual.len() {
        return Err("pixel count differs");
    }
    if expected.iter().any(|v| !v.is_finite()) || actual.iter().flatten().any(|v| !v.is_finite()) {
        return Err("nonfinite comparison sample");
    }
    Ok(expected
        .iter()
        .zip(actual)
        .map(|(e, a)| {
            (*e - DVec3::new(a[0] as f64, a[1] as f64, a[2] as f64))
                .abs()
                .max_element()
        })
        .fold(0., f64::max))
}

pub(super) fn max_error(expected: &[DVec3], actual: &[[f32; 4]]) -> f64 {
    checked_error(expected, actual).expect("comparison requires complete finite images")
}

#[test]
fn composed_real_grade_and_multi_pass_fragment_match_independent_signed_hdr_edges_and_seams() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let data = fixture();
    let node = chain(&device, &renderer, Arc::new(data.clone()), false, None, 0.9);
    let plan = make_plan(node, [13, 11]);
    assert_eq!(plan.domain.meaning, Meaning::Graded);
    assert!(plan.peak_scratch_bytes <= 32 << 20);
    assert!(plan.tiles.len() > 1);
    let first = &plan.tiles[0];
    let maps = first
        .fragment
        .work
        .iter()
        .filter(|w| {
            matches!(
                w,
                Work::Compute {
                    entry: "sample_main",
                    ..
                }
            )
        })
        .count();
    assert_eq!(
        maps, 1,
        "interpretation and actual photographic shader functions share one dispatch"
    );
    assert!(first.last_use[first.result.resource.0] == first.fragment.work.len());
    assert!(
        first.slots.len() < first.fragment.resources.len(),
        "declared last uses allow scratch reuse"
    );
    let mut executor = execute::Executor::default();
    let result = executor
        .run(
            &device,
            &queue,
            &crate::gpu::submission::SubmissionGate::default(),
            &plan,
            || false,
        )
        .unwrap()
        .unwrap();
    assert_eq!(result.counts.submissions, plan.tiles.len());
    let actual = execute::read(&device, &queue, &result.pixels);
    let expected = scalar(&data, false, None, 0.9);
    let error = max_error(&expected, &actual);
    assert!(error < 2e-5, "independent signed/HDR model error {error}");
    assert!(expected.iter().any(|v| v.min_element() < 0.));
    assert!(expected.iter().any(|v| v.max_element() > 1.));
    let control = scalar(&data, false, None, 0.);
    assert!(
        control
            .iter()
            .zip(&expected)
            .any(|(a, b)| (*a - *b).abs().max_element() > 1e-3),
        "fixture must detect omitted Gaussian"
    );
    let whole = make_plan(
        chain(&device, &renderer, Arc::new(data.clone()), false, None, 0.9),
        [41, 35],
    );
    let whole = executor
        .run(
            &device,
            &queue,
            &crate::gpu::submission::SubmissionGate::default(),
            &whole,
            || false,
        )
        .unwrap()
        .unwrap();
    let whole = execute::read(&device, &queue, &whole.pixels);
    let omitted = make_plan(
        chain(&device, &renderer, Arc::new(data), false, None, 0.),
        [13, 11],
    );
    let omitted = executor
        .run(&device, &queue, &Default::default(), &omitted, || false)
        .unwrap()
        .unwrap();
    let omitted = execute::read(&device, &queue, &omitted.pixels);
    assert!(
        max_error(&expected, &omitted) > 8. * 2e-5,
        "actual dispatch-omission control"
    );
    assert_eq!(whole.len(), actual.len());
    assert!(
        whole
            .iter()
            .zip(&actual)
            .all(|(a, b)| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 2e-5))
    );
}

#[cfg(feature = "ocio-reference")]
#[test]
fn both_sampling_orders_are_compositions_with_independent_ocio_expectations() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let data = fixture();
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let mut results = Vec::new();
        for physical in [false, true] {
            let p = make_plan(
                chain(
                    &device,
                    &renderer,
                    Arc::new(data.clone()),
                    physical,
                    Some(target),
                    0.9,
                ),
                [13, 11],
            );
            let result = execute::Executor::default()
                .run(
                    &device,
                    &queue,
                    &crate::gpu::submission::SubmissionGate::default(),
                    &p,
                    || false,
                )
                .unwrap()
                .unwrap();
            let actual = execute::read(&device, &queue, &result.pixels);
            let expected = scalar(&data, physical, Some(target), 0.9);
            let error = max_error(&expected, &actual);
            assert!(
                error < 2e-5,
                "physical={physical} target={target:?} error={error}"
            );
            results.push(expected);
        }
        assert!(
            results[0]
                .iter()
                .zip(&results[1])
                .any(|(a, b)| (*a - *b).abs().max_element() > 1e-3),
            "fixture must distinguish noncommuting sampling orders"
        );
    }
}
