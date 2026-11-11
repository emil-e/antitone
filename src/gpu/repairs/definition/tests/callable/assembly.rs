use super::*;

struct Background;
impl Source for Background {
    fn domain(&self) -> Domain {
        Replacement(0).domain()
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("repair index background"))
    }
    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        Ok(vec![
            [2., -0.5, 0.25, 0.75];
            region.width as usize * region.height as usize
        ])
    }
}

#[test]
fn packed_replacements_cover_bin_edges_collisions_and_overlapping_unselected_envelopes() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let points = [[63, 24], [65, 24], [256, 24], [512, 65], [768, 65]];
    let colors = [
        [-1., 2., 0.125, 0.],
        [3., -0.25, 0.5, 0.],
        [-2., 4., -1., 0.],
        [5., -0.5, 2., 0.],
        [0.25, -3., 6., 0.],
    ];
    let replacements = points
        .into_iter()
        .zip(colors)
        .enumerate()
        .map(|(index, (point, color))| {
            let input = source(Arc::new(Replacement(index as u32)));
            let image = Value::from_operation(input.domain(), [input], Tint(color));
            (repair(point, [0, 0]).support, image)
        })
        .collect();
    let root = compose_images(source(Arc::new(Background)), replacements).unwrap();
    let region: CellRegion = [60, 22, 712, 46].into();
    let query = context
        .request(
            root.clone(),
            Region {
                bounds: region,
                grid: root.domain().grid,
            },
            Budget {
                tile: [712, 46],
                scratch_bytes: 4 << 20,
            },
        )
        .unwrap();
    let mut actual = vec![[0.; 4]; region.width as usize * region.height as usize];
    context
        .read(
            &queue,
            &Default::default(),
            &query,
            || false,
            |block, pixels| {
                for row in 0..block.height {
                    let from = row as usize * block.width as usize;
                    let to = region.index([block.x, block.y + row as i32]);
                    actual[to..to + block.width as usize]
                        .copy_from_slice(&pixels[from..from + block.width as usize]);
                }
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
    for y in region.y..region.bottom() {
        for x in region.x..region.right() {
            let mut expected = [2., -0.5, 0.25, 0.75];
            if let Some(index) = points.iter().position(|p| *p == [x as u32, y as u32]) {
                expected[..3].copy_from_slice(&colors[index][..3]);
            }
            assert_eq!(
                actual[region.index([x, y])],
                expected,
                "all selected cells, false mask holes and base metadata must survive spatial argument lookup"
            );
        }
    }
}

struct HostValue(Arc<AtomicUsize>);
impl Operation for HostValue {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("top-level host publication"))
    }
    fn calculation(
        self: Arc<Self>,
        _inputs: &[crate::gpu::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _: Budget,
    ) -> Result<Option<crate::gpu::evaluation::value::Calculation>, Error> {
        Ok(Some(value::Calculation::new(
            domain,
            output,
            Vec::new(),
            value::Operation::cpu("top-level host publication", self.key(), self),
        )?))
    }
}
impl CpuKernel for HostValue {
    fn run(
        &self,
        _: &[CpuImage],
        output: CellRegion,
        _: &dyn Fn() -> bool,
    ) -> Result<Option<Vec<[f32; 4]>>, Error> {
        assert_eq!(
            self.0.load(Ordering::Relaxed),
            0,
            "the fixture has no earlier GPU work that could mask a missing upload submission"
        );
        Ok(Some(vec![
            [3., -4., 0.5, 0.25];
            output.width as usize * output.height as usize
        ]))
    }
}
#[test]
fn standalone_host_materialization_submits_before_publishing_a_gpu_value() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let counter = Arc::new(AtomicUsize::new(0));
    let observed = counter.clone();
    let gate = crate::gpu::submission::SubmissionGate::default();
    gate.observe_encoded_resources(Arc::new(move || {
        observed.fetch_add(1, Ordering::Relaxed);
    }));
    let domain = Domain {
        bounds: Some([0, 0, 2, 2].into()),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Neutral,
    };
    let root = Value::from_operation(domain, [], HostValue(counter.clone()));
    let query = context
        .request(
            root,
            Region {
                bounds: domain.finite().unwrap(),
                grid: domain.grid,
            },
            Budget {
                tile: [2, 2],
                scratch_bytes: 1 << 20,
            },
        )
        .unwrap();
    let image = pollster::block_on(
        context
            .materialize(
                &queue,
                &gate,
                &context.materializations(),
                query,
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    )
    .unwrap();
    assert!(image.as_buffer().is_some());
    assert!(
        counter.load(Ordering::Relaxed) > 0,
        "a standalone host upload has no parent submission to defer to"
    );
}
