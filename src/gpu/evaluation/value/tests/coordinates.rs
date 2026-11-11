use super::*;

#[test]
fn pixel_world_coordinates_preserve_small_offsets_at_large_integer_cell_origins() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    for x in [-1_000_000_000, 1_000_000_000] {
        let area = CellRegion::new([x, 3], [7, 5]);
        let domain = Domain {
            bounds: Some(area),
            grid: Grid::axis_aligned([2. - f64::from(x) * 0.5, -1.], [0.5, 2.]),
            meaning: Meaning::Graded,
        };
        let value = Value::new(
            domain,
            area,
            vec![],
            Operation::new(
                "world-coordinate witness v1",
                (),
                "fn pixel(p: vec2i, position: vec2f) -> vec4f { return vec4f(position, 1.0, 0.5); }",
                "pixel",
                Access::Pointwise,
            ),
        )
        .unwrap();
        for policy in [Policy::Materialized, Policy::Composed] {
            let (pixels, _) = run(&device, &queue, &value, policy);
            assert_eq!(pixels.len(), (area.width * area.height) as usize);
            for (i, actual) in pixels.iter().enumerate() {
                let cell = [
                    f64::from(area.x) + (i % area.width as usize) as f64 + 0.5,
                    f64::from(area.y) + (i / area.width as usize) as f64 + 0.5,
                ];
                let expected = domain.point(cell);
                assert_eq!(actual[0], expected[0] as f32, "cell {cell:?}");
                assert_eq!(actual[1], expected[1] as f32, "cell {cell:?}");
                assert_eq!(&actual[2..], &[1., 0.5]);
            }
        }
    }
}
