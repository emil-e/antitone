use super::*;

struct Pixels {
    domain: Domain,
    values: Vec<[f32; 4]>,
}
impl Source for Pixels {
    fn domain(&self) -> Domain {
        self.domain
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(self.values.clone()))
    }
    fn read(&self, r: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        Ok((r.y..r.bottom())
            .flat_map(|y| {
                (r.x..r.right())
                    .map(move |x| self.values[self.domain.finite().unwrap().index([x, y])])
            })
            .collect())
    }
}
fn reconstructed(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    context: &Context,
    size: [u32; 2],
    values: &[[f32; 4]],
    affine: [[f32; 2]; 3],
    columns: u32,
) -> Vec<[f32; 4]> {
    let domain = Domain {
        bounds: Some(crate::gpu::evaluation::CellRegion::new([0, 0], size)),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Neutral,
    };
    let input = source(Arc::new(Pixels {
        domain,
        values: values.to_vec(),
    }));
    let scope = context.test_materializations(false);
    let gate = crate::gpu::SubmissionGate::default();
    let mut images = Vec::new();
    for x in (0..size[0]).step_by(columns as usize) {
        let region = [x, 0, columns.min(size[0] - x), size[1]]
            .try_into()
            .unwrap();
        let request = context
            .request(
                input.clone(),
                Region {
                    bounds: region,
                    grid: domain.grid,
                },
                Budget {
                    tile: size,
                    scratch_bytes: 1 << 20,
                },
            )
            .unwrap();
        images.push(
            pollster::block_on(
                context
                    .materialize(
                        queue,
                        &gate,
                        &scope,
                        request,
                        ImageFormat::Rgba32Float,
                        ImageBacking::Texture,
                    )
                    .unwrap(),
            )
            .unwrap(),
        );
    }
    let mapping = super::super::reconstruction::Mapping {
        start: affine[0].map(f64::from),
        axes: [affine[1].map(f64::from), affine[2].map(f64::from)],
    };
    let output = Domain {
        bounds: Some([0, 0, 3, 2].into()),
        grid: Grid {
            origin: mapping.start,
            axes: mapping.axes,
            pitch: mapping.axes.map(|a| a[0].hypot(a[1])),
            coverage: 8,
        },
        meaning: Meaning::Neutral,
    };
    let root = super::super::reconstruction::reconstruct_mapped(input, output, mapping).unwrap();
    let request = context
        .request(
            root,
            Region {
                bounds: output.finite().unwrap(),
                grid: output.grid,
            },
            Budget {
                tile: [3, 2],
                scratch_bytes: 1 << 20,
            },
        )
        .unwrap();
    let result = pollster::block_on(
        context
            .materialize(
                queue,
                &gate,
                &scope,
                request,
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    )
    .unwrap();
    let values = execute::read(device, queue, result.as_buffer().unwrap());
    drop(images);
    values
}

#[test]
fn rectangular_footprints_integrate_signed_light_and_extend_only_real_source_edges() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let size = [4, 3];
    let values: Vec<_> = (0..12)
        .map(|i| {
            [
                i as f32 * 0.25 - 1.,
                (i % 3) as f32 * 2.,
                (11 - i) as f32 * 0.125,
                (i % 8) as f32,
            ]
        })
        .collect();
    for affine in [
        [[-0.4, -0.25], [1.7, 0.], [0., 1.9]],
        [[4.4, 0.2], [-1.6, 0.], [0., 1.45]],
        [[0.25, 3.4], [0., -1.2], [2.2, 0.]],
    ] {
        for columns in [1, 4] {
            let actual = reconstructed(
                &device,
                &queue,
                renderer.evaluation(),
                size,
                &values,
                affine,
                columns,
            );
            for (i, actual) in actual.iter().enumerate() {
                let [x, y] = [(i % 3) as f64, (i / 3) as f64];
                let start: [f64; 2] = std::array::from_fn(|a| {
                    f64::from(affine[0][a])
                        + x * f64::from(affine[1][a])
                        + y * f64::from(affine[2][a])
                });
                let end: [f64; 2] = std::array::from_fn(|a| {
                    start[a] + f64::from(affine[1][a]) + f64::from(affine[2][a])
                });
                let low: [f64; 2] = std::array::from_fn(|a| start[a].min(end[a]));
                let high: [f64; 2] = std::array::from_fn(|a| start[a].max(end[a]));
                let area = (high[0] - low[0]) * (high[1] - low[1]);
                let mut expected = [0.; 3];
                let mut flags = 0u32;
                for cy in 0..size[1] {
                    for cx in 0..size[0] {
                        let coordinates = [cx, cy];
                        let lengths: [f64; 2] = std::array::from_fn(|a| {
                            let lower = if coordinates[a] == 0 {
                                f64::NEG_INFINITY
                            } else {
                                f64::from(coordinates[a])
                            };
                            let upper = if coordinates[a] + 1 == size[a] {
                                f64::INFINITY
                            } else {
                                f64::from(coordinates[a] + 1)
                            };
                            (high[a].min(upper) - low[a].max(lower)).max(0.)
                        });
                        let weight = lengths[0] * lengths[1] / area;
                        if weight > 0. {
                            let source = values[(cy * size[0] + cx) as usize];
                            for c in 0..3 {
                                expected[c] += f64::from(source[c]) * weight;
                            }
                            flags |= source[3] as u32 | 8;
                        }
                    }
                }
                for c in 0..3 {
                    assert!(
                        (f64::from(actual[c]) - expected[c]).abs() < 3e-6,
                        "affine={affine:?} columns={columns} pixel={i} actual={actual:?} expected={expected:?}"
                    );
                }
                assert_eq!(actual[3], flags as f32);
            }
        }
    }
}

#[test]
fn a_shear_with_one_zero_off_diagonal_is_not_an_axis_aligned_rectangle() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let values = [[2., -1., 0.5, 1.], [-1., 4., 2., 2.]];
    for shear in [0.25, 1. / 4096.] {
        for transpose in [false, true] {
            let (size, affine) = if transpose {
                ([1, 2], [[0., 0.], [1., shear], [0., 1.]])
            } else {
                ([2, 1], [[0., 0.], [1., 0.], [shear, 1.]])
            };
            let actual = reconstructed(
                &device,
                &queue,
                renderer.evaluation(),
                size,
                &values,
                affine,
                size[0],
            );
            // Integrating the sloping boundary gives a triangular area of shear/2 in cell 1.
            let w = f64::from(shear) / 2.;
            for c in 0..3 {
                let expected = f64::from(values[0][c]) * (1. - w) + f64::from(values[1][c]) * w;
                assert!(
                    (f64::from(actual[0][c]) - expected).abs() < 2e-6,
                    "shear={shear} transpose={transpose} actual={:?} expected={expected}",
                    actual[0]
                );
            }
            assert_eq!(actual[0][3], 11.);
        }
    }
}
