use super::*;
use crate::{
    geometry::SourceRegion as Rect,
    pixel_pipeline::{
        EffectDefinition, PixelPipeline, PixelStage,
        authoring_examples::{SignedCurve, TentConvolution},
    },
};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone)]
struct Pattern {
    domain: Domain,
    pixels: Vec<[f32; 4]>,
    reads: Arc<AtomicUsize>,
}
impl Source for Pattern {
    fn domain(&self) -> Domain {
        self.domain
    }
    fn read(&self, region: Rect) -> Result<Vec<[f32; 4]>, Error> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        let bounds = self.domain.bounds;
        Ok((region.y..region.y + region.height)
            .flat_map(|y| {
                (region.x..region.x + region.width).map(move |x| {
                    self.pixels[((y - bounds.y) * bounds.width + x - bounds.x) as usize]
                })
            })
            .collect())
    }
}
fn pattern(coverage: u32) -> Pattern {
    let angle = 0.37f64;
    let (c, s) = (angle.cos(), angle.sin());
    let domain = Domain {
        bounds: [11, 17, 13, 11].into(),
        grid: Grid {
            origin: [127.25, 83.5],
            axes: [[2. * c, 2. * s], [-3. * s, 3. * c]],
            pitch: [2., 3.],
            coverage,
        },
        meaning: Meaning::Graded,
    };
    let pixels = (0..11)
        .flat_map(|y| {
            (0..13).map(move |x| {
                if coverage != 0 && x == 4 && y == 5 {
                    return [77., -50., 80., 0.];
                }
                let flags = ((x + 2 * y) % 8) | if coverage != 0 { 8 } else { 0 };
                [
                    if x < 6 { 3.5 + x as f32 * 0.03 } else { -0.25 },
                    0.12 + y as f32 * 0.02,
                    if y < 5 { 1.8 } else { -1.5 },
                    flags as f32,
                ]
            })
        })
        .collect();
    Pattern {
        domain,
        pixels,
        reads: Arc::new(AtomicUsize::new(0)),
    }
}
fn curve(pixels: &[[f32; 4]], coverage: u32, gain: f32, curvature: f32) -> Vec<[f64; 4]> {
    pixels
        .iter()
        .map(|p| {
            let mut out = p.map(f64::from);
            if p[3] >= 0. && (coverage == 0 || p[3] as u32 & 8 != 0) {
                for c in 0..3 {
                    let x = f64::from(p[c]);
                    out[c] = x / (1. + f64::from(curvature) * x.abs()) * f64::from(gain);
                }
            }
            out
        })
        .collect()
}
fn tent(pixels: &[[f64; 4]], size: [u32; 2], radii: [u32; 2], coverage: u32) -> Vec<[f64; 4]> {
    let mut values = pixels.to_vec();
    for axis in 0..2 {
        let input = values;
        values = (0..size[1])
            .flat_map(|y| (0..size[0]).map(move |x| [x, y]))
            .map(|xy| {
                let center = input[(xy[1] * size[0] + xy[0]) as usize];
                if center[3] < 0. || (coverage != 0 && center[3] as u32 & 8 == 0) {
                    return center;
                }
                let mut rgb = [0.; 3];
                let mut total = 0.;
                for (offset, weight) in [(-1, 0.25), (0, 0.5), (1, 0.25)] {
                    let mut at = xy;
                    at[axis] = (i64::from(xy[axis]) + offset * i64::from(radii[axis]))
                        .clamp(0, i64::from(size[axis] - 1)) as u32;
                    let p = input[(at[1] * size[0] + at[0]) as usize];
                    if p[3] < 0. {
                        return p;
                    }
                    if coverage != 0 && p[3] as u32 & 8 == 0 {
                        continue;
                    }
                    for c in 0..3 {
                        rgb[c] += weight * p[c];
                    }
                    total += weight;
                }
                [rgb[0] / total, rgb[1] / total, rgb[2] / total, center[3]]
            })
            .collect();
    }
    values
}
fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    context: &Context,
    image: Arc<ImageNode>,
    region: Rect,
    tile: [u32; 2],
) -> Vec<[f32; 4]> {
    let grid = image.domain.grid;
    let evaluation = context
        .request(
            image,
            Region {
                bounds: region,
                grid,
            },
            Budget {
                tile,
                scratch_bytes: 1 << 20,
            },
        )
        .unwrap();
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(region.width) * u64::from(region.height) * 16,
        usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    context
        .evaluate(
            queue,
            &Default::default(),
            &evaluation,
            || false,
            |encoder, image, pixels| {
                for row in 0..image.region.height {
                    let offset = u64::from(image.region.y - region.y + row)
                        * u64::from(region.width)
                        + u64::from(image.region.x - region.x);
                    encoder.copy_buffer_to_buffer(
                        pixels,
                        u64::from(row * image.region.width) * 16,
                        &output,
                        offset * 16,
                        u64::from(image.region.width) * 16,
                    );
                }
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
    execute::read(device, queue, &output)
}
fn assert_pixels(actual: &[[f32; 4]], expected: &[[f64; 4]]) {
    assert_eq!(actual.len(), expected.len());
    for (i, (a, e)) in actual.iter().zip(expected).enumerate() {
        assert!(a.iter().all(|v| v.is_finite()));
        assert_eq!(f64::from(a[3]), e[3], "metadata at {i}");
        for c in 0..3 {
            assert!(
                (f64::from(a[c]) - e[c]).abs() < 2e-5,
                "pixel {i} channel {c}: {a:?} vs {e:?}"
            );
        }
    }
}
#[test]
fn pointwise_definitions_capture_cpu_uniforms_without_shader_specialization_or_a_device() {
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let data = pattern(1);
    let root = source(Arc::new(data.clone()));
    let first = SignedCurve::new(1.2, 0.15)
        .unwrap()
        .compose(root.clone())
        .unwrap();
    let changed = SignedCurve::new(0.7, 0.4).unwrap().compose(root).unwrap();
    assert_eq!(
        data.reads.load(Ordering::Relaxed),
        0,
        "assembly must not acquire pixels"
    );
    assert_eq!(context.programs.len(), 0);
    let expected = curve(&data.pixels, 1, 1.2, 0.15);
    let actual = render(
        &device,
        &queue,
        &context,
        first.clone(),
        data.domain.bounds,
        [5, 4],
    );
    assert_pixels(&actual, &expected);
    let compiled = context.programs.len();
    let other = render(
        &device,
        &queue,
        &context,
        changed,
        data.domain.bounds,
        [13, 11],
    );
    assert_pixels(&other, &curve(&data.pixels, 1, 0.7, 0.4));
    assert!(
        actual
            .iter()
            .zip(&other)
            .any(|(a, b)| (a[0] - b[0]).abs() > 0.05),
        "parameters must change pixels"
    );
    assert_eq!(
        context.programs.len(),
        compiled,
        "numeric edits must reuse the same shader"
    );
    assert_eq!(
        actual,
        render(
            &device,
            &queue,
            &context,
            first,
            data.domain.bounds,
            [13, 11]
        ),
        "captured old values survive another definition"
    );
}
#[test]
fn a_convolution_definition_owns_two_passes_and_halos_across_tiled_rotated_grid_requests() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    for coverage in [0, 1] {
        let data = pattern(coverage);
        let size = [13, 11];
        let pipeline = PixelPipeline {
            stages: vec![
                PixelStage::new(SignedCurve::new(1.2, 0.15).unwrap()),
                PixelStage::new(TentConvolution::new(2.2).unwrap()),
            ],
        };
        assert_eq!(
            pipeline.support(data.domain.grid.pitch).unwrap(),
            crate::processing_support::Support([2, 1])
        );
        let image = pipeline.compose(source(Arc::new(data.clone()))).unwrap();
        let expected = tent(
            &curve(&data.pixels, coverage, 1.2, 0.15),
            size,
            [2, 1],
            coverage,
        );
        let before = tent(
            &data
                .pixels
                .iter()
                .map(|p| p.map(f64::from))
                .collect::<Vec<_>>(),
            size,
            [2, 1],
            coverage,
        );
        assert!(
            expected
                .iter()
                .zip(before)
                .any(|(a, b)| (a[0] - b[0]).abs() > 0.05),
            "omission must be observable"
        );
        let wrong_order = tent(
            &data
                .pixels
                .iter()
                .map(|p| p.map(f64::from))
                .collect::<Vec<_>>(),
            size,
            [2, 1],
            coverage,
        )
        .iter()
        .map(|p| {
            let x = p[0];
            x / (1. + 0.15 * x.abs()) * 1.2
        })
        .collect::<Vec<_>>();
        assert!(
            expected
                .iter()
                .zip(wrong_order)
                .any(|(a, b)| (a[0] - b).abs() > 0.01),
            "the fixture must distinguish operation order"
        );
        let whole = render(
            &device,
            &queue,
            renderer.evaluation(),
            image.clone(),
            data.domain.bounds,
            size,
        );
        assert_pixels(&whole, &expected);
        let off = Context::new(&device, CachePolicy { generated_bytes: 0 });
        let tiled = render(
            &device,
            &queue,
            &off,
            image.clone(),
            data.domain.bounds,
            [5, 4],
        );
        assert_eq!(
            whole, tiled,
            "working tiles and optional retention cannot change results"
        );
        let crop: Rect = [13, 19, 8, 6].into();
        let expected_crop = (crop.y..crop.y + crop.height)
            .flat_map(|y| (crop.x..crop.x + crop.width).map(move |x| [x, y]))
            .map(|[x, y]| {
                expected[((y - data.domain.bounds.y) * 13 + x - data.domain.bounds.x) as usize]
            })
            .collect::<Vec<_>>();
        assert_pixels(
            &render(&device, &queue, &off, image, crop, [3, 2]),
            &expected_crop,
        );
        assert!(
            expected
                .iter()
                .filter(|p| coverage == 0 || p[3] as u32 & 8 != 0)
                .any(|p| p[..3].iter().any(|x| *x < 0.))
        );
        assert!(
            expected
                .iter()
                .filter(|p| coverage == 0 || p[3] as u32 & 8 != 0)
                .any(|p| p[..3].iter().any(|x| *x > 1.))
        );
    }
}
#[test]
fn uniform_fusion_requires_identical_captured_values_and_rejects_invalid_authoring_inputs() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let data = pattern(0);
    let domain = data.domain;
    for (gain, curvature) in [(1.2f32, 0.15f32), (0.7, 0.4)] {
        let second = SignedCurve::new(gain, curvature).unwrap();
        let first = SignedCurve::new(1.2, 0.15).unwrap();
        let pipeline = PixelPipeline {
            stages: vec![PixelStage::new(first), PixelStage::new(second)],
        };
        let image = pipeline.compose(source(Arc::new(data.clone()))).unwrap();
        let plan = Plan::new(
            image.clone(),
            domain.bounds,
            Budget {
                tile: [13, 11],
                scratch_bytes: 1 << 20,
            },
        )
        .unwrap();
        let fragment = &plan.tiles[0].fragment;
        let compute = fragment
            .work
            .iter()
            .filter(|w| matches!(w, Work::Compute { .. }))
            .count();
        assert_eq!(
            compute,
            if first == second { 1 } else { 2 },
            "conflicting captures need separate uniforms/passes"
        );
        assert_eq!(
            fragment
                .resources
                .iter()
                .filter(|r| matches!(r, Storage::Uniform(_)))
                .count(),
            compute * 2,
            "one parameter block and grid mapping per pass"
        );
        let expected = curve(&data.pixels, 0, 1.2, 0.15)
            .iter()
            .map(|p| {
                let mapped =
                    |c: usize| p[c] / (1. + f64::from(curvature) * p[c].abs()) * f64::from(gain);
                [mapped(0), mapped(1), mapped(2), p[3]]
            })
            .collect::<Vec<_>>();
        assert_pixels(
            &render(
                &device,
                &queue,
                renderer.evaluation(),
                image,
                domain.bounds,
                [13, 11],
            ),
            &expected,
        );
    }
    let untouched = pattern(0);
    let oversized=pointwise(source(Arc::new(untouched.clone())),Meaning::Graded,Function::new(
        "@group(0) @binding(18) var<uniform> values:array<vec4f,5>; fn choose(p:vec4f,pos:vec2f)->vec4f{return vec4f(p.rgb+values[0].rgb,p.w);}","choose"
    ).with_uniform(18,&[0f32;20]));
    let plan = Plan::new(
        oversized,
        domain.bounds,
        Budget {
            tile: [13, 11],
            scratch_bytes: 1 << 20,
        },
    )
    .unwrap();
    let mut limits = device.limits();
    limits.max_uniform_buffer_binding_size = 64;
    assert!(
        Executor::admit(&limits, &plan).is_err(),
        "captured uniform bytes must participate in admission"
    );
    assert_eq!(untouched.reads.load(Ordering::Relaxed), 0);
    assert!(SignedCurve::new(f32::NAN, 0.1).is_err());
    assert!(SignedCurve::new(1., -0.1).is_err());
    assert!(TentConvolution::new(f64::INFINITY).is_err());
    assert!(TentConvolution::new(0.).is_err());
    let mut bad = data.clone();
    bad.domain.meaning = Meaning::CameraCodes;
    assert!(
        SignedCurve::new(1., 0.)
            .unwrap()
            .compose(source(Arc::new(bad)))
            .is_err()
    );
    let invalid = pointwise(
        source(Arc::new(data)),
        Meaning::Graded,
        Function::new(
            "fn identity(p:vec4f,pos:vec2f)->vec4f{return p;}",
            "identity",
        )
        .with_uniform(20, &[1f32; 4]),
    );
    assert!(
        Plan::new(
            invalid,
            domain.bounds,
            Budget {
                tile: [13, 11],
                scratch_bytes: 1 << 20
            }
        )
        .is_err()
    );
}
