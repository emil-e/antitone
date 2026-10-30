use super::*;
use crate::geometry::SourceRegion as Rect;

struct Chart {
    domain: Domain,
    marker: f32,
}

impl Source for Chart {
    fn domain(&self) -> Domain {
        self.domain
    }
    fn read(&self, r: Rect) -> Result<Vec<[f32; 4]>, Error> {
        assert!(self.domain.bounds.contains_region(r));
        Ok((r.y..r.y + r.height)
            .flat_map(|y| {
                (r.x..r.x + r.width).map(move |x| {
                    let sign = if x % 2 == 0 { 1. } else { -1. };
                    [
                        sign * 3e38,
                        -2e38,
                        6e37,
                        if y == 1 {
                            self.marker
                        } else {
                            8. + (1u32 << (x % 3)) as f32
                        },
                    ]
                })
            })
            .collect())
    }
}

#[test]
fn rotated_cell_areas_preserve_constant_mass_at_large_coordinates_and_true_source_edges() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let domain = Domain {
        bounds: [0, 0, 1, 1].into(),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Graded,
    };
    for origin in [0., 1024., 8192.] {
        let code = format!(
            r#"
{}
fn area_control(p: vec4f, xy: vec2f) -> vec4f {{
    let origin = vec2f({origin}, {origin});
    let dx = vec2f(1.1654092, 0.4520193);
    let dy = vec2f(-0.2892923, 0.7458619);
    let start = origin + vec2f(1.25);
    var mass = 0.0;
    for(var y=0u; y<4u; y++) {{ for(var x=0u; x<4u; x++) {{
        mass += overlap(start, origin + vec2f(f32(x), f32(y)), dx, dy);
    }} }}
    let edge = extended_overlap(origin - vec2f(0.3,0.2), origin, dx, dy, origin, origin+vec2f(1.0));
    return vec4f(mass, edge, 0.0, 1.0);
}}
"#,
            include_str!("../../shaders/footprint.wgsl")
        );
        let root = pointwise(
            operators::source(Arc::new(Chart { domain, marker: 0. })),
            Meaning::Graded,
            Function::new(code, "area_control"),
        );
        let plan = super::tests::make_plan(root, [1, 1]);
        let result = Executor::default()
            .run(&device, &queue, &Default::default(), &plan, || false)
            .unwrap()
            .unwrap();
        let actual = execute::read(&device, &queue, &result.pixels)[0];
        // The area of the parallelogram is its determinant, independent of translation
        // or how source cells partition it. A sole source cell extends over the full plane.
        let area = 1.1654092_f64 * 0.7458619 - 0.4520193 * (-0.2892923);
        for (c, value) in actual.iter().enumerate().take(2) {
            assert!(
                (f64::from(*value) - area).abs() < 2e-6,
                "origin={origin} channel={c}: {} vs {area}",
                actual[c]
            );
        }
    }
}

#[test]
fn aligned_grid_reconstruction_preserves_signed_hdr_flags_and_failed_look_markers_across_tiles() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    for marker in [8., -1., -2.] {
        for angle in [0_f64, 0.37, std::f64::consts::FRAC_PI_2] {
            let domain = Domain {
                bounds: [0, 0, 12, 6].into(),
                grid: Grid {
                    origin: [123.25, -81.5],
                    axes: [
                        [0.3 * angle.cos(), 0.3 * angle.sin()],
                        [0.7 * angle.sin(), -0.7 * angle.cos()],
                    ],
                    pitch: [0.3, 0.7],
                    coverage: 1,
                },
                meaning: Meaning::Graded,
            };
            let mut output = domain;
            output.bounds = [0, 0, 4, 2].into();
            output.grid.origin = domain.grid.point([2., 0.]);
            output.grid.axes = domain.grid.axes.map(|v| v.map(|x| x * 2.));
            output.grid.pitch = domain.grid.pitch.map(|v| v * 2.);
            let root = reconstruct_cells(
                operators::source(Arc::new(Chart { domain, marker })),
                output,
                [2., 0.],
                [2.; 2],
            )
            .unwrap();
            let mut results = Vec::new();
            for tile in [[4, 2], [1, 1]] {
                let plan = super::tests::make_plan(root.clone(), tile);
                let result = Executor::default()
                    .run(&device, &queue, &Default::default(), &plan, || false)
                    .unwrap()
                    .unwrap();
                let pixels = execute::read(&device, &queue, &result.pixels);
                results.push(pixels);
            }
            assert_eq!(results[0], results[1]);
            for (i, p) in results[0].iter().enumerate() {
                assert!(p.iter().all(|v| v.is_finite()));
                assert!(
                    p[0].abs() < 1e32,
                    "opposite finite HDR contributions must cancel without overflow"
                );
                assert!((p[1] / -2e38 - 1.).abs() < 1e-6);
                assert!((p[2] / 6e37 - 1.).abs() < 1e-6);
                let flags = if i < 4 && marker < 0. {
                    marker
                } else {
                    let x = 2 + (i % 4) as u32 * 2;
                    (8 | (1 << (x % 3)) | (1 << ((x + 1) % 3))) as f32
                };
                assert_eq!(
                    p[3], flags,
                    "coverage/clipping union and any contributing look failure are categorical"
                );
            }
        }
    }
}
