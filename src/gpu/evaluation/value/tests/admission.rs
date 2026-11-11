use super::*;

#[test]
fn adaptive_requests_preserve_rotated_masked_gaussian_math_and_complete_coverage() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let gate = crate::gpu::submission::SubmissionGate::default();
    let mut plate = Plate::new();
    let [sin, cos] = [0.31f64.sin(), 0.31f64.cos()];
    let [px, py] = plate.domain.grid.pitch;
    plate.domain.grid.axes = [[cos * px, sin * px], [-sin * py, cos * py]];
    plate.domain.grid.coverage = 1;
    let area = plate.domain.finite().unwrap();
    let sigma = 0.65;
    let gain = 1.75;
    let offset = -0.5;
    let node = gaussian(affine(source(Arc::new(plate.clone())), gain, offset), sigma).unwrap();
    let allowance = Budget {
        tile: [area.width, area.height],
        scratch_bytes: 8 << 10,
    };
    assert!(
        matches!(
            node.plan_unit(area, allowance)
                .and_then(|unit| unit.admit(allowance)),
            Err(Error::WorkingAllowance(_))
        ),
        "fixture must actually require adaptive subdivision"
    );
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let evaluation = context
        .request(
            node,
            Region {
                bounds: area,
                grid: plate.domain.grid,
            },
            allowance,
        )
        .unwrap();
    let mut actual = vec![[f32::NAN; 4]; (area.width * area.height) as usize];
    let mut visited = vec![false; actual.len()];
    context
        .read(
            &queue,
            &gate,
            &evaluation,
            || false,
            |region, values| {
                assert_eq!(values.len(), (region.width * region.height) as usize);
                for (i, value) in values.iter().enumerate() {
                    let p = [
                        region.x + (i % region.width as usize) as i32,
                        region.y + (i / region.width as usize) as i32,
                    ];
                    let at = area.index(p);
                    assert!(!visited[at]);
                    visited[at] = true;
                    actual[at] = *value;
                }
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
    assert!(
        plate.reads.load(Ordering::Relaxed) > 1,
        "one oversized source acquisition must not bypass adaptive admission"
    );
    assert!(
        visited.into_iter().all(|value| value),
        "every output cell must be delivered exactly once"
    );
    let mut expected = (area.y..area.bottom())
        .flat_map(|y| (area.x..area.right()).map(move |x| [x, y]))
        .map(|p| {
            let pixel = plate.pixel(p);
            let world = plate.domain.point([p[0] as f64 + 0.5, p[1] as f64 + 0.5]);
            [
                f64::from(pixel[0]) * gain + offset + world[0] * 0.01,
                f64::from(pixel[1]) * gain + offset + world[0] * 0.01,
                f64::from(pixel[2]) * gain + offset + world[0] * 0.01,
                f64::from(pixel[3]),
            ]
        })
        .collect::<Vec<_>>();
    for axis in 0..2 {
        let before = expected.clone();
        let s = sigma / plate.domain.grid.pitch[axis];
        let radius = (4. * s).ceil() as i32;
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                let mut sum = [0f64; 3];
                let mut mass = 0.;
                for delta in -radius..=radius {
                    let mut q = [x, y];
                    q[axis] += delta;
                    q[0] = q[0].clamp(area.x, area.right() - 1);
                    q[1] = q[1].clamp(area.y, area.bottom() - 1);
                    let input = before[area.index(q)];
                    if input[3] as u32 & 1 == 0 {
                        continue;
                    }
                    let weight = (-0.5 * (f64::from(delta) / s).powi(2)).exp();
                    mass += weight;
                    for c in 0..3 {
                        sum[c] += weight * input[c];
                    }
                }
                if mass > 0. {
                    for c in 0..3 {
                        expected[area.index([x, y])][c] = sum[c] / mass;
                    }
                }
            }
        }
    }
    assert_eq!(actual.len(), expected.len());
    let mut changed = false;
    for (i, (actual, expected)) in actual.iter().zip(&expected).enumerate() {
        for c in 0..3 {
            assert!(
                (f64::from(actual[c]) - expected[c]).abs() < 3e-5,
                "cell {i}, channel {c}: {actual:?}, expected {expected:?}"
            );
        }
        let p = [
            area.x + (i % area.width as usize) as i32,
            area.y + (i / area.width as usize) as i32,
        ];
        let original = plate.pixel(p);
        assert_eq!(actual[3].to_bits(), original[3].to_bits());
        let world = plate.domain.point([p[0] as f64 + 0.5, p[1] as f64 + 0.5]);
        changed |=
            (expected[2] - (f64::from(original[2]) * gain + offset + world[0] * 0.01)).abs() > 0.1;
    }
    assert!(
        changed,
        "the fixture must distinguish Gaussian processing from pass-through"
    );
}
