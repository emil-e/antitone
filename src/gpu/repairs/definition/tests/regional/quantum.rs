use super::*;

struct Damage;
impl Operation for Damage {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("coarse regional dark control"))
    }
    fn reads(&self, region: CellRegion) -> Result<Vec<Read>, Error> {
        Ok(vec![Read::new(0, region)])
    }
    fn calculation(
        self: std::sync::Arc<Self>,
        inputs: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _budget: crate::gpu::evaluation::Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, crate::Error> {
        let inputs = crate::gpu::evaluation::value::Calculation::references(inputs);
        let function = Function::new(
            "fn damage(p:vec4f,at:vec2f)->vec4f{if all(vec2i(floor(at))==vec2i(41,41)){return vec4f(0.,0.,0.,p.w);}return p;}",
            "damage",
        );
        value::Calculation::new(
            domain,
            output,
            vec![inputs[0].request(output)?],
            value::Operation::shader_function(self.key(), function),
        )
        .map(Some)
    }
}
#[test]
fn default_regional_quantum_preserves_the_complete_healed_ramp_and_real_edge_extension() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let original = source(Arc::new(Plate));
    let damaged = Value::from_operation(original.domain(), [original], Damage);
    let mut control = None;
    context
        .read(
            &queue,
            &Default::default(),
            &context
                .request(
                    damaged.clone(),
                    Region {
                        bounds: [41, 41, 1, 1].into(),
                        grid: damaged.domain().grid,
                    },
                    Budget {
                        tile: [1, 1],
                        scratch_bytes: 1 << 20,
                    },
                )
                .unwrap(),
            || false,
            |_, values| {
                control = Some(values[0]);
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        control.unwrap()[..3],
        [0.; 3],
        "the fixture must contain a real damaged target"
    );
    let edge = 256;
    let region = CellRegion::new([0, 0], [3 * edge, edge]);
    let domain = Domain {
        bounds: Some(region),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Neutral,
    };
    let root = reconstruct(compose(damaged, &[repair(40, 32)]).unwrap(), domain).unwrap();
    let budget = Budget {
        tile: [region.width, region.height],
        scratch_bytes: u64::from(edge) * u64::from(edge) * 128,
    };
    let units = root
        .operation()
        .work_units(region, budget)
        .unwrap()
        .collect::<Vec<_>>();
    assert!(
        units.len() > 1 && units.iter().any(|unit| unit.width == edge),
        "exercise the default production quantum across multiple units, not only the small-budget fallback"
    );
    let mut pixels = vec![[0.; 4]; region.width as usize * region.height as usize];
    context
        .read(
            &queue,
            &Default::default(),
            &context
                .request(
                    root,
                    Region {
                        bounds: region,
                        grid: domain.grid,
                    },
                    budget,
                )
                .unwrap(),
            || false,
            |block, values| {
                for row in 0..block.height {
                    let from = row as usize * block.width as usize;
                    let to = region.index([block.x, block.y + row as i32]);
                    pixels[to..to + block.width as usize]
                        .copy_from_slice(&values[from..from + block.width as usize]);
                }
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
    for y in 0..region.height {
        for x in 0..region.width {
            let expected = [
                1. + x.min(511) as f32 / 512.,
                -0.5 - y.min(255) as f32 / 256.,
                0.25,
                0.,
            ];
            let actual = pixels[y as usize * region.width as usize + x as usize];
            for (a, b) in actual.into_iter().zip(expected) {
                assert!(
                    (a - b).abs() < 2e-6,
                    "global ramp/metadata or source-edge extension changed at {x},{y}: {actual:?} versus {expected:?}"
                );
            }
        }
    }
}
