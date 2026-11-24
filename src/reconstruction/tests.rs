use super::*;
mod regional;

#[test]
fn complete_crop_mask_matches_cell_clipping_across_full_and_partial_rows() {
    let samples: Rect = [5, 7, 107, 89].into();
    let source = source(Arc::new(Pixels {
        domain: Domain {
            bounds: Some([0, 0, 128, 112].into()),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        },
        values: Vec::new(),
    }));
    for (origin, axes) in [
        ([21., 25.], [[1., 0.], [0., 1.]]),
        ([21.2, 25.5], [[1., 0.015], [-0.015, 1.]]),
        ([91., 25.5], [[-1., 0.015], [0.015, 1.]]),
        ([58.5, 9.5], [[0.5, 0.5], [-0.5, 0.5]]),
        ([21.2, 25.5], [[1., 0.015], [-0.00015, 0.01]]),
    ] {
        let output: Rect = [0, 0, 70, 50].into();
        let root = reconstruct_complete(
            source.clone(),
            Domain {
                bounds: Some(output),
                grid: Grid {
                    origin,
                    axes,
                    pitch: axes.map(|[x, y]: [f64; 2]| x.hypot(y)),
                    coverage: 8,
                },
            },
        )
        .unwrap();
        let reconstruction = (root.operation().as_ref() as &dyn std::any::Any)
            .downcast_ref::<Reconstruction>()
            .unwrap();
        let mask = reconstruction.support_mask(output, samples).unwrap();
        let coverage = reconstruction.support.unwrap().coverage;
        for y in samples.y..samples.bottom() {
            for x in samples.x..samples.right() {
                let index = samples.index([x, y]);
                let marked = mask[index / 32] & (1 << (index % 32)) != 0;
                assert_eq!(
                    marked,
                    coverage.rectangle_weight([f64::from(x), f64::from(y)], [1.; 2]) > 0.,
                    "source cell ({x}, {y}) for {origin:?}, {axes:?}"
                );
            }
        }
    }
}

struct Pixels {
    domain: Domain,
    values: Vec<[f32; 4]>,
}
impl Source for Pixels {
    fn domain(&self) -> Domain {
        self.domain
    }
    fn read(&self, region: Rect) -> Result<Vec<[f32; 4]>, Error> {
        let b = self.domain.finite().unwrap();
        Ok((region.y..region.bottom())
            .flat_map(|y| (region.x..region.right()).map(move |x| self.values[b.index([x, y])]))
            .collect())
    }
}
fn evaluate(
    size: [u32; 2],
    values: Vec<[f32; 4]>,
    mapping: Mapping,
    output: [u32; 2],
    tile: [u32; 2],
) -> Vec<[f32; 4]> {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let input = Domain {
        bounds: Some(crate::gpu::evaluation::CellRegion::new([11, 17], size)),
        grid: Grid::axis_aligned([123.25, -81.5], [1.; 2]),
    };
    let mapping = Mapping {
        start: [mapping.start[0] + 11., mapping.start[1] + 17.],
        ..mapping
    };
    let domain = Domain {
        bounds: Some(crate::gpu::evaluation::CellRegion::new([0, 0], output)),
        grid: Grid {
            origin: input.grid.point(mapping.start),
            axes: mapping.axes,
            pitch: mapping.axes.map(|a| a[0].hypot(a[1])),
            coverage: 8,
        },
    };
    let root = reconstruct(
        source(Arc::new(Pixels {
            domain: input,
            values,
        })),
        domain,
    )
    .unwrap();
    let plan = make_plan(root, tile);
    let result = Executor::default()
        .run(&device, &queue, &plan, || false)
        .unwrap()
        .unwrap();
    crate::gpu::evaluation::read_buffer(&device, &queue, &result.pixels)
}

#[test]
fn signed_grid_origins_preserve_sample_addressing_and_mean_placement_across_tiles() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let input = Domain {
        bounds: Some([-4, -2, 4, 2].into()),
        grid: Grid {
            coverage: 8,
            ..Grid::axis_aligned([0.; 2], [1.; 2])
        },
    };
    let values = (0..8)
        .map(|i| {
            let value = (2 * i + 1) as f32;
            [value, value + 1., value + 2., 8.]
        })
        .collect();
    let root = reconstruct(
        source(Arc::new(Pixels {
            domain: input,
            values,
        })),
        Domain {
            bounds: Some([-2, -1, 2, 1].into()),
            grid: Grid {
                coverage: 8,
                ..Grid::axis_aligned([0.; 2], [2.; 2])
            },
        },
    )
    .unwrap();
    for tile in [[2, 1], [1, 1]] {
        let plan = make_plan(root.clone(), tile);
        let result = Executor::default()
            .run(&device, &queue, &plan, || false)
            .unwrap()
            .unwrap();
        assert_eq!(
            crate::gpu::evaluation::read_buffer(&device, &queue, &result.pixels),
            vec![[6., 7., 8., 8.], [10., 11., 12., 8.]]
        );
    }
}

#[test]
fn affine_reconstruction_integrates_a_diamond_without_using_its_bounding_rectangle() {
    let mut values = vec![[0., 0., 0., 4.]; 9];
    for i in [1, 3, 5, 7] {
        values[i][3] = 2.;
    }
    values[4] = [2., -4., 8., 1.];
    let actual = evaluate(
        [3, 3],
        values,
        Mapping {
            start: [1.5, 0.5],
            axes: [[1., 1.], [-1., 1.]],
        },
        [1, 1],
        [1, 1],
    );
    assert_eq!(
        actual[0][3], 11.,
        "corner cells touch the diamond but contribute no area"
    );
    for (a, b) in actual[0][..3].iter().zip([1., -2., 4.]) {
        assert!((*a - b).abs() < 2e-6);
    }
}

#[test]
fn affine_reconstruction_preserves_triangular_shear_weights_in_both_directions() {
    let values = vec![[2., -1., 0.5, 1.], [-1., 4., 2., 2.]];
    for shear in [0.25, 1. / 4096.] {
        for transpose in [false, true] {
            let (size, axes) = if transpose {
                ([1, 2], [[1., shear], [0., 1.]])
            } else {
                ([2, 1], [[1., 0.], [shear, 1.]])
            };
            let actual = evaluate(
                size,
                values.clone(),
                Mapping {
                    start: [0., 0.],
                    axes,
                },
                [1, 1],
                [1, 1],
            );
            let weight = shear / 2.;
            for c in 0..3 {
                let expected =
                    f64::from(values[0][c]) * (1. - weight) + f64::from(values[1][c]) * weight;
                assert!(
                    (f64::from(actual[0][c]) - expected).abs() < 2e-6,
                    "shear={shear} transpose={transpose} actual={actual:?}"
                );
            }
            assert_eq!(actual[0][3], 11.);
        }
    }
}

#[test]
fn reflected_fractional_cells_extend_true_edges_and_are_independent_of_execution_tiles() {
    let values = vec![[1., -2., 4., 1.]; 12];
    let mapping = Mapping {
        start: [4.4, -0.25],
        axes: [[-1.6, 0.], [0., 1.9]],
    };
    let whole = evaluate([4, 3], values.clone(), mapping, [3, 2], [3, 2]);
    let tiled = evaluate([4, 3], values, mapping, [3, 2], [1, 1]);
    assert_eq!(whole, tiled);
    for p in whole {
        assert_eq!(p, [1., -2., 4., 9.]);
    }
}
