use super::*;

struct Constant(f32);
impl Source for Constant {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some(CellRegion::new([0, 0], [4, 2])),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::Neutral,
        }
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new((
            "constant definition witness",
            self.0.to_bits(),
        )))
    }
    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        Ok(vec![
            [self.0, self.0, self.0, 0.];
            (region.width * region.height) as usize
        ])
    }
}
#[test]
fn an_operand_view_cannot_hide_mathematics_needed_by_a_later_demand() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let base = source(Arc::new(Constant(0.1)));
    let initial = CellRegion::new([0, 0], [1, 2]);
    let later = CellRegion::new([2, 0], [1, 2]);
    let capture = |replacement: f32| {
        let image = crate::gpu::repairs::definition::compose_images(
            base.clone(),
            vec![(
                crate::dust_repair::SupportMask::new([2, 0], [1, 2], vec![true; 2]).unwrap(),
                source(Arc::new(Constant(replacement))),
            )],
        )
        .unwrap();
        let view = image.request(initial).unwrap();
        assert_eq!(
            view.identity(),
            base.identity(),
            "the initial view intentionally excludes the replacement"
        );
        Value::new(
            image.domain(),
            initial,
            vec![view],
            Operation::new(
                "partial operand capture witness",
                (),
                "fn pixel(p:vec2i, position:vec2f, a:vec4f)->vec4f{return a;}",
                "pixel",
                Access::Pointwise,
            ),
        )
        .unwrap()
    };
    let first = capture(10.);
    let second = capture(20.);
    for capacity in [0, 1 << 20] {
        let context = Context::new(
            &device,
            CachePolicy {
                generated_bytes: capacity,
            },
        );
        let scope = context.materializations();
        let gate = crate::gpu::submission::SubmissionGate::default();
        let acquire = |definition: &Value| {
            let demand = definition.request(later).unwrap();
            assert!(definition.same_capture(&demand));
            let query = context
                .request(
                    demand,
                    Region {
                        bounds: later,
                        grid: definition.domain().grid,
                    },
                    budget(later),
                )
                .unwrap();
            pollster::block_on(
                context
                    .materialize(
                        &queue,
                        &gate,
                        &scope,
                        query,
                        ImageFormat::Rgba32Float,
                        ImageBacking::Buffer,
                    )
                    .unwrap(),
            )
            .unwrap()
        };
        let held = acquire(&first);
        assert!(
            scope
                .ready::<ImageLayout>(first.request(later).unwrap().identity().unwrap())
                .iter()
                .any(|image| Arc::ptr_eq(&image.image, &held.image)),
            "the contrasting wrapper itself must be discoverable"
        );
        let repeated = acquire(&first);
        assert!(
            Arc::ptr_eq(&held.image, &repeated.image),
            "the positive control must actually reuse the held mathematical result"
        );
        let changed = acquire(&second);
        for (image, expected) in [(&held, 10.), (&changed, 20.)] {
            assert_eq!(image.descriptor.region, later);
            let actual =
                super::super::super::execute::read(&device, &queue, image.as_buffer().unwrap());
            assert_eq!(actual, [[expected, expected, expected, 0.]; 2]);
        }
    }
}
