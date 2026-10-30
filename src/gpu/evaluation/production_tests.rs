use super::*;
use crate::{
    adjustment_layers::{AdjustmentLayer, AdjustmentLayerSettings, AdjustmentLayerSnapshot},
    geometry::{Composition, FramePoint},
    gpu::{GpuFlatField, ProcessingInput, repairs::Snapshot},
    processing::{Calibration, InputSettings, RollSettings},
    rendering::{OutputReference, OutputTarget},
};
use glam::DVec3;

fn canonical(
    renderer: &crate::renderer::GpuRenderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    request: crate::gpu::execution::Request<'_>,
) -> Vec<[f32; 4]> {
    let composition = request.composition;
    let mut pixels = None;
    crate::gpu::execution::execute(
        renderer,
        device,
        (queue, &Default::default()),
        request,
        |render, finish| {
            let size = composition.size();
            let mut values = vec![[0.; 4]; (size[0] * size[1]) as usize];
            let tile = crate::gpu::execution::TILE;
            for y in (0..size[1]).step_by(tile[1] as usize) {
                for x in (0..size[0]).step_by(tile[0] as usize) {
                    let extent = [tile[0].min(size[0] - x), tile[1].min(size[1] - y)];
                    let block = render([x, y], extent)?;
                    let crate::gpu::execution::OutputBlock::Linear(block) = block else {
                        panic!("float result");
                    };
                    for row in 0..extent[1] {
                        let dst = ((y + row) * size[0] + x) as usize;
                        let src = (row * extent[0]) as usize;
                        values[dst..dst + extent[0] as usize]
                            .copy_from_slice(&block[src..src + extent[0] as usize]);
                    }
                }
            }
            pixels = Some(values);
            finish()
        },
        || Ok(()),
    )
    .unwrap();
    pixels.unwrap()
}

#[test]
fn complete_negative_correction_repairs_mtf_grain_and_look_preserve_original_processing() {
    use crate::dust_repair::{Repair, RepairId, RepairOrigin, SupportMask};
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let file = crate::dng_fixture::Fixture {
        side: 512,
        bits: 16,
        ramp: true,
        marker: Some((90, 130, 60000)),
        ..Default::default()
    }
    .write();
    let mut scan = crate::scan::Scan::open(file.path(), None).unwrap();
    let a = scan.area;
    let correction = Arc::new(
        crate::flat_field::FlatField::from_nodes(
            [a.x, a.y, a.width, a.height],
            [2, 2],
            vec![
                DVec3::new(1.2, 0.9, 1.1),
                DVec3::new(0.95, 1.1, 0.85),
                DVec3::new(1.1, 1.3, 0.95),
                DVec3::new(0.9, 1.05, 1.2),
            ],
        )
        .unwrap(),
    );
    scan.set_flat_field(Some(correction.clone())).unwrap();
    let field = Arc::new(GpuFlatField::new(&device, correction.clone()).unwrap());
    let base = Calibration {
        dmin: DVec3::splat(1.05),
        dmax: DVec3::splat(0.25),
        input_floor: scan.input_floor,
    };
    let settings = InputSettings::from(correction.processing_calibration(base).unwrap());
    let roll = RollSettings {
        exposure: 0.,
        ..Default::default()
    };
    // A local grade edge keeps finite MTF filtering observable after reconstruction.
    let mut local = AdjustmentLayer::global(AdjustmentLayerSettings {
        exposure: -2.,
        ..Default::default()
    });
    local.mask = crate::adjustment_layers::AdjustmentLayerCoverage::Radial {
        center: crate::geometry::SourcePoint([
            f64::from(a.x) + 0.75 * f64::from(a.width),
            f64::from(a.y) + 0.75 * f64::from(a.height),
        ]),
        radii: [45., 35.],
        angle: 0.2,
        feather: 0.,
    };
    let grade = AdjustmentLayerSnapshot::new(vec![
        AdjustmentLayer::global(AdjustmentLayerSettings {
            contrast: 1.15,
            saturation: 0.9,
            ..Default::default()
        }),
        local,
    ])
    .unwrap();
    let repair = Repair {
        id: RepairId(1),
        enabled: true,
        origin: RepairOrigin::Manual,
        support: SupportMask::new([a.x + 64, a.y + 100], [40, 40], vec![true; 1600]).unwrap(),
        offset: [48, -32],
    };
    let neutral = crate::scan::ImageSource::new(&scan, InputSettings::from(base)).unwrap();
    let evaluated = Arc::new(crate::repair_source::evaluate(&neutral, &repair).unwrap());
    let snapshot = Snapshot::logical(vec![evaluated.clone()]).unwrap();
    let pipeline = Arc::new(
        ProcessingInput::new(
            &device,
            settings,
            roll,
            grade.clone(),
            scan.positive_matrix(&settings).unwrap(),
            a,
        )
        .unwrap(),
    );
    let mut input = ProcessingInput::from(pipeline)
        .with_frame_size([a.width, a.height])
        .with_film_seed((1 << 60) + 17)
        .with_film_emulation(crate::film_emulation::FilmEmulationSettings {
            enabled: true,
            size: 0.1,
            image_sigma: 0.1,
            amount: 0.12,
            colour: 0.7,
        });
    input.repairs = snapshot;
    let look = Arc::new(
        crate::looks::LookAsset::new(
            "production layered look".into(),
            crate::looks::LookOrigin::Imported,
            Arc::new(
                crate::looks::Lut3D::new(
                    2,
                    [-2.; 3],
                    [2.; 3],
                    (0..8)
                        .map(|i| {
                            [
                                -1.9 + 4. * (i % 2) as f32,
                                -1.95 + 3.8 * ((i / 2) % 2) as f32,
                                -1.97 + 4. * (i / 4) as f32,
                            ]
                        })
                        .collect(),
                )
                .unwrap(),
            ),
        )
        .unwrap(),
    );
    input = input.with_look(Some(look.clone()));
    let stages = input.pixel_pipeline().unwrap();
    let repairs = vec![evaluated];
    let request = || crate::gpu::execution::Request {
        scan: &scan,
        field: Some(field.clone()),
        input: InputSettings::from(base),
        roll,
        grade: grade.clone(),
        repairs: &repairs,
        composition: Composition::new(a, Default::default(), Default::default()).unwrap(),
        stages: stages.clone(),
        output: OutputReference::DisplayReferred(OutputTarget::Srgb),
        look: Some(look.clone()),
        encoding: crate::gpu::execution::SampleEncoding::LinearFloat,
        budget: crate::gpu::execution::WorkBudget::for_device(&device),
    };
    let expected = canonical(&renderer, &device, &queue, request());
    let composition = Composition::new(a, Default::default(), Default::default()).unwrap();
    let frame = crate::frame_raster::FrameRaster::new(
        composition,
        FramePoint([0.; 2]),
        FramePoint(composition.size().map(f64::from)),
        [211, 197],
    )
    .unwrap();
    let expected = reconstructed(&expected, frame);
    for omitted in ["grain", "repairs", "MTF", "look"] {
        let mut control = request();
        match omitted {
            "grain" => control
                .stages
                .stages
                .retain(|s| s.get::<crate::pixel_pipeline::Grain>().is_none()),
            "MTF" => control
                .stages
                .stages
                .retain(|s| s.get::<crate::image_mtf::ImageMtfParameters>().is_none()),
            "repairs" => control.repairs = &[],
            "look" => control.look = None,
            _ => unreachable!(),
        }
        let control = canonical(&renderer, &device, &queue, control);
        let control = reconstructed(&control, frame);
        let difference = control
            .iter()
            .zip(&expected)
            .map(|(a, b)| (*a - *b).abs().max_element())
            .fold(0., f64::max);
        assert!(
            difference > 4. * 2e-5,
            "fixture must detect omitted {omitted}: {difference}"
        );
    }
    let uncorrected = crate::scan::Scan::from_source(scan.source().clone());
    let mut control = request();
    control.scan = &uncorrected;
    control.field = None;
    let control = canonical(&renderer, &device, &queue, control);
    let control = reconstructed(&control, frame);
    assert!(
        control
            .iter()
            .zip(&expected)
            .any(|(a, b)| (*a - *b).abs().max_element() > 4. * 2e-5),
        "fixture must detect omitted camera correction"
    );
    let actual = crate::refinement::refine(
        &renderer,
        &device,
        (&queue, &Default::default()),
        (&scan, Some(field)),
        input,
        OutputTarget::Srgb,
        (&std::sync::atomic::AtomicU64::new(1), 1),
        crate::refinement::ViewRequest {
            area: a,
            output: frame.output,
            frame: Some(frame),
        },
    )
    .unwrap()
    .unwrap();
    let actual = crate::viewer::readback::image_pixels(&device, &queue, &actual);
    assert_eq!(actual.len(), (frame.output[0] * frame.output[1]) as usize);
    assert!(
        super::tests::max_error(&expected, &actual) < 2e-5,
        "full-model original reconstruction"
    );
}

fn reconstructed(values: &[[f32; 4]], frame: crate::frame_raster::FrameRaster) -> Vec<DVec3> {
    let area = frame.composition.area();
    (0..frame.output[1])
        .flat_map(|y| {
            (0..frame.output[0]).map(move |x| {
                crate::sampling::weighted_mean_iter(
                    frame
                        .footprint([x, y])
                        .unwrap()
                        .pixels(area)
                        .unwrap()
                        .map(|(p, w)| {
                            let value =
                                values[((p[1] - area.y) * area.width + p[0] - area.x) as usize];
                            Ok((
                                DVec3::new(value[0] as f64, value[1] as f64, value[2] as f64),
                                w,
                            ))
                        }),
                )
                .unwrap()
            })
        })
        .collect()
}

#[test]
fn large_source_jobs_plan_tiles_lazily_without_reading_or_retaining_a_full_image() {
    struct Header;

    impl Source for Header {
        fn domain(&self) -> Domain {
            Domain {
                bounds: [0, 0, 11648, 8736].into(),
                grid: Grid::axis_aligned([0.; 2], [1.; 2]),
                meaning: Meaning::CameraCodes,
            }
        }

        fn read(&self, _: crate::geometry::SourceRegion) -> Result<Vec<[f32; 4]>, Error> {
            panic!("planning must not read camera data")
        }
    }

    struct Observed {
        inner: Arc<dyn Operation>,
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl Operation for Observed {
        fn input(
            &self,
            output: crate::geometry::SourceRegion,
        ) -> Result<Option<crate::geometry::SourceRegion>, Error> {
            self.inner.input(output)
        }

        fn lower(
            &self,
            fragment: &mut Fragment,
            input: Option<PlannedImage>,
            output: PlannedImage,
        ) -> Result<(), Error> {
            self.calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.inner.lower(fragment, input, output)
        }
    }
    let root = source(Arc::new(Header));
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let root = ImageNode::new(
        root.domain,
        None,
        Observed {
            inner: root.operation.clone(),
            calls: calls.clone(),
        },
    );
    let job = Job::new(
        root.clone(),
        root.domain.bounds,
        Budget {
            tile: [512, 128],
            scratch_bytes: 8 << 20,
        },
    )
    .unwrap();
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::Relaxed),
        0,
        "job creation must not lower all source tiles"
    );
    let mut tiles = job.plans();
    let first = tiles.next().unwrap().unwrap();
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert_eq!([first.region.width, first.region.height], [512, 128]);
    assert!(first.peak_scratch_bytes <= 8 << 20);
    let second = tiles.next().unwrap().unwrap();
    assert_eq!(second.region.x, 512);
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 2);
}

#[test]
fn immutable_programs_are_reused_without_retaining_the_native_source() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let file = crate::dng_fixture::Fixture {
        side: 128,
        ..Default::default()
    }
    .write();
    let scan = crate::scan::Scan::open(file.path(), None).unwrap();
    let lease = Arc::downgrade(scan.source());
    let area = scan.area;
    let mut count = 0;
    let mut result = None;
    for exposure in [0., 0.5] {
        let input = ProcessingInput::from(Arc::new(
            ProcessingInput::new(
                &device,
                InputSettings::Digital,
                RollSettings {
                    exposure,
                    ..Default::default()
                },
                AdjustmentLayerSnapshot::new(vec![]).unwrap(),
                scan.positive_matrix(&InputSettings::Digital).unwrap(),
                area,
            )
            .unwrap(),
        ));
        result = crate::refinement::refine(
            &renderer,
            &device,
            (&queue, &Default::default()),
            (&scan, None),
            input,
            OutputTarget::Srgb,
            (&std::sync::atomic::AtomicU64::new(1), 1),
            crate::refinement::ViewRequest {
                area,
                output: [31, 29],
                frame: None,
            },
        )
        .unwrap();
        let programs = renderer.evaluation.programs.len();
        assert!(programs > 0);
        if exposure == 0. {
            count = programs;
        } else {
            assert_eq!(
                programs, count,
                "changing captured constants reuses shader programs"
            );
        }
    }
    drop(scan);
    assert!(
        lease.upgrade().is_none(),
        "compiled programs and delivered pixels do not retain original leases"
    );
    assert!(result.is_some());
}
