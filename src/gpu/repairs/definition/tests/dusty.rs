use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
struct DustyPhoto {
    side: u32,
    across: u32,
    spacing: u32,
}
impl DustyPhoto {
    fn count(self) -> u32 {
        self.across * self.across
    }
    fn point(self, index: u32) -> [u32; 2] {
        let origin = (self.side - (self.across - 1) * self.spacing) / 2;
        // This permutation spreads consecutive clicks through the photograph.
        let index = (index * 73) % self.count();
        [
            origin + index % self.across * self.spacing,
            origin + index / self.across * self.spacing,
        ]
    }
    fn value(self, x: i32, y: i32) -> [u16; 4] {
        let origin = (self.side - (self.across - 1) * self.spacing) as i32 / 2;
        let cell =
            [x, y].map(|p| (p - origin + self.spacing as i32 / 2).div_euclid(self.spacing as i32));
        let marked = cell.into_iter().all(|i| i >= 0 && i < self.across as i32)
            && (x - origin - cell[0] * self.spacing as i32).pow(2)
                + (y - origin - cell[1] * self.spacing as i32).pow(2)
                <= 1;
        if marked {
            [0; 4]
        } else {
            [12000, 24000, 36000, 0]
        }
    }
    fn repair(self, index: u32) -> Repair {
        let [x, y] = self.point(index);
        Repair {
            id: RepairId(u64::from(index) + 1),
            enabled: true,
            origin: RepairOrigin::Manual,
            support: SupportMask::new(
                [x - 1, y - 1],
                [3, 3],
                vec![false, true, false, true, true, true, false, true, false],
            )
            .unwrap(),
            // Target and donor neighbourhoods are isolated from the other dusts.
            offset: [16, 16],
        }
    }
    fn frame(self) -> crate::frame_raster::FrameRaster {
        let composition = crate::geometry::Composition::with_crop(
            [0, 0, self.side, self.side].into(),
            Default::default(),
            Default::default(),
            Some(crate::crop::Crop {
                center: [f64::from(self.side) / 2.; 2],
                size: [self.side * 3 / 4; 2],
                angle: 0.13,
            }),
        )
        .unwrap();
        let mut frame = crate::frame_raster::FrameRaster::full(composition).unwrap();
        frame.output = if self.side == 512 {
            [64, 64]
        } else {
            [768, 768]
        };
        frame
    }
}
impl Source for DustyPhoto {
    fn domain(&self) -> Domain {
        Domain {
            bounds: Some(CellRegion::new([0, 0], [self.side, self.side])),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::CameraCodes,
        }
    }
    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new((
            "dusty plate",
            self.side,
            self.across,
            self.spacing,
        )))
    }
    fn storage_format(&self) -> ImageFormat {
        ImageFormat::Rgba16Uint
    }
    fn read_codes(&self, region: CellRegion) -> Result<Vec<[u16; 4]>, Error> {
        Ok((region.y..region.bottom())
            .flat_map(|y| (region.x..region.right()).map(move |x| self.value(x, y)))
            .collect())
    }
    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        Ok(self
            .read_codes(region)?
            .into_iter()
            .map(|p| p.map(f32::from))
            .collect())
    }
}
struct Signed;
impl Operation for Signed {
    fn key(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new("dusty signed normalization"))
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
            "fn normalized(p:vec4f,at:vec2f)->vec4f {return vec4f(p.rgb / 10000. * vec3f(1.,-1.,1.),p.w);}",
            "normalized",
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

fn collect(context: &Context, queue: &wgpu::Queue, query: &Evaluation) -> Vec<[f32; 4]> {
    let region = query.region().bounds;
    let mut pixels = vec![[0.; 4]; region.width as usize * region.height as usize];
    context
        .read(
            queue,
            &Default::default(),
            query,
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
    pixels
}

fn exercise(device: &wgpu::Device, queue: &wgpu::Queue, photo: DustyPhoto, batch: u32) {
    let context = Context::new(
        device,
        CachePolicy {
            generated_bytes: 512 << 20,
        },
    );
    let scope = context.materializations();
    scope.set_test_pressure(crate::sys::MemoryPressure::Normal);
    let gate = crate::gpu::submission::SubmissionGate::default();
    let camera = source(Arc::new(photo));
    let query = context
        .request(
            camera.clone(),
            Region {
                bounds: camera.domain().finite().unwrap(),
                grid: camera.domain().grid,
            },
            Budget {
                tile: [512, 128],
                scratch_bytes: 128 << 20,
            },
        )
        .unwrap();
    let held = pollster::block_on(
        context
            .materialize(
                queue,
                &gate,
                &scope,
                query,
                ImageFormat::Rgba16Uint,
                ImageBacking::Buffer,
            )
            .unwrap(),
    )
    .unwrap();
    let base = Value::from_operation(
        Domain {
            meaning: Meaning::Neutral,
            ..camera.domain()
        },
        [camera],
        Signed,
    );
    let frame = photo.frame();
    let bounds = CellRegion::new([0, 0], frame.output);
    let request = |repairs: &[Repair]| {
        let image = compose(base.clone(), repairs).unwrap();
        let root = reconstruct_tiled(
            image,
            Domain {
                bounds: Some(bounds),
                grid: Grid::from_frame(frame),
                meaning: Meaning::Neutral,
            },
        )
        .unwrap();
        context
            .request(
                root,
                Region {
                    bounds,
                    grid: Grid::from_frame(frame),
                },
                Budget {
                    tile: frame.output,
                    scratch_bytes: 128 << 20,
                },
            )
            .unwrap()
    };
    let control = collect(&context, queue, &request(&[]));
    assert!(
        control.iter().any(|p| p[0] < 1.19),
        "the reduced view must visibly contain the known dark dusts before healing"
    );
    let counter = Arc::new(AtomicUsize::new(0));
    let observed = counter.clone();
    gate.observe_encoded_resources(Arc::new(move || {
        observed.fetch_add(1, Ordering::Relaxed);
    }));
    let mut repairs = Vec::new();
    let mut previous = None;
    let mut latest = None;
    let mut durations = Vec::<Duration>::new();
    while repairs.len() < photo.count() as usize {
        let before = repairs.len() as u32;
        let end = (before + batch).min(photo.count());
        repairs.extend((before..end).map(|i| photo.repair(i)));
        counter.store(0, Ordering::Relaxed);
        let began = Instant::now();
        let query = request(&repairs);
        let image = pollster::block_on(
            context
                .materialize(
                    queue,
                    &gate,
                    &scope,
                    query.clone(),
                    ImageFormat::Rgba32Float,
                    ImageBacking::Buffer,
                )
                .unwrap(),
        )
        .unwrap();
        let elapsed = began.elapsed();
        let submissions = counter.load(Ordering::Relaxed);
        assert!(
            // Cold tiles need assembly and may reacquire evicted local heals.
            // The allowance is independent of the photograph's total repair count.
            submissions <= 256 + 4 * (end - before) as usize,
            "adding {} repairs at total {end} must stay within bounded view assembly plus new repair work: {submissions}",
            end - before
        );
        if end.is_power_of_two() || end == photo.count() {
            eprintln!(
                "DUST clicks={end} batch={batch} elapsed={elapsed:?} submissions={submissions}"
            );
        }
        durations.push(elapsed);
        previous = Some(image);
        latest = Some(query);
    }
    for pixel in collect(&context, queue, &latest.unwrap()) {
        for (actual, expected) in pixel.into_iter().zip([1.2, -2.4, 3.6, 0.]) {
            assert!(
                (actual - expected).abs() < 2e-5,
                "every dust must be restored to the analytic constant plate, retaining signed/HDR RGB and original metadata: {pixel:?}"
            );
        }
    }
    durations.sort();
    eprintln!(
        "DUST total={} batch={batch} updates={} p50={:?} p95={:?} max={:?}",
        photo.count(),
        durations.len(),
        durations[durations.len() / 2],
        durations[durations.len() * 95 / 100],
        durations.last().unwrap()
    );
    drop(previous);
    drop(held);
}

#[test]
fn many_tiny_repairs_keep_incremental_work_bounded_and_restore_every_dust() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    exercise(
        &device,
        &queue,
        DustyPhoto {
            side: 512,
            across: 8,
            spacing: 40,
        },
        1,
    );
}

#[test]
#[ignore = "1024 incremental heals and rapid-burst captures on a 4096-square source"]
fn thousand_tiny_repairs_incremental_and_burst_stress() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let photo = DustyPhoto {
        side: 4096,
        across: 32,
        spacing: 80,
    };
    exercise(&device, &queue, photo, 1);
    exercise(&device, &queue, photo, 16);
}
