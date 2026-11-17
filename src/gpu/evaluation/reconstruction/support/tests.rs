use super::*;

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

fn reconstruction(bounds: Option<Rect>) -> Reconstruction {
    let mapping = Mapping {
        start: [0.; 2],
        axes: [[1., 0.], [0., 1.]],
    };
    Reconstruction {
        tiled: true,
        input: Domain {
            bounds,
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::Neutral,
        },
        mapping,
        packed: mapping,
        coverage: 8,
        support: None,
    }
}

#[test]
fn covering_slices_keep_the_same_input_lattice_across_moving_demands() {
    let reconstruction = reconstruction(Some([17, 23, 512, 384].into()));
    let first: Rect = [180, 125, 34, 18].into();
    let moved: Rect = [181, 126, 34, 18].into();
    let expected: Rect = [145, 119, 128, 32].into();
    assert_eq!(
        reconstruction.covering_slices(first, [128, 32]).unwrap(),
        expected
    );
    assert_eq!(
        reconstruction.covering_slices(moved, [128, 32]).unwrap(),
        expected
    );
    let crossing: Rect = [270, 125, 34, 18].into();
    let slices = reconstruction
        .input_slices(crossing, [128, 32])
        .unwrap()
        .collect::<Vec<_>>();
    assert_eq!(slices, [expected, [273, 119, 128, 32].into()]);
    assert!(
        slices
            .iter()
            .all(|slice| slice.width * slice.height == 128 * 32)
    );
}

#[test]
fn slices_clip_only_at_true_input_edges_and_support_negative_unbounded_cells() {
    let bounds: Rect = [17, 23, 301, 157].into();
    let reconstruction = reconstruction(Some(bounds));
    let demand: Rect = [310, 170, 8, 10].into();
    let covered = reconstruction.covering_slices(demand, [128, 32]).unwrap();
    assert_eq!(covered, [273, 151, 45, 29].into());
    assert!(bounds.contains_region(covered));
    assert!(covered.contains_region(demand));
    let unbounded = self::reconstruction(None);
    assert_eq!(
        unbounded
            .covering_slices([-7, -3, 10, 8].into(), [8, 4])
            .unwrap(),
        [-8, -4, 16, 12].into()
    );
    let edge = unbounded
        .covering_slices([i32::MAX - 3, i32::MAX - 2, 3, 2].into(), [8, 4])
        .unwrap();
    assert_eq!(edge.right(), i32::MAX);
    assert_eq!(edge.bottom(), i32::MAX);
}
