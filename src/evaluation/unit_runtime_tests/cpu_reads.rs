use super::*;

struct InputPlate(Arc<AtomicUsize>);
impl Source for InputPlate {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some([0, 0, 32, 32].into()),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        }
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("CPU input batching plate"))
    }
    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, crate::Error> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(vec![
            [1., -0.25, 0.125, 0.];
            region.width as usize * region.height as usize
        ])
    }
}

struct Mean;
impl Operation for Mean {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("whole-input CPU mean"))
    }
    fn calculation(
        self: Arc<Self>,
        inputs: &[crate::evaluation::value::Value],
        domain: Domain,
        output: CellRegion,
        _: Tiling,
    ) -> Result<Option<crate::evaluation::value::Calculation>, crate::Error> {
        Ok(Some(value::Calculation::new(
            domain,
            output,
            vec![value::Calculation::input(
                inputs[0].request([0, 0, 32, 32].into())?,
            )],
            value::Operation::cpu("whole-input CPU mean", self.key(), self),
        )?))
    }
}
impl CpuKernel for Mean {
    fn run(
        &self,
        inputs: &[CpuImage],
        _: CellRegion,
        _: &dyn Fn() -> bool,
    ) -> Result<Option<Vec<[f32; 4]>>, crate::Error> {
        assert_eq!(inputs.len(), 1);
        assert_eq!(
            inputs[0].pixels.len(),
            32 * 32,
            "the kernel still receives its full declared input"
        );
        let mut value = [0_f64; 4];
        for pixel in &inputs[0].pixels {
            for c in 0..4 {
                value[c] += f64::from(pixel[c]) / 1024.;
            }
        }
        Ok(Some(vec![value.map(|v| v as f32)]))
    }
}

#[test]
fn cpu_input_reads_are_batched_independently_of_tiny_consumer_tiles() {
    let (device, queue, _) = crate::photographic::test_support::render_context();
    for tile in [[1, 1], [2, 2], [32, 32]] {
        let maximum_submissions = 4;
        let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
        let reads = Arc::new(AtomicUsize::new(0));
        let source = source(Arc::new(InputPlate(reads.clone())));
        let gpu = pointwise(
            source,
            Function::new(
                "fn double(p:vec4f,at:vec2f)->vec4f {return vec4f(2.*p.rgb,p.w);}",
                "double",
            ),
        );
        let region: CellRegion = [0, 0, 1, 1].into();
        let root = Value::from_operation(
            Domain {
                bounds: Some(region),
                grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            },
            [gpu],
            Mean,
        );
        let request = context
            .request(
                root,
                Region {
                    bounds: region,
                    grid: Grid::axis_aligned([0.; 2], [1.; 2]),
                },
                Tiling { tile },
            )
            .unwrap();
        let mut actual = Vec::new();
        let counts = context
            .read(
                &queue,
                &request,
                || false,
                |_, pixels| {
                    actual.extend_from_slice(pixels);
                    Ok(())
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            actual,
            vec![[2., -0.5, 0.25, 0.]],
            "both the GPU transform and CPU reduction must run"
        );
        assert!(
            counts.submissions <= maximum_submissions,
            "input batching must not inherit consumer-sized fragments: tile={tile:?}, {counts:?}"
        );
        assert!(reads.load(Ordering::Relaxed) <= maximum_submissions);
    }
}
