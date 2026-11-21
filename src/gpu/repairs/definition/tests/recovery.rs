use super::*;

#[test]
fn healing_does_not_turn_an_upstream_definition_error_into_a_failed_repair() {
    struct Unavailable;
    impl Operation for Unavailable {
        fn describe(
            &self,
            _: Output,
            _: &[value::Value],
            _: &[CellRegion],
        ) -> Result<Output, Error> {
            Err(Error::InvalidParameter("upstream definition unavailable"))
        }
    }
    let original = base(Arc::new(Mutex::new(Vec::new())));
    let unavailable = Value::from_operation(original.domain(), [original], Unavailable);
    assert!(matches!(
        healing_image(&unavailable, &repair([32, 32], [10, 0])),
        Err(Error::InvalidParameter("upstream definition unavailable"))
    ));
}

#[test]
fn healing_does_not_swallow_underlying_source_read_errors() {
    struct Unavailable;
    impl Source for Unavailable {
        fn domain(&self) -> Domain {
            Domain {
                bounds: Some([0, 0, 96, 64].into()),
                grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            }
        }
        fn read(&self, _: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
            Err(Error::InvalidParameter("underlying source unavailable"))
        }
    }
    let (_, queue, renderer) = crate::gpu::test_support::render_context();
    let image = compose(source(Arc::new(Unavailable)), &[repair([32, 32], [10, 0])]).unwrap();
    let error = pixels(renderer.evaluation(), &queue, image, [32, 32, 1, 1].into()).unwrap_err();
    assert!(
        error.to_string().contains("underlying source unavailable"),
        "{error}"
    );
}
