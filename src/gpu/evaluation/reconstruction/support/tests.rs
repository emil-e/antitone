use super::row_span;

#[test]
fn axis_aligned_row_support_includes_area_but_not_boundary_contact() {
    let rectangle = [[12., 7.], [20., 7.], [20., 11.], [12., 11.]];
    for start in 0..4 {
        let corners: [[f64; 2]; 4] = std::array::from_fn(|i| rectangle[(start + i) % 4]);
        for polygon in [corners, [corners[3], corners[2], corners[1], corners[0]]] {
            assert_eq!(row_span(polygon, 8., 9.), Some([12., 20.]));
            assert_eq!(row_span(polygon, 6., 8.), Some([12., 20.]));
            assert_eq!(row_span(polygon, 6., 7.), None);
            assert_eq!(row_span(polygon, 11., 12.), None);
            assert_eq!(row_span(polygon, 9., 9.), None);
        }
    }
}

#[test]
fn oblique_row_support_uses_edge_intersections_and_interior_vertices() {
    // Side edges are x=y/2 and x=3.5+y/2 between y=1 and y=4.
    let skewed = [[0., 0.], [4., 1.], [6., 5.], [2., 4.]];
    assert_eq!(row_span(skewed, 1., 2.), Some([0.5, 4.5]));
    assert_eq!(row_span(skewed, 2., 3.), Some([1., 5.]));

    // Both horizontal clips intersect two edges; the result has six vertices.
    let diamond = [[0., 2.], [3., 0.], [6., 2.], [3., 4.]];
    assert_eq!(row_span(diamond, 1., 3.), Some([0., 6.]));
    assert_eq!(row_span(diamond, 0., 1.), Some([1.5, 4.5]));
    assert_eq!(row_span(diamond, 4., 5.), None);
}

#[test]
fn nearly_axis_aligned_support_still_clips_its_oblique_edges() {
    let epsilon = 1. / 65536.;
    let polygon = [
        [12., 7.],
        [20., 7. + epsilon],
        [20. - epsilon, 11. + epsilon],
        [12. - epsilon, 11.],
    ];
    let span = row_span(polygon, 8., 9.).unwrap();
    assert!(span[0] < 12.);
    assert!(span[1] < 20.);
}
